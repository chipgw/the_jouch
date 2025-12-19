use crate::db::{Db, UserData, UserKey};
use crate::CommandResult;
use anyhow::anyhow;
use chrono::format::{parse, Parsed, StrftimeItems};
use chrono::{prelude::*, Duration};
use enum_utils::FromStr;
use serde::{Deserialize, Serialize};
use serenity::all::{
    ChannelId, CommandDataOptionValue, CommandInteraction, Context, EditInteractionResponse,
    GuildId, MessageBuilder, UserId,
};
use std::collections::HashSet;
use std::str::FromStr;
use tracing::{error, info, trace, warn};

use super::autonick::check_nick_user;

const DATE_OPTIONS: &[&str] = &[
    "%F",       // e.g. 1990-01-30
    "%m/%d/%Y", // e.g. 01/30/1990
];
const TIME_OPTIONS: &[&str] = &[
    "%l:%M%p", // e.g. 12:30pm
    "%l:%M%P", // e.g. 12:30PM
    "%H:%M",   // e.g. 12:30
    "%l%p",    // e.g. 12pm
    "%l%P",    // e.g. 12PM
];
const ZONE_OPTIONS: &[&str] = &[
    "%:z", // e.g. -05:00
    "%#z", // e.g. -05 or -0500
];
// Default to CST
const DEFAULT_OFFSET: FixedOffset = FixedOffset::west_opt(21600).unwrap();

#[derive(Eq, PartialEq, Debug, Serialize, Deserialize, Clone, Copy, FromStr, sqlx::Type)]
#[sqlx(type_name = "birthday_privacy")]
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
    NaiveDate::from_ymd_opt(2021, 7, 31).unwrap()
}
#[inline]
pub fn get_jesus_birthday() -> NaiveDate {
    NaiveDate::from_ymd_opt(0, 12, 25).unwrap()
}

