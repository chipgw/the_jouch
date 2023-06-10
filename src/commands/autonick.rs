use std::{collections::HashSet,time::Duration};
use serenity::model::application::interaction::application_command::{ApplicationCommandInteraction, CommandDataOptionValue};
use serenity::model::prelude::*;
use serenity::prelude::*;
use serenity::utils::MessageBuilder;
use chrono::prelude::*;
use crate::config::Config;
use crate::db::{Db, UserKey};
use crate::CommandResult;

// how many seconds between nickname update checks?
pub const DEFAULT_INTERVAL: u64 = 600;
pub const fn default_interval() -> u64 { DEFAULT_INTERVAL }

const JOUCH_PAT: &str = "%j";
const AGE_PAT: &str = "%a";
const AGE_PAT2: &str = "%A";

async fn set_nick(ctx: &Context, guild: GuildId, user: UserId, nick: Option<String>) -> CommandResult<String> {
    let mut data = ctx.data.write().await;
    let db = data.get_mut::<Db>().ok_or("Unable to get database")?;

    let key = UserKey {
        user,
        guild,
    };

    let has_birthday = db.update(&key, |data| { 
        data.auto_nick = nick.clone();
        data.birthday.is_some()
    })?;

    // update immediately
    check_nick_user(ctx, &key, db).await?;

    let mut builder = MessageBuilder::new();
    builder.push("Set nickname to ");
    builder.push_mono_line_safe(match &nick {
        Some(n) => n,
        None => "none",
    });

    if let Some(nick) = nick {
        if (nick.contains(AGE_PAT) || nick.contains(AGE_PAT2)) && !has_birthday {
            builder.push("WARNING: age used in nickname but birthday is not set: use the ")
                .push_mono("/birthday")
                .push_line(" command to set.");
        }
    }

    Ok(builder.build())
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

pub async fn check_nick_user(ctx: &Context, user_key: &UserKey, db: &Db) -> CommandResult {
    let nick = db.read(&user_key, |data| {
        if let Some(mut nick) = data.auto_nick.clone() {

            if nick.contains(JOUCH_PAT) {
                nick = nick.replace(JOUCH_PAT, &data.sit_count.unwrap_or_default().to_string());
            }
            if nick.contains(AGE_PAT) || nick.contains(AGE_PAT2) {
                if let Some(birthday) = data.birthday {
                    let birthday = birthday.naive_utc();
                    let now = Utc::now().naive_utc();
                    // TODO - If with_year() fails this will silently block the nickname update
                    let birthday_thisyear = birthday.with_year(now.year())?;

                    let (next_birthday, last_birthday) = if birthday_thisyear > now {
                        // birthday_thisyear is in the future
                        (birthday_thisyear, birthday_thisyear.with_year(now.year() - 1)?)
                    } else {
                        // birthday_thisyear is in the past
                        (birthday_thisyear.with_year(now.year() + 1)?, birthday_thisyear)
                    };

                    let diff = last_birthday.year() - birthday.year();
                    // using (next_birthday - last_birthday) rather than hard-coding year length means it will account for leap years properly.
                    let percent = (now - last_birthday).num_days() as f64 / (next_birthday - last_birthday).num_days() as f64;

                    nick = nick.replace(AGE_PAT, &diff.to_string());
                    nick = nick.replace(AGE_PAT2, &format!("{:.4}", diff as f64 + percent));
                }
            }
            Some(nick)
        } else {
            None
        }
    })?.ok_or("error getting user")?;

    if let Some(nick) = nick {
        user_key.guild.edit_member(&ctx.http, user_key.user, |e|{
            println!("Updating nick for user {}", user_key.user);
            e.nickname(nick)
        }).await?;
    }

    Ok(())
}

pub async fn autonick(ctx: &Context, command: &ApplicationCommandInteraction) -> CommandResult {
    if let Some(subcommand) = command.data.options.get(0) {
        let guild = command.guild_id.ok_or("Unable to get guild where command was sent")?;
        match subcommand.name.as_str() {
            "set" => {
                if let Some(nick_arg) = subcommand.options.first() {
                    if let Some(CommandDataOptionValue::String(nick_str)) = &nick_arg.resolved {                        
                        let response = set_nick(ctx, guild, command.user.id, Some(nick_str.clone())).await?;

                        command.edit_original_interaction_response(&ctx.http, |r| {
                            r.content(response)
                        }).await?;

                        return Ok(())
                    }
                }
                Err("No format string argument passed".into())
            },
            "clear" => {
                set_nick(ctx, guild, command.user.id, None).await?;

                Ok(())
            },
            _ => {
                Err(format!("Unknown option {}", subcommand.name).into())
            },
        }
    } else {
        Err("Please provide a valid subcommand".into())
    }
}
