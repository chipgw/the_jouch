use std::io::Cursor;
use image::{GenericImage,GenericImageView,DynamicImage,ImageResult,error,imageops::FilterType,Pixel,ImageOutputFormat};
use serenity::builder::CreateEmbed;
use serenity::framework::standard::{macros::command, Args, CommandResult};
use serenity::model::interactions::application_command::{ApplicationCommandInteraction, ApplicationCommandInteractionDataOptionValue};
use serenity::model::prelude::*;
use serenity::prelude::*;
use crate::db::{Db, UserKey};
use super::autonick::check_nick_user;
use super::birthday::is_birthday_today;

const SIT_ONE: (u32, u32) = (385, 64);
const SIT_WITH: (u32, u32, u32, u32) = (240, 64, 580, 64);
const HAT_OFFSET: (u32, u32) = (48, 64);

async fn get_face(user: &User) -> CommandResult<DynamicImage> {
    let buffer = reqwest::get(user.face()).await?.bytes().await?;

    Ok(if let Some(img) = webp::Decoder::new(&buffer).decode() {
        img.to_image()
    } else {
        image::load_from_memory(&buffer)?
    }.resize(128, 128, FilterType::CatmullRom))
}

// basically stolen from copy_from, but with blending the source & target pixels rather than replacement & limiting to a circle.
fn blend(target: &mut DynamicImage, source: &DynamicImage, x: u32, y: u32, circle: bool) -> ImageResult<()> {
    // Do bounds checking here so we can use the non-bounds-checking
    // functions to copy pixels.
    if target.width() < source.width() + x || target.height() < source.height() + y {
        return Err(error::ImageError::Parameter(error::ParameterError::from_kind(
            error::ParameterErrorKind::DimensionMismatch,
        )));
    }

    let source_ctr = source.height() as f32 / 2.0;
    let r_squared = source_ctr * source_ctr;

    for k in 0..source.height() {
        for i in 0..source.width() {
            let coord = (i as f32 - source_ctr, k as f32 - source_ctr);
            // Limit to circle since that's how Discord shows profile pictures
            // TODO - could try to do some kind of anti-aliasing to this...
            if !circle || (coord.0 * coord.0 + coord.1 * coord.1) < r_squared {
                let mut out_pixel = target.get_pixel(i + x, k + y);
                out_pixel.blend(&source.get_pixel(i, k));
                target.put_pixel(i + x, k + y, out_pixel);
            }
        }
    }
    Ok(())
}

pub fn increment_sit_counter(db: &mut Db, user: &User, guild: GuildId) -> CommandResult {
    let key = UserKey {
        user: user.id,
        guild,
    };

    db.update(&key, |data| { 
        data.sit_count = Some(data.sit_count.unwrap_or_default() + 1);
    })?;

    Ok(())
}

async fn sit_check(ctx: &Context, user: &User, guild: Option<GuildId>, users: &Vec<User>) -> CommandResult<CreateEmbed> {
    let data = ctx.data.read().await;
    let db = data.get::<Db>().ok_or("Unable to get database")?;

    let mut sit_data: Vec<(String, u64)> = Vec::new();

    let title = if let Some(guild) = guild {
        if users.is_empty() {
            let users = db.get_users(guild)?;

            for user_key in &users {
                if let Some(count) = db.read(user_key, |data|{ data.sit_count })?.unwrap_or_default() {
                    let user = user_key.user.to_user(ctx).await?;
                    let name = user.nick_in(ctx, guild).await.unwrap_or(user.name);
                    sit_data.push((name, count));
                }
            };
            sit_data.sort_unstable_by(|a, b|{ b.1.cmp(&a.1) });
            sit_data.truncate(10);

            "Sit Leaderboard"
        } else {
            for user in users {
                let key = UserKey {
                    user: user.id,
                    guild,
                };
                let name = user.nick_in(ctx, guild).await.unwrap_or(user.name.clone());
                let count = db.read(&key, |data| {
                    data.sit_count.unwrap_or_default()
                })?;
                sit_data.push((name, count.unwrap_or_default()));
            }
            "Sit Data For Users"
        }
    } else {
        let guilds = db.get_guilds()?;

        for guild in guilds {
            let user_key = UserKey { user: user.id, guild };
            if let Some(count) = db.read(&user_key, |data|{ data.sit_count })?.unwrap_or_default() {
                let name = if let Some(name) = guild.name(&ctx.cache) {
                    name
                } else {
                    guild.to_partial_guild(&ctx.http).await?.name
                };
                sit_data.push((name, count));
            }
        }

        "Sit data in all servers"
    };

    let mut embed = CreateEmbed::default();

    for (user, count) in sit_data {
        embed.field(user, format!("Times on The Jouch: {}", count), false);
    }
    embed.title(title);

    Ok(embed)
}