pub fn parse_date(
    in_str: &str,
    default_time: Option<NaiveTime>,
    default_date: Option<DateTime<FixedOffset>>,
) -> CommandResult<DateTime<FixedOffset>> {
    let mut parsed = Parsed::new();

    // TODO - all this splitting could probably stand to be a little more robust...
    let (date_str, time_str) = in_str.split_once([' ', 'T']).unwrap_or((in_str, in_str));
    // if above split failed, time_str may have '-' from the date, but if that's the case it doesn't contain the time anyway so who cares
    let (time_str, zone_str) = time_str
        .split_at_checked(time_str.rfind(['+', '-']).unwrap_or(usize::MAX))
        .unwrap_or((time_str, time_str));

    let has_date = DATE_OPTIONS.iter().any(|date_option| {
        trace!("Trying date format string: `{date_option}` on {date_str}");
        if let Err(err) = parse(&mut parsed, date_str, StrftimeItems::new(date_option)) {
            trace!(" parse failed with reason: {}", err);
            false
        } else {
            true
        }
    });
    let has_time = TIME_OPTIONS.iter().any(|time_option| {
        trace!("Trying time format string: `{time_option}` on {time_str}");
        if let Err(err) = parse(&mut parsed, time_str, StrftimeItems::new(time_option)) {
            trace!(" parse failed with reason: {}", err);
            false
        } else {
            true
        }
    });
    let _has_zone = ZONE_OPTIONS.iter().any(|zone_option| {
        trace!("Trying zone format string: `{zone_option}` on {zone_str}");
        if let Err(err) = parse(&mut parsed, zone_str, StrftimeItems::new(zone_option)) {
            trace!(" parse failed with reason: {}", err);
            false
        } else {
            true
        }
    });

    trace!("parsed: {:?}", parsed);

    let parsed_date = if !has_time {
        if let Some(default_time) = default_time {
            parsed.to_naive_date().map(|date| {
                date.and_time(default_time)
                    .and_local_timezone(default_date.map_or(DEFAULT_OFFSET, |x| x.timezone()))
                    .unwrap()
            })
        } else {
            return Err(anyhow!("Only date passed, but time is required!"));
        }
    } else if !has_date {
        if let Some(default_date) = default_date {
            let offset = parsed.to_fixed_offset().unwrap_or(DEFAULT_OFFSET);
            // convert the default date into the timezone being supplied before joining
            let converted_default = default_date.with_timezone(&offset).date_naive();

            parsed.to_naive_time().map(|time| {
                converted_default
                    .and_time(time)
                    .and_local_timezone(offset)
                    .unwrap()
            })
        } else {
            return Err(anyhow!("Only time passed, but date is required!"));
        }
    } else {
        // This will only set if it isn't already.
        let _ = parsed.set_offset(DEFAULT_OFFSET.local_minus_utc() as i64);
        parsed.to_datetime()
    };

    match parsed_date {
        Ok(date) => {
            trace!("parsed date: {date}");
            return Ok(date);
        }
        Err(err) => trace!(" conversion failed with reason: {err}"),
    }

    Err(anyhow!(
        "Unable to parse date/time '{}'
        Supported date formats are: `YYYY-MM-DD` & `MM/DD/YYYY`
        Supported time formats are: `HHam`, `HHAM`, `HH:MMam`, `HH:MMAM`, & `HH:MM` (24 hour)
        Time can be followed by a time zone offset from UTC; supported formats: `±ZZ:ZZ`, `±ZZZZ`, or `±ZZ`
        When entering both date & time, use a space or a 'T' to separate them, e.g. `2021-07-31T15:00-0500`
        Time zone is assumed to be CST if not provided
        Not all parsed components may be relevant everywhere parsing function is used (e.g. time isn't usually useful for birthdays)
        All this complicated mess could be avoided if Discord added a date picker component",
        date_str
    ))
}

pub async fn clear_birthday(ctx: &Context, guild: GuildId, user: UserId) -> CommandResult<String> {
    let data = ctx.data.read().await;
    let db = data.get::<Db>().ok_or(anyhow!("Unable to get database"))?;

    let key = UserKey {
        user: user.into(),
        guild: guild.into(),
    };

    let user_data = db
        .update(&key, "birthday", Option::<DateTime<FixedOffset>>::None)
        .await?;

    // try updating the user nickname but ignore if it fails.
    let _ = check_nick_user(ctx, &user_data).await;

    Ok("Cleared birthday".into())
}

pub async fn set_birthday(
    ctx: &Context,
    guild: GuildId,
    user: UserId,
    date_str: &str,
    privacy: Option<BirthdayPrivacy>,
) -> CommandResult<String> {
    let date = parse_date(date_str, Some(NaiveTime::default()), None)?;

    let data = ctx.data.read().await;
    let db = data.get::<Db>().ok_or(anyhow!("Unable to get database"))?;

    let key = UserKey {
        user: user.into(),
        guild: guild.into(),
    };

    // TODO - make it so these can be done in one query
    db.update(&key, "birthday", date).await?;
    let user_data = db.update(&key, "birthday_privacy", privacy).await?;

    // try updating the user nickname but ignore if it fails.
    let _ = check_nick_user(ctx, &user_data).await;

    Ok(MessageBuilder::new()
        .push("Set birthday to ")
        .push_bold_safe(
            date.format(privacy.unwrap_or(BirthdayPrivacy::PublicFull).date_format())
                .to_string(),
        )
        .build())
}

fn birthday_date_check(day: DateTime<Local>, user_data: &UserData) -> bool {
    if let Some(birthday) = user_data.birthday {
        day.day() == birthday.day()
            && day.month() == birthday.month()
            && user_data.birthday_privacy != Some(BirthdayPrivacy::Private)
    } else {
        false
    }
}

pub async fn is_birthday_today(ctx: &Context, user_key: UserKey) -> CommandResult<bool> {
    let now = Local::now();
    if user_key.user == ctx.cache.current_user().id.get() as i64 {
        let bot_birthday = get_bot_birthday();
        return Ok(now.day() == bot_birthday.day() && now.month() == bot_birthday.month());
    }
    let data = ctx.data.read().await;
    let db = data.get::<Db>().ok_or(anyhow!("Unable to get database"))?;

    if let Some(user_data) = db.read(&user_key).await? {
        Ok(birthday_date_check(now, &user_data))
    } else {
        Ok(false)
    }
}

pub async fn todays_birthdays(ctx: &Context, guild: GuildId) -> CommandResult<String> {
    let mut message = MessageBuilder::new();
    message.push("Birthdays today: ");
    let mut birthday_count = 0;

    let data = ctx.data.read().await;
    let db = data.get::<Db>().ok_or(anyhow!("Unable to get database"))?;

    let users = db
        .read_users(
            guild,
            "AND DATE_PART('doy', birthday) = DATE_PART('doy', CURRENT_DATE)",
        )
        .await?;
    for user_data in &users {
        if user_data.birthday_privacy != Some(BirthdayPrivacy::Private) {
            if birthday_count > 0 {
                message.push(", ");
            }
            message.mention(&UserId::new(user_data.id.user as u64));
            birthday_count += 1;
        }
    }
    let now = Local::now();
    let bot_birthday = get_bot_birthday();
    if now.day() == bot_birthday.day() && now.month() == bot_birthday.month() {
        if birthday_count > 0 {
            message.push(", ");
        }
        message.mention(&ctx.cache.current_user().id);
        birthday_count += 1;
    }
    let jesus_birthday = get_jesus_birthday();
    if now.day() == jesus_birthday.day() && now.month() == jesus_birthday.month() {
        if birthday_count > 0 {
            message.push(", ");
        }
        message.push("Jesus");
        birthday_count += 1;
    }
    if birthday_count == 0 {
        message.push("None");
    } else {
        message.push("\nHappy Birthday!");
    }
    Ok(message.build())
}

pub async fn user_birthdays(
    ctx: &Context,
    guild: GuildId,
    users: Vec<UserId>,
) -> CommandResult<String> {
    let mut message = MessageBuilder::new();
    let now = Local::now();
    for user in users {
        message.mention(&user).push("'s birthday is ");

        if user == ctx.cache.current_user().id {
            let bot_birthday = get_bot_birthday();
            if now.day() == bot_birthday.day() && now.month() == bot_birthday.month() {
                message.push_line("today! Happy Birthday!");
            } else {
                message.push_line(bot_birthday.to_string());
            }
            continue;
        }

        let key = UserKey {
            user: user.into(),
            guild: guild.into(),
        };

        let data = ctx.data.read().await;
        let db = data.get::<Db>().ok_or(anyhow!("Unable to get database"))?;
        if let Some(user_data) = db.read(&key).await? {
            match user_data.birthday {
                Some(birthday) => {
                    if now.day() == birthday.day()
                        && now.month() == birthday.month()
                        && user_data.birthday_privacy != Some(BirthdayPrivacy::Private)
                    {
                        message.push_line("today! Happy Birthday!");
                    } else if let Some(privacy) = user_data.birthday_privacy {
                        message.push_line(birthday.format(privacy.date_format()).to_string());
                    } else {
                        message.push_line(birthday.date_naive().to_string());
                    }
                }
                None => {
                    message.push_line("not set");
                }
            }
        } else {
            message.push_line("not set");
        }
    }
    Ok(message.build())
}

// function to be spun off into its own thread to periodically check for birthdays
pub async fn check_birthdays_loop(ctx: Context) {
    loop {
        let guilds = {
            let data = ctx.data.read().await;
            if let Some(db) = data.get::<Db>() {
                // If a guild has a channel set it will appear in the guilds table, so no need to check user table.
                db.get_guilds().await.unwrap_or_default()
            } else {
                error!("error getting database");
                HashSet::new()
            }
        };
        for guild in &guilds {
            let (announce_channel, announce_when_none) = {
                let data = ctx.data.read().await;
                if let Some(db) = data.get::<Db>() {
                    match db.read_guild(*guild).await {
                        Ok(data) => data
                            .map(|guild_data| {
                                (
                                    guild_data.birthday_announce_channel,
                                    guild_data.birthday_announce_when_none,
                                )
                            })
                            .unwrap_or_default(),
                        Err(err) => {
                            error!("error getting guild data for guild {}; {:?}", guild, err);
                            (None, None)
                        }
                    }
                } else {
                    error!("error getting database");
                    (None, None)
                }
            };

            if let Some(channel_id) = announce_channel {
                info!("checking birthdays in guild {}", guild);
                match todays_birthdays(&ctx, (*guild).into()).await {
                    Err(err) => {
                        error!("got error {:?} when checking birthdays for {}", err, guild);
                    }
                    Ok(msg) => {
                        if announce_when_none.unwrap_or_default() || !msg.contains("None") {
                            // Birthday announcement happens today
                            if let Err(err) =
                                ChannelId::new(channel_id as u64).say(&ctx.http, msg).await
                            {
                                warn!(
                                    "got error {:?} when sending birthday alert for {}",
                                    err, guild
                                );
                            }
                        }
                    }
                }
            }
        }

        // check every day at 8 am local time (where bot is run)
        let next = Local::now().date_naive().and_hms_opt(8, 0, 0).unwrap() + Duration::days(1);

        // wait until next check
        tokio::time::sleep(
            (Local.from_local_datetime(&next).unwrap() - Local::now())
                .to_std()
                .unwrap(),
        )
        .await;
    }
}

pub async fn birthday(ctx: &Context, command: &CommandInteraction) -> CommandResult {
    if let Some(subcommand) = command.data.options.get(0) {
        let guild = command
            .guild_id
            .ok_or(anyhow!("Unable to get guild where command was sent"))?;
        match subcommand.name.as_str() {
            "set" => {
                if let CommandDataOptionValue::SubCommand(subcommand_args) = &subcommand.value {
                    if let Some(date_arg) = subcommand_args.first() {
                        if let CommandDataOptionValue::String(date_str) = &date_arg.value {
                            let privacy = subcommand_args.iter().find_map(|x| {
                                if let CommandDataOptionValue::String(arg_str) = &x.value {
                                    BirthdayPrivacy::from_str(arg_str).ok()
                                } else {
                                    None
                                }
                            });

                            let response =
                                set_birthday(ctx, guild, command.user.id, date_str, privacy)
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
                Err(anyhow!("No date argument passed"))
            }
            "clear" => {
                let response = clear_birthday(ctx, guild, command.user.id).await?;

                command
                    .edit_response(&ctx, EditInteractionResponse::new().content(response))
                    .await?;

                Ok(())
            }
            "check" => {
                let users: Vec<UserId> = if let CommandDataOptionValue::SubCommand(
                    subcommand_args,
                ) = &subcommand.value
                {
                    subcommand_args
                        .iter()
                        .filter_map(|x| {
                            if let CommandDataOptionValue::User(user) = &x.value {
                                Some(user.clone())
                            } else {
                                None
                            }
                        })
                        .collect()
                } else {
                    vec![]
                };

                let response = if users.is_empty() {
                    todays_birthdays(ctx, guild).await?
                } else {
                    user_birthdays(ctx, guild, users).await?
                };

                command
                    .edit_response(&ctx, EditInteractionResponse::new().content(response))
                    .await?;

                Ok(())
            }
            _ => Err(anyhow!("Unknown option {}", subcommand.name)),
        }
    } else {
        Err(anyhow!("Please provide a valid subcommand"))
    }
}
