use anyhow::anyhow;
use chrono::prelude::*;
use serenity::all::{
    CommandDataOptionValue, CommandInteraction, Context, EditInteractionResponse, EditMember,
    GuildId, MessageBuilder, UserId,
};
use std::{collections::HashSet, time::Duration};
use tracing::{error, info, warn};

use crate::config::Config;
use crate::db::{Db, UserData, UserKey};
use crate::CommandResult;

// how many seconds between nickname update checks?
pub const DEFAULT_INTERVAL: i64 = 600;
pub const fn default_interval() -> i64 {
    DEFAULT_INTERVAL
}

const JOUCH_PAT: &str = "%j";
const FLIP_PAT: &str = "%f";
const AGE_PAT: &str = "%a";
const AGE_PAT2: &str = "%A";

async fn set_nick(
    ctx: &Context,
    guild: GuildId,
    user: UserId,
    nick: Option<String>,
) -> CommandResult<String> {
    let mut data = ctx.data.write().await;
    let db = data
        .get_mut::<Db>()
        .ok_or(anyhow!("Unable to get database"))?;

    let key = UserKey {
        user: user.into(),
        guild: guild.into(),
    };

    let user_data = db.update(&key, "auto_nick", &nick).await?;

    // update immediately
    check_nick_user(ctx, &user_data).await?;

    let mut builder = MessageBuilder::new();
    builder.push("Set nickname to ");
    builder.push_mono_line_safe(match &nick {
        Some(n) => n,
        None => "none",
    });

    if let Some(nick) = nick {
        if (nick.contains(AGE_PAT) || nick.contains(AGE_PAT2)) && !user_data.birthday.is_some() {
            builder
                .push("WARNING: age used in nickname but birthday is not set: use the ")
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
                // we want any guild that has a user (any user) in the user table
                // only the user table matters as with no user there's no nickname to update
                // TODO - could be made even more efficient by filtering to only users with a nickname
                db.get_user_guilds(None).await.unwrap_or_default()
            } else {
                error!("error getting database");
                HashSet::new()
            }
        };
        for guild in &guilds {
            info!("Updating nicknames in guild {}", guild);
            if let Err(e) = check_nicks_in_guild(&ctx, *guild).await {
                warn!("Got error {:?} when updating nicks for {}", e, guild);
            }

            // wait between guild checks
            tokio::time::sleep(Duration::from_secs(interval as u64)).await;
        }
    }
}

async fn check_nicks_in_guild(ctx: &Context, guild: GuildId) -> CommandResult {
    let data = ctx.data.read().await;
    let db = data.get::<Db>().ok_or(anyhow!("Unable to get database"))?;

    let users = db.read_users(guild, "AND auto_nick IS NOT NULL").await?;

    for user in &users {
        if let Err(e) = check_nick_user(ctx, user).await {
            // Don't pass the error up the chain, instead print and move on to the next user in the guild.
            warn!(
                "Error updating nick for user {:?}, {:?}\ncontinuing...",
                user, e
            );
        }
    }

    Ok(())
}

pub async fn check_nick_user_key(ctx: &Context, user_key: &UserKey, db: &Db) -> CommandResult {
    if let Some(user_data) = db.read(user_key).await? {
        check_nick_user(ctx, &user_data).await
    } else {
        Ok(())
    }
}
pub async fn check_nick_user(ctx: &Context, user_data: &UserData) -> CommandResult {
    let nick = if let Some(mut nick) = user_data.auto_nick.clone() {
        if nick.contains(JOUCH_PAT) {
            nick = nick.replace(JOUCH_PAT, &user_data.sit_count.to_string());
        }
        if nick.contains(FLIP_PAT) {
            nick = nick.replace(FLIP_PAT, &user_data.flip_count.to_string());
        }
        if nick.contains(AGE_PAT) || nick.contains(AGE_PAT2) {
            if let Some(birthday) = user_data.birthday {
                let birthday = birthday.naive_utc();
                let now = Utc::now().naive_utc();
                // TODO - If with_year() fails this will silently block the nickname update
                let birthday_thisyear = birthday
                    .with_year(now.year())
                    .ok_or(anyhow!("error with date modification"))?;

                let (next_birthday, last_birthday) = if birthday_thisyear > now {
                    // birthday_thisyear is in the future
                    (
                        birthday_thisyear,
                        birthday_thisyear
                            .with_year(now.year() - 1)
                            .ok_or(anyhow!("error with date modification"))?,
                    )
                } else {
                    // birthday_thisyear is in the past
                    (
                        birthday_thisyear
                            .with_year(now.year() + 1)
                            .ok_or(anyhow!("error with date modification"))?,
                        birthday_thisyear,
                    )
                };

                let diff = last_birthday.year() - birthday.year();
                // using (next_birthday - last_birthday) rather than hard-coding year length means it will account for leap years properly.
                let percent = (now - last_birthday).num_days() as f64
                    / (next_birthday - last_birthday).num_days() as f64;

                nick = nick.replace(AGE_PAT, &diff.to_string());
                nick = nick.replace(AGE_PAT2, &format!("{:.4}", diff as f64 + percent));
            }
        }
        Some(nick)
    } else {
        None
    };

    if let Some(nick) = nick {
        info!("Updating nick for user {}", user_data.id.user);
        GuildId::new(user_data.id.guild as u64)
            .edit_member(
                &ctx.http,
                UserId::new(user_data.id.user as u64),
                EditMember::new().nickname(nick),
            )
            .await?;
    }

    Ok(())
}

pub async fn autonick(ctx: &Context, command: &CommandInteraction) -> CommandResult {
    if let Some(subcommand) = command.data.options.get(0) {
        let guild = command
            .guild_id
            .ok_or(anyhow!("Unable to get guild where command was sent"))?;
        match subcommand.name.as_str() {
            "set" => {
                if let CommandDataOptionValue::SubCommand(subcommand_args) = &subcommand.value {
                    if let Some(nick_arg) = subcommand_args.first() {
                        if let CommandDataOptionValue::String(nick_str) = &nick_arg.value {
                            let response =
                                set_nick(ctx, guild, command.user.id, Some(nick_str.clone()))
                                    .await?;

                            command
                                .edit_response(
                                    &ctx,
                                    EditInteractionResponse::new().content(response),
                                )
                                .await?;

                            return Ok(());
                        }
                    }
                }
                Err(anyhow!("No format string argument passed"))
            }
            "clear" => {
                let response = set_nick(ctx, guild, command.user.id, None).await?;

                command
                    .edit_response(&ctx, EditInteractionResponse::new().content(response))
                    .await?;

                Ok(())
            }
            _ => Err(anyhow!("Unknown option {}", subcommand.name).into()),
        }
    } else {
        Err(anyhow!("Please provide a valid subcommand"))
    }
}
