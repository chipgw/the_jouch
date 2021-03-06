use std::collections::HashSet;
use std::str::FromStr;
use serenity::framework::standard::{macros::command, Args, CommandResult};
use serenity::model::interactions::application_command::{ApplicationCommandInteraction, ApplicationCommandInteractionDataOptionValue};
use serenity::model::prelude::*;
use serenity::prelude::*;
use serenity::utils::MessageBuilder;
use chrono::{Duration, prelude::*};
use serde::{Serialize, Deserialize};
use enum_utils::FromStr;
use crate::db::{Db, UserKey, UserData};

use super::autonick::check_nick_user;

const DATE_OPTIONS: &[&str] = &[
    "%F",           // e.g. 1990-01-01
];
const TIME_OPTIONS: &[&str] = &[
    "T%H:%M%:z",    // e.g. T12:30
    "T%H:%M%#z",    // e.g. T12:30
    "",             // no time provided
];

#[derive(Eq, PartialEq, Debug, Serialize, Deserialize, Clone, Copy, FromStr)]
#[enumeration(case_insensitive)]
pub enum BirthdayPrivacy {
    // Public YMD
    #[enumeration(alias = "Public")]
    PublicFull,

    // Only MD public
    #[enumeration(alias = "DayOnly", alias = "MonthDay")]
    PublicDay,

    // Only known to the bot (to be used internally)
    #[enumeration(alias = "Hidden")]
    Private,
}
impl BirthdayPrivacy {
    pub fn date_format(&self) -> &str {
        match self {
            Self::PublicFull => "%F",
            Self::PublicDay => "%m-%d",
            Self::Private => "private",
        }
    }
}

// would have just made this const but there's no way to do a const Date as far as I can tell
#[inline]
pub fn get_bot_birthday() -> NaiveDate {
    NaiveDate::from_ymd(2021, 7, 31)
}

pub fn parse_date(date_str: &str) -> CommandResult<DateTime<FixedOffset>> {
    // Default to CST
    let default_offset: FixedOffset = FixedOffset::west(21600);

    let mut format_str = String::with_capacity(10);
    for date_option in DATE_OPTIONS {
        for time_option in TIME_OPTIONS {
            format_str.clear();
            format_str.push_str(date_option);
            format_str.push_str(time_option);

            print!("Trying format_str: \"{}\"", format_str);

            // Trying with no time zone uses different parse function than trying with time zone
            let parsed_date = if time_option.is_empty() {
                NaiveDate::parse_from_str(date_str, format_str.as_str()).map(|datetime|{
                    default_offset.from_local_datetime(&datetime.and_time(NaiveTime::from_hms(0, 0, 0))).unwrap()
                })
            } else {
                DateTime::parse_from_str(date_str, format_str.as_str())
            };

            match parsed_date {
                Ok(date) => return Ok(date),
                Err(err) => println!(" failed with reason: {}", err),
            }
        }
    }
    Err(format!("Unable to parse date '{}'", date_str).into())
}

pub async fn clear_birthday(ctx: &Context, guild: GuildId, user: UserId) -> CommandResult<String> {    
    let mut data = ctx.data.write().await;
    let db = data.get_mut::<Db>().ok_or("Unable to get database")?;

    let key = UserKey { user, guild };

    db.update(&key, |data| { 
        data.birthday = None;
    })?;

    // try updating the user nickname but ignore if it fails.
    let _ = check_nick_user(ctx, &key, db).await;

    Ok(MessageBuilder::new()
        .push("Cleared birthday")
        .build())
}

pub async fn set_birthday(ctx: &Context, guild: GuildId, user: UserId, date_str: &str, privacy: Option<BirthdayPrivacy>) -> CommandResult<String> {
    let date = parse_date(date_str)?;
    
    let mut data = ctx.data.write().await;
    let db = data.get_mut::<Db>().ok_or("Unable to get database")?;

    let key = UserKey { user, guild };

    db.update(&key, |data| { 
        data.birthday = Some(date);
        data.birthday_privacy = privacy;
    })?;

    // try updating the user nickname but ignore if it fails.
    let _ = check_nick_user(ctx, &key, db).await;

    Ok(MessageBuilder::new()
        .push("Set birthday to ")
        .push_bold_safe(date.format(privacy.unwrap_or(BirthdayPrivacy::PublicFull).date_format()))
        .build())
}

