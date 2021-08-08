use serenity::framework::standard::{macros::command, Args, CommandResult};
use serenity::model::prelude::*;
use serenity::prelude::*;
use serenity::utils::MessageBuilder;
use chrono::prelude::*;
use crate::db::{Db, UserKey};

const DATE_OPTIONS: &[&str] = &[
    "%F",           // e.g. 1990-01-01
];
const TIME_OPTIONS: &[&str] = &[
    "T%H:%M%:z",    // e.g. T12:30
    "T%H:%M%#z",    // e.g. T12:30
    "",             // no time provided
];
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

pub async fn set_birthday(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult<String> {
    let date = parse_date(args.current().ok_or("No date argument passed")?)?;
    args.advance();
    
    let mut data = ctx.data.write().await;
    let db = data.get_mut::<Db>().ok_or("Unable to get database")?;

    let key = UserKey {
        user: msg.author.id, 
        guild: msg.guild_id.ok_or("Unable to get guild where command was sent")?,
    };

    db.update(key, |data| { data.birthday = Some(date) })?;

    Ok(MessageBuilder::new()
        .push("Set birthday to ")
        .push_bold_safe(date)
        .build())
}

#[command]
pub async fn birthday(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    let subcommand = if args.is_empty() {
        // Assume that when !birthday is called with no arguments that the user wants to run the check subcommand
        "check".into()
    } else {
        args.single::<String>()?.to_lowercase()
    };
    
    let response = match subcommand.as_str() {
        "set" => {
            set_birthday(ctx, msg, args).await?
        },
        "check" => {
            // TODO - is it someone's birthday today?
            "No birthdays today.".into()
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
