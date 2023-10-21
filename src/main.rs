mod canned_responses;
mod commands;
mod db;
mod config;

use std::path::PathBuf;

use anyhow::anyhow;
use commands::db_migration::migrate;
use mongodb::Database;
use serenity::model::Permissions;
use serenity::prelude::TypeMapKey;
use shuttle_persist::PersistInstance;
use shuttle_runtime;
use shuttle_serenity::ShuttleSerenity;
use shuttle_secrets::SecretStore;
use serenity::model::application::interaction::application_command::ApplicationCommandInteraction;
use serenity::{async_trait, client::{Client, Context, EventHandler}, model::{
        channel::Message, gateway::Ready, id::GuildId,
        application::{
            command::{
                Command,
                CommandOptionType,
                CommandType,
            },
            interaction::{
                Interaction,
                InteractionResponseType,
            },
        },
    }, builder::CreateApplicationCommands, prelude::GatewayIntents};
use tracing::{error, info, trace, warn};

use commands::{autonick::*, birthday::*, clear::*, sit::*};

pub type CommandResult<T = ()> = anyhow::Result<T>;

struct Handler;

impl Handler {
    fn create_commands(commands: &mut CreateApplicationCommands) -> &mut CreateApplicationCommands {
        commands
        .create_application_command(|command| {
            command.name("sit").description("Sit on The Jouch");
            command.create_option(|option| {
                option
                    .name("friend")
                    .description("a friend to sit on The Jouch with")
                    .kind(CommandOptionType::User)
            })
        })
        .create_application_command(|command| {
            command.name("rankings").description("Check how often users have sat on and/or flipped The Jouch");

            command.create_option(|option|{
                option
                    .name("sort")
                    .description("what to sort users by (ignored when specifying users)")
                    .kind(CommandOptionType::Integer)
                    .add_int_choice("Sits", 0)
                    .add_int_choice("Flips", 1)
            });

            // allow up to 10 users to check in on.
            for i in 0..10 {
                command.create_option(|option| {
                    option
                        .name(format!("user{}", i))
                        .description("a user to check on")
                        .kind(CommandOptionType::User)
                });
            }
            command
        })
        .create_application_command(|command| {
            command.name("flip").description("Flip The Jouch")
        })
        .create_application_command(|command| {
            command.name("rectify").description("Put The Jouch back upright")
        })
        .create_application_command(|command| {
            command.name("birthday").description("Birthday tracking by The Jouch");

            command.create_option(|option| {
                option
                    .name("set")
                    .description("set your birthday")
                    .kind(CommandOptionType::SubCommand)
                    .create_sub_option(|option|{
                        option
                            .name("birthday")
                            .kind(CommandOptionType::String)
                            .description("Birthday date string")
                            .required(true)
                    })
                    .create_sub_option(|option|{
                        option
                            .name("privacy")
                            .kind(CommandOptionType::String)
                            .description("Optional birthday privacy setting (defaults to Public)")
                            .add_string_choice("Public", "PublicFull")
                            .add_string_choice("Public Month/Day", "PublicDay")
                            .add_string_choice("Private", "Private")
                    })
            });
            command.create_option(|option| {
                option
                    .name("check")
                    .description("check birthday for user")
                    .kind(CommandOptionType::SubCommand)
                    .create_sub_option(|option|{
                        option.name("user")
                        .description("a user to check on")
                        .kind(CommandOptionType::User)
                    })
            });
            command.create_option(|option| {
                option
                    .name("clear")
                    .description("clear your birthday")
                    .kind(CommandOptionType::SubCommand)
            });

            command
        })
        .create_application_command(|command| {
            command.name("autonick").description("Automatic nickname updating tracking by The Jouch");

            command.create_option(|option| {
                option
                    .name("set")
                    .description("set your nickname format string")
                    .kind(CommandOptionType::SubCommand)
                    .create_sub_option(|option|{
                        option
                            .name("nickname")
                            .kind(CommandOptionType::String)
                            .description("format string; %a will be replaced with age and %j with times sat on The Jouch")
                            .required(true)
                    })
            });
            command.create_option(|option| {
                option
                    .name("clear")
                    .description("clear automatic nickname")
                    .kind(CommandOptionType::SubCommand)
            });

            command
        })
    }

    async fn handle_app_command(ctx: &Context, command: ApplicationCommandInteraction) -> CommandResult {
        command.create_interaction_response(&ctx.http, |r| {
            r.kind(InteractionResponseType::DeferredChannelMessageWithSource)
        }).await?;

        let content = match command.data.name.as_str() {
            "sit" => sit(&ctx, &command).await,
            "rankings" => rank(&ctx, &command).await,
            "flip" => flip(ctx, &command).await,
            "rectify" => rectify(ctx, &command).await,
            "birthday" => birthday(&ctx, &command).await,
            "clear_from" => clear_from(&ctx, &command).await,
            "migrate" => migrate(&ctx, &command).await,
            "autonick" => autonick(&ctx, &command).await,
            _ => Err(anyhow!("not implemented :(")),
        };

        if let Err(content) = content {
            command.edit_original_interaction_response(&ctx.http, |response| {
                response.content(content)
            }).await?;
        }

        Ok(())
    }
}

