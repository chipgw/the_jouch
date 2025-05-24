mod canned_responses;
mod commands;
mod config;
mod db;

use std::path::PathBuf;

use anyhow::anyhow;
use commands::db_migration::migrate;
use serenity::all::{
    Command, CommandInteraction, CommandOptionType, CommandType, Context, CreateCommand,
    CreateCommandOption, CreateInteractionResponse, CreateInteractionResponseMessage,
    EditInteractionResponse, EventHandler, GatewayIntents, GuildId, Interaction, Message, Ready,
};
use serenity::model::Permissions;
use serenity::prelude::TypeMapKey;
use serenity::{async_trait, Client};
use shuttle_runtime;
use shuttle_runtime::SecretStore;
use shuttle_serenity::ShuttleSerenity;
use tracing::{error, info, trace, warn};

use commands::{autonick::*, birthday::*, clear::*, sit::*};

pub type CommandResult<T = ()> = anyhow::Result<T>;

struct Handler;

impl Handler {
    fn create_commands() -> Vec<CreateCommand> {
        vec![
            CreateCommand::new("sit").description("Sit on The Jouch")
                .add_option(CreateCommandOption::new(CommandOptionType::User, "friend", "a friend to sit on The Jouch with")),
        {
            let mut options = vec![
                CreateCommandOption::new(CommandOptionType::Integer, "sort", "what to sort users by (ignored when specifying users)")
                    .add_int_choice("Sits", 0)
                    .add_int_choice("Flips", 1)
            ];

            // allow up to 10 users to check in on.
            for i in 0..10 {
                options.push(CreateCommandOption::new(CommandOptionType::User, format!("user{}", i), "a user to check on"));
            }

            CreateCommand::new("rankings").description("Check how often users have sat on and/or flipped The Jouch").set_options(options)
        },
        CreateCommand::new("flip").description("Flip The Jouch"),
        CreateCommand::new("flip").kind(CommandType::Message),
        CreateCommand::new("rectify").description("Put The Jouch back upright"),
        CreateCommand::new("birthday").description("Birthday tracking by The Jouch")
            .add_option(CreateCommandOption::new(CommandOptionType::SubCommand, "set", "set your birthday")
                .add_sub_option(CreateCommandOption::new(CommandOptionType::String, "birthday", "Birthday date string")
                        .required(true))
                .add_sub_option(CreateCommandOption::new(CommandOptionType::String, "privacy", "Optional birthday privacy setting (defaults to Public)")
                        .add_string_choice("Public", "PublicFull")
                        .add_string_choice("Public Month/Day", "PublicDay")
                        .add_string_choice("Private", "Private")
                )
            )
            .add_option(CreateCommandOption::new(CommandOptionType::SubCommand, "check", "check birthday for user")
                .add_sub_option(CreateCommandOption::new(CommandOptionType::User, "user", "a user to check on"))
            )
            .add_option(CreateCommandOption::new(CommandOptionType::SubCommand, "clear", "clear your birthday")),
        CreateCommand::new("autonick").description("Automatic nickname updating tracking by The Jouch")
            .add_option(CreateCommandOption::new(CommandOptionType::SubCommand,"set","set your nickname format string")
                .add_sub_option(
                    CreateCommandOption::new(CommandOptionType::String, "nickname", 
                        "format string; %a will be replaced with age and %j with times sat on The Jouch")
                        .required(true)
                )
            )
            .add_option(CreateCommandOption::new(CommandOptionType::SubCommand,"clear","clear automatic nickname")),
        ]
    }

    async fn handle_app_command(ctx: &Context, command: CommandInteraction) -> CommandResult {
        command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Defer(CreateInteractionResponseMessage::new()),
            )
            .await?;

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
            command
                .edit_response(
                    &ctx.http,
                    EditInteractionResponse::new().content(content.to_string()),
                )
                .await?;
        }

        Ok(())
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        if let Interaction::Command(command) = interaction {
            if let Err(why) = Handler::handle_app_command(&ctx, command).await {
                error!("Cannot respond to slash command: {}", why);
            }
        }
    }

    async fn ready(&self, ctx: Context, ready: Ready) {
        info!("{} is connected!", ready.user.name);

        let commands = Command::set_global_commands(&ctx.http, Handler::create_commands()).await;

        trace!("I now have the following slash commands: {:#?}", commands);

        let data = ctx.data.read().await;

        let testing_guild = if let Some(shuttle_items) = data.get::<ShuttleItemsContainer>() {
            shuttle_items
                .secret_store
                .get("test_guild")
                .and_then(|id| id.parse::<u64>().ok())
        } else {
            None
        };
        trace!("testing guild: {:?}", testing_guild);

        if let Some(guild_id) = testing_guild {
            let commands = GuildId::new(guild_id)
                .set_commands(&ctx.http, {
                    let mut commands = Handler::create_commands();

                    // only available in testing
                    commands.push(
                        CreateCommand::new("clear_from")
                            .kind(CommandType::Message)
                            .default_member_permissions(Permissions::ADMINISTRATOR),
                    );

                    commands.push(
                        CreateCommand::new("migrate")
                            .default_member_permissions(Permissions::ADMINISTRATOR)
                            .description("Migrate data from Ron files to the new MongoDB storage.")
                            .add_option(CreateCommandOption::new(
                                CommandOptionType::Attachment,
                                "jouch_db",
                                "a jouch_db.ron file from prior to the MongoDB switch.",
                            ))
                            .add_option(CreateCommandOption::new(
                                CommandOptionType::Attachment,
                                "config",
                                "a config.ron file from prior to the MongoDB switch.",
                            )),
                    );
                    commands
                })
                .await;
            trace!(
                "I also have testing guild ({:?}) specific slash commands: {:#?}",
                testing_guild,
                commands
            );
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
}

impl TypeMapKey for ShuttleItemsContainer {
    type Value = ShuttleItemsContainer;
}

#[shuttle_runtime::main]
async fn serenity(
    #[shuttle_runtime::Secrets] secret_store: SecretStore,
    #[shuttle_shared_db::Postgres] db: sqlx::PgPool,
) -> ShuttleSerenity {
    // Run SQL migrations; TODO
    sqlx::migrate!()
        .run(&db)
        .await
        .expect("Failed to run migrations");

    let token = secret_store
        .get("discord_token")
        .ok_or(outer!("Unable to load token from secret store!"))?;
    let app_id = secret_store
        .get("app_id")
        .ok_or(outer!("Unable to load app id from secret store!"))?;
    let app_id = app_id
        .parse::<u64>()
        .map_err(|e| outer!("Unable to parse app id from secret store! {}", e))?;

    const INTENTS: GatewayIntents =
        GatewayIntents::all().difference(GatewayIntents::GUILD_PRESENCES);

    // Login with a bot token from the configuration file
    let client = Client::builder(token.as_str(), INTENTS)
        .event_handler(Handler)
        .application_id(app_id.into())
        .await
        .expect("Error creating client");

    let config = config::Config::load(&db).await?;

    trace!("loaded config data: {:#?}", config);

    let shuttle_items = ShuttleItemsContainer {
        secret_store,
        assets_dir: PathBuf::from("assets"),
    };

    {
        let mut data = client.data.write().await;
        data.insert::<db::Db>(db::Db::new(db));
        data.insert::<config::Config>(config);
        data.insert::<ShuttleItemsContainer>(shuttle_items);
    }

    Ok(client.into())
}
