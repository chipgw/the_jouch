mod canned_responses;
mod commands;
mod db;
mod config;

use std::{collections::HashSet};

use serenity::{async_trait, client::{Client, Context, EventHandler}, framework::standard::{
        StandardFramework,
        macros::{group, hook},
        DispatchError,
        CommandResult,
    }, http::Http, model::{
        channel::Message, gateway::Ready, id::GuildId,
        interactions::{
            application_command::{
                ApplicationCommand,
                ApplicationCommandOptionType, ApplicationCommandType, ApplicationCommandInteraction,
            },
            Interaction,
            InteractionResponseType,
        },
    }, utils::MessageBuilder, builder::CreateApplicationCommands, prelude::GatewayIntents};

use commands::{autonick::*, birthday::*, clear::*, sit::*};

#[group]
#[commands(birthday, clear_from, sit)]
struct General;

struct Handler;

impl Handler {
    fn create_commands(commands: &mut CreateApplicationCommands) -> &mut CreateApplicationCommands {
        commands
        .create_application_command(|command| {
            command.name("sit").description("Sit on The Jouch");
            // TODO - restore once it's possible to attach images to command responses
            command.create_option(|option| {
                option.name("with")
                    .description("sit with another user")
                    .kind(ApplicationCommandOptionType::SubCommand)
                    .create_sub_option(|option|{
                        option.name("friend")
                        .description("a friend to sit on The Jouch with")
                        .kind(ApplicationCommandOptionType::User)
                        .required(true)
                    })
            });
            command.create_option(|option| {
                option
                    .name("solo")
                    .description("sit by yourself")
                    .kind(ApplicationCommandOptionType::SubCommand)
            });
            command.create_option(|option| {
                option
                    .name("check")
                    .description("check how often users have sat on The Jouch")
                    .kind(ApplicationCommandOptionType::SubCommand);

                    // allow up to 10 users to check in on.
                    for i in 0..10 {
                        option.create_sub_option(|option|{
                            option.name(format!("user{}", i))
                            .description("a user to check on")
                            .kind(ApplicationCommandOptionType::User)
                        });
                    }

                    option
            });
            command
        })
        .create_application_command(|command| {
            command.name("birthday").description("Birthday tracking by The Jouch");

            command.create_option(|option| {
                option
                    .name("set")
                    .description("set your birthday")
                    .kind(ApplicationCommandOptionType::SubCommand)
                    .create_sub_option(|option|{
                        option
                            .name("birthday")
                            .kind(ApplicationCommandOptionType::String)
                            .description("Birthday date string")
                            .required(true)
                    })
                    .create_sub_option(|option|{
                        option
                            .name("privacy")
                            .kind(ApplicationCommandOptionType::String)
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
                    .kind(ApplicationCommandOptionType::SubCommand)
                    .create_sub_option(|option|{
                        option.name("user")
                        .description("a user to check on")
                        .kind(ApplicationCommandOptionType::User)
                    })
            });
            command.create_option(|option| {
                option
                    .name("clear")
                    .description("clear your birthday")
                    .kind(ApplicationCommandOptionType::SubCommand)
            });

            command
        })
        .create_application_command(|command| {
            command.name("autonick").description("Automatic nickname updating tracking by The Jouch");

            command.create_option(|option| {
                option
                    .name("set")
                    .description("set your nickname format string")
                    .kind(ApplicationCommandOptionType::SubCommand)
                    .create_sub_option(|option|{
                        option
                            .name("nickname")
                            .kind(ApplicationCommandOptionType::String)
                            .description("format string; %a will be replaced with age and %j with times sat on The Jouch")
                            .required(true)
                    })
            });
            command.create_option(|option| {
                option
                    .name("clear")
                    .description("clear automatic nickname")
                    .kind(ApplicationCommandOptionType::SubCommand)
            });

            command
        })
    }

    async fn handle_app_command(ctx: &Context, command: ApplicationCommandInteraction) -> CommandResult {
        command.create_interaction_response(&ctx.http, |r| {
            r.kind(InteractionResponseType::DeferredChannelMessageWithSource)
        }).await?;

        let content = match command.data.name.as_str() {
            "sit" => sit_slashcommand(&ctx, &command).await,
            "birthday" => birthday_slashcommand(&ctx, &command).await,
            "clear_from" => clear_from_slashcommand(&ctx, &command).await,
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
                        command.name("clear_from").kind(ApplicationCommandType::Message)
                    })
                }).await
            } else {
                ApplicationCommand::set_global_application_commands(&ctx.http, Handler::create_commands).await
            }
        } else {
            ApplicationCommand::set_global_application_commands(&ctx.http, Handler::create_commands).await
        };

        println!("I now have the following slash commands: {:#?}", commands);
    }
    async fn cache_ready(&self, ctx: Context, _guilds: Vec<GuildId>) {
        // Spawn the nickname & birthday checkers.
        tokio::spawn(check_nicks_loop(ctx.clone()));
        tokio::spawn(check_birthdays_loop(ctx.clone()));
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

    let http = Http::new_with_application_id(&config.token, config.app_id);

    // We will fetch your bot's owners and id
    let (owners, bot_id) = match http.get_current_application_info().await {
        Ok(info) => {
            let mut owners = HashSet::new();
            if let Some(team) = info.team {
                owners.insert(team.owner_user_id);
            } else {
                owners.insert(info.owner.id);
            }
            match http.get_current_user().await {
                Ok(bot_id) => (owners, bot_id.id),
                Err(why) => panic!("Could not access the bot id: {:?}", why),
            }
        }
        Err(why) => panic!("Could not access application info: {:?}", why),
    };

    let framework = StandardFramework::new()
        .configure(|c| c
            .prefix(config.prefix.as_str())
            .on_mention(Some(bot_id))
            .owners(owners)) 
        .group(&GENERAL_GROUP)
        .on_dispatch_error(dispatch_error)
        .before(before)
        .after(after)
        .unrecognised_command(unknown_command)
        .normal_message(canned_responses::process);

    const INTENTS: GatewayIntents = GatewayIntents::all().difference(GatewayIntents::GUILD_PRESENCES);

    // Login with a bot token from the configuration file
    let mut client = Client::builder(config.token.as_str(), INTENTS)
        .event_handler(Handler)
        .application_id(config.app_id)
        .framework(framework)
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

#[hook]
async fn dispatch_error(context: &Context, msg: &Message, error: DispatchError, _string: &str) {
    match error {
        DispatchError::NotEnoughArguments { min, given } => {
            let s = format!("Need {} arguments, but only got {}.", min, given);

            let _ = msg.channel_id.say(&context, &s).await;
        },
        DispatchError::TooManyArguments { max, given } => {
            let s = format!("Max arguments allowed is {}, but got {}.", max, given);

            let _ = msg.channel_id.say(&context, &s).await;
        },
        _ => println!("Unhandled dispatch error."),
    }
}

#[hook]
async fn before(_ctx: &Context, msg: &Message, command_name: &str) -> bool {
    println!("Got command '{}' by user '{}'", command_name, msg.author.name);

    // TODO - let guilds enable/disable features and filter out disabled commands here

    true // if `before` returns false, command processing doesn't happen.
}

#[hook]
async fn after(ctx: &Context, msg: &Message, command_name: &str, command_result: CommandResult) {
    match command_result {
        Ok(()) => println!("Processed command '{}'", command_name),
        Err(why) => {
            println!("Command '{}' returned error {:?}", command_name, why);

            let reply_result = msg.reply(ctx, MessageBuilder::new()
                .push("Error executing command")
                .push_codeblock(why.to_string(), None)
                .build()).await;
                
            if let Err(err_replying) = reply_result {
                println!("Command '{}' returned error {:?} when replying with error message", command_name, err_replying);
            }
        },
    }
}

#[hook]
async fn unknown_command(_ctx: &Context, _msg: &Message, unknown_command_name: &str) {
    println!("Could not find command named '{}'", unknown_command_name);
}