async fn sit_internal(ctx: &Context, user: &User, guild: Option<GuildId>, with: Option<&User>) -> CommandResult<Vec<u8>> {
    let base_image_path = if with.is_some() {
        "assets/jouch-0002.png"
    } else {
        "assets/jouch-0001.png"
    };

    let mut base_image = image::io::Reader::open(base_image_path)?.decode()?;

    let party_hat_image = image::io::Reader::open("assets/party-hat-0001.png")?.decode()?;

    let user_avatar = get_face(user).await?;

    if let Some(other) = with {
        let with_avatar = get_face(other).await?;

        blend(&mut base_image, &user_avatar, SIT_WITH.0, SIT_WITH.1, true)?;
        blend(&mut base_image, &with_avatar, SIT_WITH.2, SIT_WITH.3, true)?;

        if let Some(guild) = guild {
            if is_birthday_today(ctx, UserKey { user: user.id, guild }).await? {
                blend(&mut base_image, &party_hat_image, SIT_WITH.0 + HAT_OFFSET.0, SIT_WITH.1 - HAT_OFFSET.1, false)?;
            }
            if is_birthday_today(ctx, UserKey { user: other.id, guild }).await? {
                blend(&mut base_image, &party_hat_image, SIT_WITH.2 + HAT_OFFSET.0, SIT_WITH.3 - HAT_OFFSET.1, false)?;
            }
        }
    } else {
        blend(&mut base_image, &user_avatar, SIT_ONE.0, SIT_ONE.1, true)?;
        
        if let Some(guild) = guild {
            if is_birthday_today(ctx, UserKey { user: user.id, guild }).await? {
                blend(&mut base_image, &party_hat_image, SIT_ONE.0 + HAT_OFFSET.0, SIT_ONE.1 - HAT_OFFSET.1, false)?;
            }
        }
    }

    let mut image_bytes: Vec<u8> = vec![];
    base_image.write_to(&mut Cursor::new(&mut image_bytes), ImageOutputFormat::Png)?;

    if let Some(guild) = guild {
        let mut data = ctx.data.write().await;
        let db = data.get_mut::<Db>().ok_or("Unable to get database")?;
        increment_sit_counter(db, user, guild)?;
        let _ = check_nick_user(ctx, &UserKey { user: user.id, guild }, db).await;
        if let Some(user) = with {
            increment_sit_counter(db, user, guild)?;
            let _ = check_nick_user(ctx, &UserKey { user: user.id, guild }, db).await;
        }
    }

    Ok(image_bytes)
}

#[command]
pub async fn sit(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    let typing = msg.channel_id.start_typing(&ctx.http)?;

    if args.is_empty() {
        let image_bytes = sit_internal(ctx, &msg.author, msg.guild_id, None).await?;
        let files = vec![(&*image_bytes, "jouch.png")];

        msg.channel_id.send_files(&ctx.http, files, |m|{
            m.reference_message(msg)
        }).await?;
    } else {
        let arg = args.single::<String>()?;
        match arg.to_lowercase().as_str() {
            "with" => {
                if msg.mentions.len() != 1 {
                    return Err(if msg.mentions.is_empty() {
                        "No one to sit with!"
                    } else {
                        "Can only `sit with` one person!"
                    }.into())
                } else {
                    let image_bytes = sit_internal(ctx, &msg.author, msg.guild_id, Some(&msg.mentions[0])).await?;
                    let files = vec![(&*image_bytes, "jouch.png")];

                    msg.channel_id.send_files(&ctx.http, files, |m|{
                        m.reference_message(msg)
                    }).await?;
                }
            },
            "count" | "check" => {
                let embed = sit_check(ctx, &msg.author, msg.guild_id, &msg.mentions).await?;

                msg.channel_id.send_message(&ctx.http, |m| {
                    m.set_embed(embed);

                    m.reference_message(msg)
                }).await?;
            }
            _ => {
                return Err(format!("Unknown option {}", arg).into())
            }
        }
    };

    let _ignore = typing.stop();

    Ok(())
}

pub async fn sit_slashcommand(ctx: &Context, command: &ApplicationCommandInteraction) -> CommandResult {
    if let Some(subcommand) = command.data.options.first() {
        match subcommand.name.as_str() {
            "with" => {
                if let Some(user_arg) = subcommand.options.first() {
                    if let Some(ApplicationCommandInteractionDataOptionValue::User(user, _)) = &user_arg.resolved {
                        let image_bytes = sit_internal(ctx, &command.user, command.guild_id, Some(user)).await?;

                        let files = vec![(&*image_bytes, "jouch.png")];

                        // command.channel_id.send_files(&ctx.http, files, |m|{ m }).await?;
                        command.create_followup_message(&ctx.http, |m| {
                            m.add_files(files)
                        }).await?;

                        return Ok(())
                    }
                }
                Err("Please provide a valid user".into())
            },
            "solo" => {
                let image_bytes = sit_internal(ctx, &command.user, command.guild_id, None).await?;
                let file = (&*image_bytes, "jouch.png");

                command.create_followup_message(&ctx.http, |m|{
                    m.add_file(file)
                }).await?;

                return Ok(())
            },
            "count" | "check" => {
                let mut users = Vec::new();
                for user_arg in &subcommand.options {
                    if let Some(ApplicationCommandInteractionDataOptionValue::User(user, _)) = &user_arg.resolved {
                        users.push(user.to_owned());
                    }
                }

                let embed = sit_check(ctx, &command.user, command.guild_id, &users).await?;

                command.edit_original_interaction_response(&ctx.http, |r| {
                    r.add_embed(embed)
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
