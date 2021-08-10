mod commands;
mod db;
mod config;

use std::{
    collections::HashSet,
};

use serenity::{
    async_trait,
    client::{Client, Context, EventHandler},
    model::{
        channel::Message,
        gateway::Ready,
    },
    framework::standard::{
        StandardFramework,
        macros::{group, hook},
        DispatchError,
        CommandResult,
    },
    utils::MessageBuilder,
    http::Http,
};

use commands::{birthday::*, clear::*, sit::*};

#[group]
#[commands(birthday, clear_from, sit)]
struct General;

struct Handler;

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, context: Context, msg: Message) {
        if msg.content == "~ping" {
            let channel = match msg.channel_id.to_channel(&context).await {
                Ok(channel) => channel,
                Err(why) => {
                    println!("Error getting channel: {:?}", why);

                    return;
                },
            };

            // The message builder allows for creating a message by
            // mentioning users dynamically, pushing "safe" versions of
            // content (such as bolding normalized content), displaying
            // emojis, and more.
            let response = MessageBuilder::new()
                .push("User ")
                .push_bold_safe(&msg.author.name)
                .push(" used the 'ping' command in the ")
                .mention(&channel)
                .push(" channel")
                .build();

            if let Err(why) = msg.channel_id.say(&context.http, &response).await {
                println!("Error sending message: {:?}", why);
            }
        }
    }

    async fn ready(&self, _: Context, ready: Ready) {
        println!("{} is connected!", ready.user.name);
    }
}

#[tokio::main]
async fn main() {
    let config = config::Config::load().expect("Unable to init config!");
    // make sure config file is created.
    config.save().expect("Unable to save config file!");

    let http = Http::new_with_token(&config.token);

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
        },
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
        .normal_message(normal_message);

    // Login with a bot token from the configuration file
    let mut client = Client::builder(config.token.as_str())
        .event_handler(Handler)
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
async fn dispatch_error(context: &Context, msg: &Message, error: DispatchError) {
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

    // Increment the number of times this command has been run once. If
    // the command's name does not exist in the counter, add a default
    // value of 0.
    // let mut data = ctx.data.write().await;
    // let counter = data.get_mut::<CommandCounter>().expect("Expected CommandCounter in TypeMap.");
    // let entry = counter.entry(command_name.to_string()).or_insert(0);
    // *entry += 1;

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

#[hook]
async fn normal_message(_ctx: &Context, msg: &Message) {
    println!("Message is not a command '{}'", msg.content);
}

#[hook]
async fn delay_action(ctx: &Context, msg: &Message) {
    // You may want to handle a Discord rate limit if this fails.
    let _ = msg.react(ctx, '⏱').await;
}