fn birthday_date_check(day: Date<Local>, user_data: &UserData) -> bool {
    if let Some(birthday) = user_data.birthday {
        day.day() == birthday.day() && day.month() == birthday.month() && user_data.birthday_privacy != Some(BirthdayPrivacy::Private)
    } else {
        false
    }
}

pub async fn is_birthday_today(ctx: &Context, user_key: UserKey) -> CommandResult<bool> {
    let now = Local::today();
    if user_key.user == ctx.cache.current_user_id() {
        let bot_birthday = get_bot_birthday();
        return Ok(now.day() == bot_birthday.day() && now.month() == bot_birthday.month());
    }
    let data = ctx.data.read().await;
    let db = data.get::<Db>().ok_or("Unable to get database")?;
    Ok(db.read(&user_key, |user_data|{
        birthday_date_check(now, user_data)
    })?.unwrap_or_default())
}

pub async fn todays_birthdays(ctx: &Context, guild: GuildId) -> CommandResult<String> {
    let mut message = MessageBuilder::new();
    message.push("Birthdays today: ");
    let mut birthday_count = 0;

    let data = ctx.data.read().await;
    let db = data.get::<Db>().ok_or("Unable to get database")?;
    let now = Local::today();
    db.foreach(guild, |user_id,user_data|{
        if birthday_date_check(now, user_data) {
            message.mention(&UserId(*user_id));
            birthday_count += 1;
        }
    })?;
    let bot_birthday = get_bot_birthday();
    if now.day() == bot_birthday.day() && now.month() == bot_birthday.month() {
        message.mention(&ctx.cache.current_user_id());
        birthday_count += 1;
    }
    if birthday_count == 0 {
        message.push("None");
    } else {
        message.push("\nHappy Birthday!");
    }
    Ok(message.build())
}

pub async fn user_birthdays(ctx: &Context, guild: GuildId, users: &Vec<User>) -> CommandResult<String> {
    let mut message = MessageBuilder::new();
    let now = Local::today();
    for user in users {
        message.mention(user).push("'s birthday is ");

        if user.id == ctx.cache.current_user_id() {
            let bot_birthday = get_bot_birthday();
            if now.day() == bot_birthday.day() && now.month() == bot_birthday.month() {
                message.push_line("today! Happy Birthday!");
            } else {
                message.push_line(bot_birthday);
            }
            continue;
        }

        let key = UserKey {
            user: user.id, 
            guild,
        };

        let data = ctx.data.read().await;
        let db = data.get::<Db>().ok_or("Unable to get database")?;
        db.read(&key, |user_data|{
            match user_data.birthday {
                Some(birthday) => {
                    if now.day() == birthday.day() && now.month() == birthday.month() && user_data.birthday_privacy != Some(BirthdayPrivacy::Private) {
                        message.push_line("today! Happy Birthday!");
                    } else if let Some(privacy) = user_data.birthday_privacy {
                        message.push_line(birthday.format(privacy.date_format()));
                    } else {
                        message.push_line(birthday.date());
                    }
                },
                None => {
                    message.push_line("not set");
                },
            }
        })?;
    }
    Ok(message.build())
}

