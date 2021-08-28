use std::{collections::HashSet,time::Duration};
use serenity::framework::standard::{macros::command, Args, CommandResult};
use serenity::model::prelude::*;
use serenity::prelude::*;
use serenity::utils::MessageBuilder;
use crate::config::Config;
use crate::db::{Db, UserKey};

// how many seconds between nickname update checks?
pub const DEFAULT_INTERVAL: u64 = 600;
pub const fn default_interval() -> u64 { DEFAULT_INTERVAL }

async fn set_nick(ctx: &Context, msg: &Message, nick: Option<String>) -> CommandResult<String> {
    let mut data = ctx.data.write().await;
    let db = data.get_mut::<Db>().ok_or("Unable to get database")?;

    let key = UserKey {
        user: msg.author.id, 
        guild: msg.guild_id.ok_or("Unable to get guild where command was sent")?,
    };

    db.update(&key, |data| { 
        data.auto_nick = nick.clone()
    })?;

    // update immediately
    check_nick_user(ctx, &key, db).await?;

    Ok(MessageBuilder::new()
        .push("Set nickname to ")
        .push_mono_safe(match nick {
            Some(n) => n,
            None => "none".into(),
        })
        .build())
}

// function to be spun off into its own thread to periodically check for nickname updates
pub async fn check_nicks_loop(ctx: Context) {
    // get the update interval from the config file if possible
    let interval = {
        let data = ctx.data.read().await;
        if let Some(config) = data.get::<Config>() {
            config.nick_interval
        } else {
            // we really should always be able to get the config, but just in case fallback to default.
            DEFAULT_INTERVAL
        }
    };
    
    loop {
        let guilds = {
            let data = ctx.data.read().await;
            if let Some(db) = data.get::<Db>() {
                db.get_guilds().unwrap_or_default()
            } else {
                println!("error getting database");
                HashSet::new()
            }
        };
        for guild in &guilds {
            println!("Updating nicknames in guild {}", guild);
            if let Err(e) = check_nicks_in_guild(&ctx, *guild).await {
                println!("Got error {:?} when updating nicks for {}", e, guild);
            }

            // wait between guild checks
            tokio::time::sleep(Duration::from_secs(interval)).await;
        }
    }
}

async fn check_nicks_in_guild(ctx: &Context, guild: GuildId) -> CommandResult {
    let data = ctx.data.read().await;
    let db = data.get::<Db>().ok_or("Unable to get database")?;

    let users = db.get_users(guild)?;

    for user in users {
        if let Err(e) = check_nick_user(ctx, &user, db).await {
            // Don't pass the error up the chain, instead print and move on to the next user in the guild.
            println!("Error updating nick for user {:?}, {:?}\ncontinuing...", user, e);
        }
    }
    
    Ok(())
}

async fn check_nick_user(ctx: &Context, user_key: &UserKey, db: &Db) -> CommandResult {
    let nick = db.read(&user_key, |data| {
        data.auto_nick.clone()
    })?.ok_or("error getting user")?;

    if let Some(nick) = nick {
        user_key.guild.edit_member(&ctx.http, user_key.user, |e|{
            println!("Updating nick for user {}", user_key.user);
            // TODO - actually make this useful with format options (e.g. user's age)
            e.nickname(nick)
        }).await?;
    }

    Ok(())
}

#[command]
pub async fn autonick(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    let subcommand = args.single::<String>()?.to_lowercase();
    
    let response = match subcommand.as_str() {
        "set" => {
            // TODO - there should probably be some verification/filtering on this...
            let nick = args.single_quoted::<String>()?;
            set_nick(ctx, msg, Some(nick)).await?
        },
        "clear" => {
            set_nick(ctx, msg, None).await?
        },
        _ => {
            MessageBuilder::new()
                .push("Unknown subcommand ")
                .push_mono_safe(subcommand)
                .build()
        },
    };

    msg.reply(ctx, response).await?;

    Ok(())
}
