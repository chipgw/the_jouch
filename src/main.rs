mod canned_responses;
mod commands;
mod db;
mod config;
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

use commands::{autonick::*, birthday::*, clear::*, sit::*};

// same as in serenety::framework::standard, but we otherwise don't want framework any more so redefine here
pub type CommandError = Box<dyn std::error::Error + Send + Sync>;
pub type CommandResult<T = ()> = Result<T, CommandError>;

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
            "birthday" => birthday(&ctx, &command).await,
            "clear_from" => clear_from(&ctx, &command).await,
            "autonick" => autonick(&ctx, &command).await,
            _ => Err("not implemented :(".into()),
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
                println!("Cannot respond to slash command: {}", why);
            }
        }
    }
    async fn ready(&self, ctx: Context, ready: Ready) {
        println!("{} is connected!", ready.user.name);

        let data = ctx.data.read().await;
        let testing_guild = if let Some(config) = data.get::<config::Config>() {
            config.testing_guild_id
        } else {
            None
        };

        let commands = if let Some(guild_id) = testing_guild {
            if let Some(guild) = ctx.cache.guild(guild_id) {
                guild.set_application_commands(&ctx.http, |commands| {
                    Handler::create_commands(commands);

                    // only available in testing
                    commands.create_application_command(|command| {
                        command.name("clear_from").kind(CommandType::Message)
                    })
                }).await
            } else {
                Command::set_global_application_commands(&ctx.http, Handler::create_commands).await
            }
        } else {
            Command::set_global_application_commands(&ctx.http, Handler::create_commands).await
        };

        println!("I now have the following slash commands: {:#?}", commands);
    }
    async fn cache_ready(&self, ctx: Context, _guilds: Vec<GuildId>) {
        // Spawn the nickname & birthday checkers.
        tokio::spawn(check_nicks_loop(ctx.clone()));
        tokio::spawn(check_birthdays_loop(ctx.clone()));
    }
    async fn message(&self, ctx: Context, msg: Message) {
        if let Err(err) = canned_responses::process(&ctx, &msg).await {
            println!("Error processing canned responses: {:?}", err);
        }
    }
}

#[tokio::main]
async fn main() {
    let config = config::Config::load().expect("Unable to init config!");
    // make sure config file is created.
    config.save().expect("Unable to save config file!");

    if config.token.is_empty() || config.app_id == 0 {
        println!("Please fill out token & app id in config.ron");
        return;
    }

    const INTENTS: GatewayIntents = GatewayIntents::all().difference(GatewayIntents::GUILD_PRESENCES);

    // Login with a bot token from the configuration file
    let mut client = Client::builder(config.token.as_str(), INTENTS)
        .event_handler(Handler)
        .application_id(config.app_id)
        .await
        .expect("Error creating client");

    {
        let mut data = client.data.write().await;
        data.insert::<db::Db>(db::Db::new().expect("Unable to init database!"));
        data.insert::<config::Config>(config);
    }

    // start listening for events by starting a single shard
    if let Err(why) = client.start().await {
        println!("An error occurred while running the client: {:?}", why);
    }
}