#[command]
pub async fn birthday(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    let subcommand = if args.is_empty() {
        // Assume that when !birthday is called with no arguments that the user wants to run the check subcommand
        "check".into()
    } else {
        args.single::<String>()?.to_lowercase()
    };
    
    let guild = msg.guild_id.ok_or("Unable to get guild where command was sent")?;

    let response = match subcommand.as_str() {
        "set" => {
            let date_str = args.current().ok_or("No date argument passed")?.to_owned();
            args.advance();

            // TODO - edit or remove original message to maintain privacy if it's set to private
            let privacy = args.find::<BirthdayPrivacy>().ok();

            set_birthday(ctx, guild, msg.author.id, &date_str, privacy).await?
        },
        "check" => {
            if msg.mentions.is_empty() {
                todays_birthdays(ctx, guild).await?
            } else {
                user_birthdays(ctx, guild, &msg.mentions).await?
            }
        },
        "clear" => {
            clear_birthday(ctx, guild, msg.author.id).await?
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

// function to be spun off into its own thread to periodically check for birthdays
pub async fn check_birthdays_loop(ctx: Context) {
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
            let (announce_channel, announce_when_none) = {
                let data = ctx.data.read().await;
                if let Some(db) = data.get::<Db>() {
                    let data = db.read_guild(*guild, |guild_data|{
                        (guild_data.birthday_announce_channel, guild_data.birthday_announce_when_none)
                    });
                    match data {
                        Ok(data) => data.unwrap_or_default(),
                        Err(err) => {
                            println!("error getting guild data for guild {}; {:?}", guild, err);
                            (None, None)
                        }
                    }
                } else {
                    println!("error getting database");
                    (None, None)
                }
            };
            
            if let Some(channel_id) = announce_channel {
                println!("chacking birthdays in guild {}", guild);
                match todays_birthdays(&ctx, (*guild).into()).await {
                    Err(err) => {
                        println!("got error {:?} when checking birthdays for {}", err, guild);
                    },
                    Ok(msg) => {
                        if announce_when_none.unwrap_or_default() || !msg.contains("None") {
                            // Birthday announcement happens today
                            if let Err(err) = ChannelId(channel_id).say(&ctx.http, msg).await {
                                println!("got error {:?} when sending birthday alert for {}", err, guild);
                            }
                        }
                    },
                }
            }
        }

        // check every day at 8 am local time (where bot is run)
        let next = Local::today().and_hms(8, 0, 0) + Duration::days(1);

        // wait until next check
        tokio::time::sleep((next - Local::now()).to_std().unwrap()).await;
    }
}

pub async fn birthday_slashcommand(ctx: &Context, command: &ApplicationCommandInteraction) -> CommandResult {
    if let Some(subcommand) = command.data.options.get(0) {
        let guild = command.guild_id.ok_or("Unable to get guild where command was sent")?;
        match subcommand.name.as_str() {
            "set" => {
                if let Some(date_arg) = subcommand.options.first() {
                    if let Some(ApplicationCommandInteractionDataOptionValue::String(date_str)) = &date_arg.resolved {
                        
                        let privacy = subcommand.options.iter().find_map(|x|{
                            if let Some(ApplicationCommandInteractionDataOptionValue::String(arg_str)) = &x.resolved {
                                BirthdayPrivacy::from_str(arg_str).ok()
                            } else {
                                None
                            }
                        });

                        let response = set_birthday(ctx, guild, command.user.id, date_str, privacy).await?;

                        command.edit_original_interaction_response(&ctx.http, |r| {
                            r.content(response)
                        }).await?;

                        return Ok(())
                    }
                }
                Err("No date argument passed".into())
            },
            "clear" => {
                let response = clear_birthday(ctx, guild, command.user.id).await?;

                command.edit_original_interaction_response(&ctx.http, |r| {
                    r.content(response)
                }).await?;

                Ok(())
            },
            "check" => {
                let users: Vec<User> = subcommand.options.iter().filter_map(|x|{
                    if let Some(ApplicationCommandInteractionDataOptionValue::User(user, _)) = &x.resolved {
                        Some(user.clone())
                    } else {
                        None
                    }
                }).collect();

                let response = if users.is_empty() {
                    todays_birthdays(ctx, guild).await?
                } else {
                    user_birthdays(ctx, guild, &users).await?
                };

                command.edit_original_interaction_response(&ctx.http, |r| {
                    r.content(response)
                }).await?;

                Ok(())
            }
            _ => {
                Err(format!("Unknown option {}", subcommand.name).into())
            }
        }
    } else {
        Err("Please provide a valid subcommand".into())
    }
}