#[async_trait]
impl EventHandler for Handler {  
    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        if let Interaction::ApplicationCommand(command) = interaction {
            if let Err(why) = Handler::handle_app_command(&ctx, command).await {
                error!("Cannot respond to slash command: {}", why);
            }
        }
    }

    async fn ready(&self, ctx: Context, ready: Ready) {
        info!("{} is connected!", ready.user.name);

        let commands = Command::set_global_application_commands(&ctx.http, Handler::create_commands).await;

        trace!("I now have the following slash commands: {:#?}", commands);

        let data = ctx.data.read().await;
        
        let testing_guild = if let Some(shuttle_items) = data.get::<ShuttleItemsContainer>() {
            shuttle_items.secret_store.get("test_guild").and_then(|id|{id.parse::<u64>().ok()})
        } else {
            None
        };
        trace!("testing guild: {:?}", testing_guild);

        if let Some(guild_id) = testing_guild {
            if let Some(guild) = ctx.cache.guild(guild_id) {
                let commands = guild.set_application_commands(&ctx.http, |commands| {
                    Handler::create_commands(commands);

                    // only available in testing
                    commands.create_application_command(|command| {
                        command.name("clear_from").kind(CommandType::Message).default_member_permissions(Permissions::ADMINISTRATOR)
                    });
                    commands.create_application_command(|command| {
                        command.name("migrate").default_member_permissions(Permissions::ADMINISTRATOR);
                        command.description("Migrate data from Ron files to the new MongoDB storage.");
                        command.create_option(|o|{
                            o.kind(CommandOptionType::Attachment).name("jouch_db").description("a jouch_db.ron file from prior to the MongoDB switch.")
                        }).create_option(|o|{
                            o.kind(CommandOptionType::Attachment).name("config").description("a config.ron file from prior to the MongoDB switch.")
                        })
                    });
                    commands
                }).await;
                trace!("I also have testing guild ({:?}) specific slash commands: {:#?}", testing_guild, commands);
            }
        }
    }

    async fn cache_ready(&self, ctx: Context, _guilds: Vec<GuildId>) {
        // Spawn the nickname & birthday checkers.
        tokio::spawn(check_nicks_loop(ctx.clone()));
        tokio::spawn(check_birthdays_loop(ctx.clone()));
    }
    async fn message(&self, ctx: Context, msg: Message) {
        // Ignore messages from bots to avoid risking an infinite response loop.
        // (mainy concerned about ourself, but any bot in theory could cause one so best to just ignore all)
        if !msg.author.bot {
            if let Err(err) = canned_responses::process(&ctx, &msg).await {
                warn!("Error processing canned responses: {:?}", err);
            }
        }
    }
}

macro_rules! outer {
    ($($tts:tt)*) => {
        shuttle_runtime::Error::Custom(anyhow!($($tts)*))
    }
}

struct ShuttleItemsContainer {
    secret_store: SecretStore,
    assets_dir: PathBuf,
    persist: PersistInstance,
}

impl TypeMapKey for ShuttleItemsContainer {
    type Value = ShuttleItemsContainer;
}

#[shuttle_runtime::main]
async fn serenity(
    #[shuttle_secrets::Secrets] secret_store: SecretStore,
    #[shuttle_shared_db::MongoDb] db: Database,
    #[shuttle_persist::Persist] persist: PersistInstance
) -> ShuttleSerenity {
    let token = secret_store.get("discord_token").ok_or(outer!("Unable to load token from secret store!"))?;
    let app_id = secret_store.get("app_id").ok_or(outer!("Unable to load app id from secret store!"))?;
    let app_id = app_id.parse::<u64>().map_err(|e|{outer!("Unable to parse app id from secret store! {}", e)})?;

    const INTENTS: GatewayIntents = GatewayIntents::all().difference(GatewayIntents::GUILD_PRESENCES);

    // Login with a bot token from the configuration file
    let client = Client::builder(token.as_str(), INTENTS)
        .event_handler(Handler)
        .application_id(app_id)
        .await
        .expect("Error creating client");

    let config = config::Config::load(&persist)?;

    trace!("loaded config data: {:#?}", config);

    let shuttle_items = ShuttleItemsContainer {
        secret_store, assets_dir: PathBuf::from("assets"), persist
    };

    {
        let mut data = client.data.write().await;
        data.insert::<db::Db>(db::Db::new(db));
        data.insert::<config::Config>(config);
        data.insert::<ShuttleItemsContainer>(shuttle_items);
    }
    
    Ok(client.into())
}
