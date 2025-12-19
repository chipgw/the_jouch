mod canned_responses;
mod commands;
mod config;
mod db;

use std::path::PathBuf;

use anyhow::anyhow;
use serenity::all::{
    Command, CommandInteraction, CommandOptionType, CommandType, Context, CreateCommand,
    CreateCommandOption, CreateInteractionResponse, CreateInteractionResponseMessage,
    EditInteractionResponse, EventHandler, GatewayIntents, GuildId, Interaction, Message, Ready,
};
use serenity::model::Permissions;
use serenity::prelude::TypeMapKey;
use serenity::{async_trait, Client};
use tracing::{error, info, trace, warn};

use commands::{autonick::*, birthday::*, clear::*, db_migration::migrate, novena::*, sit::*};

pub type CommandResult<T = ()> = anyhow::Result<T>;

struct Handler;

impl Handler {
    fn create_commands() -> Vec<CreateCommand> {
        vec![
            CreateCommand::new("sit").description("Sit on The Jouch")
                .add_option(CreateCommandOption::new(CommandOptionType::User, "friend", "a friend to sit on The Jouch with")),
            CreateCommand::new("rankings").description("Check how often users have sat on and/or flipped The Jouch").set_options({
                let mut options = vec![
                    CreateCommandOption::new(CommandOptionType::Integer, "sort", "what to sort users by")
                        // RankSortBy::Default is used to indicate no option was passed, and thus doesn't get added here.
                        .add_int_choice("Sits", RankSortBy::Sits as i32)
                        .add_int_choice("Flips", RankSortBy::Flips as i32)
                ];

                // allow up to 10 users to check in on.
                for i in 0..10 {
                    options.push(CreateCommandOption::new(CommandOptionType::User, format!("user{}", i), "a user to check on"));
                }

                options
            }),
            CreateCommand::new("novena").description("Manage scheduled novena messages")
                .default_member_permissions(Permissions::MANAGE_EVENTS)
                .add_option(CreateCommandOption::new(CommandOptionType::SubCommand, "unique", "start a novena with a unique prayer for each day")
                    .set_sub_options({
                        let mut options = vec![
                            CreateCommandOption::new(CommandOptionType::String, "title", "novena title").required(true),
                        ];

                        // add 9 text args for the novena text
                        for i in 1..10 {
                            options.push(CreateCommandOption::new(CommandOptionType::String, format!("day{}", i), "novena text for the day").required(true));
                        }

                        options.push(CreateCommandOption::new(CommandOptionType::String, "start", "date and/or time to start the novena (rounded to the hour, defaults to now)"));

                        options
                    })
                )
                .add_option(CreateCommandOption::new(CommandOptionType::SubCommand, "repeated", "start a novena with the same prayer repeated each day")
                    .add_sub_option(CreateCommandOption::new(CommandOptionType::String, "title", "novena title").required(true))
                    .add_sub_option(CreateCommandOption::new(CommandOptionType::String, "prayer", "novena text").required(true))
                    .add_sub_option(CreateCommandOption::new(CommandOptionType::String, "start", "date and/or time to start the novena (rounded to the hour, defaults to now)"))
                )
                .add_option(CreateCommandOption::new(CommandOptionType::SubCommand, "stop", "stop a novena")),
            CreateCommand::new("flip").description("Flip The Jouch"),
            CreateCommand::new("flip").kind(CommandType::Message),
            CreateCommand::new("rectify").description("Put The Jouch back upright"),
            CreateCommand::new("birthday").description("Birthday tracking by The Jouch").add_integration_type(serenity::all::InstallationContext::Guild)
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
            CreateCommand::new("autonick").description("Automatic nickname updating tracking by The Jouch").add_integration_type(serenity::all::InstallationContext::Guild)
                .add_option(CreateCommandOption::new(CommandOptionType::SubCommand,"set","set your nickname format string")
                    .add_sub_option(
                        CreateCommandOption::new(CommandOptionType::String, "nickname",
                            "format string; %a becomes age (requires birthday), %j & %f times sat on & flipping The Jouch")
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
            "novena" => novena(&ctx, &command).await,
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

        if let Err(err) = commands {
            warn!("Error setting slash commands: {err}");
        } else {
            trace!("I now have the following slash commands: {:#?}", commands);
        }

        let data = ctx.data.read().await;

        let testing_guild = if let Some(shuttle_items) = data.get::<EnvItemsContainer>() {
            shuttle_items.test_guild
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
                            .description("Migrate data from Ron files to Postgres storage, or export for later import (pass no arguments).")
                            .add_option(CreateCommandOption::new(
                                CommandOptionType::Attachment,
                                "jouch_db",
                                "a jouch_db.ron file, as exported by `/migrate` or from prior to the migration to Shuttle.",
                            ))
                            .add_option(CreateCommandOption::new(
                                CommandOptionType::Attachment,
                                "config",
                                "a config.ron file, as exported by `/migrate` or from prior to the migration to Shuttle.",
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
        // Spawn the nickname, novena, & birthday checkers.
        tokio::spawn(check_nicks_loop(ctx.clone()));
        tokio::spawn(check_birthdays_loop(ctx.clone()));
        tokio::spawn(check_novenas_loop(ctx.clone()));
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

struct EnvItemsContainer {
    test_guild: Option<u64>,
    assets_dir: PathBuf,
}

impl TypeMapKey for EnvItemsContainer {
    type Value = EnvItemsContainer;
}

#[tokio::main]
async fn main() {
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let token = std::env::var("DISCORD_TOKEN").expect("DISCORD_TOKEN must be set");
    let app_id = std::env::var("DISCORD_APP_ID").expect("DISCORD_APP_ID must be set");
    let test_guild = std::env::var("DISCORD_TEST_GUILD").ok();

    let db = sqlx::PgPool::connect(&database_url).await.unwrap();

    // Run SQL migrations
    sqlx::migrate!()
        .run(&db)
        .await
        .expect("Failed to run migrations");

    let app_id = app_id
        .parse::<u64>()
        .expect("Unable to parse app id from secret store!");

    let test_guild = test_guild.and_then(|guild| guild.parse::<u64>().ok());

    const INTENTS: GatewayIntents =
        GatewayIntents::all().difference(GatewayIntents::GUILD_PRESENCES);

    // Login with a bot token from the configuration file
    let mut client = Client::builder(token.as_str(), INTENTS)
        .event_handler(Handler)
        .application_id(app_id.into())
        .await
        .expect("Error creating client");

    let config = config::Config::load(&db)
        .await
        .expect("Unable to load config!");

    trace!("loaded config data: {:#?}", config);

    let shuttle_items = EnvItemsContainer {
        test_guild,
        assets_dir: PathBuf::from("assets"),
    };

    {
        let mut data = client.data.write().await;
        data.insert::<db::Db>(db::Db::new(db));
        data.insert::<config::Config>(config);
        data.insert::<EnvItemsContainer>(shuttle_items);
    }

    // start listening for events by starting the number of shards Discord thinks we need
    if let Err(why) = client.start_autosharded().await {
        println!("An error occurred while running the client: {:?}", why);
    }
}
