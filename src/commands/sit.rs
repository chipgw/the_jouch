use image::{GenericImage,GenericImageView,DynamicImage,ImageResult,error,imageops::FilterType,Pixel,ImageOutputFormat};
use serenity::framework::standard::{macros::command, Args, CommandResult};
use serenity::model::prelude::*;
use serenity::prelude::*;
use crate::db::{Db, UserKey};

const SIT_ONE: (u32, u32) = (385, 64);
const SIT_WITH: (u32, u32, u32, u32) = (240, 64, 580, 64);

async fn get_face(user: &User) -> CommandResult<DynamicImage> {
    let buffer = reqwest::get(user.face()).await?.bytes().await?;

    Ok(if let Some(img) = webp::Decoder::new(&buffer).decode() {
        img.to_image()
    } else {
        image::load_from_memory(&buffer)?
    }.resize(128, 128, FilterType::CatmullRom))
}

// basically stolen from copy_from, but with blending the source & target pixels rather than replacement & limiting to a circle.
fn blend_circle(target: &mut DynamicImage, source: &DynamicImage, x: u32, y: u32) -> ImageResult<()> {
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
            if (coord.0 * coord.0 + coord.1 * coord.1) < r_squared {
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

async fn sit_check(ctx: &Context, msg: &Message) -> CommandResult {
    let guild = msg.guild_id.ok_or("Unable to get guild where command was sent")?;

    let data = ctx.data.read().await;
    let db = data.get::<Db>().ok_or("Unable to get database")?;

    let mut sit_data: Vec<(User, u64)> = Vec::new();

    let title = if msg.mentions.is_empty() {
        let users = db.get_users(guild)?;

        for user_key in &users {
            if let Some(count) = db.read(user_key, |data|{ data.sit_count })?.unwrap_or_default() {
                sit_data.push((user_key.user.to_user(ctx).await?, count));
            }
        };
        sit_data.sort_unstable_by(|a, b|{ b.1.cmp(&a.1) });
        sit_data.truncate(10);

        "Sit Leaderboard"
    } else {
        for user in &msg.mentions {
            let key = UserKey {
                user: user.id,
                guild,
            };
            db.read(&key, |data| {
                sit_data.push((user.clone(), data.sit_count.unwrap_or_default()));
            })?;
        }
        "Sit Data For Users"
    };

    msg.channel_id.send_message(&ctx.http, |m| {
        m.embed(|e| {
            for (user, count) in sit_data {
                e.field(user.name, format!("Times on The Jouch: {}", count), false);
            }
            e.title(title)
        });

        m
    }).await?;

    Ok(())
}

async fn sit_internal(ctx: &Context, msg: &Message, with: Option<&User>) -> CommandResult {
    let typing = msg.channel_id.start_typing(&ctx.http)?;

    let base_image_path = if with.is_some() {
        "assets/jouch-0002.png"
    } else {
        "assets/jouch-0001.png"
    };

    let mut base_image = image::io::Reader::open(base_image_path)?.decode()?;

    let author_avatar = get_face(&msg.author).await?;

    if let Some(user) = with {
        let with_avatar = get_face(user).await?;

        blend_circle(&mut base_image, &author_avatar, SIT_WITH.0, SIT_WITH.1)?;
        blend_circle(&mut base_image, &with_avatar, SIT_WITH.2, SIT_WITH.3)?;
    } else {
        blend_circle(&mut base_image, &author_avatar, SIT_ONE.0, SIT_ONE.1)?;
    }

    let mut image_bytes: Vec<u8> = vec![];
    base_image.write_to(&mut image_bytes, ImageOutputFormat::Png)?;
    let files = vec![(&*image_bytes, "jouch.png")];

    msg.channel_id.send_files(&ctx.http, files, |m|{
        m.reference_message(msg)
    }).await?;

    typing.stop();

    let mut data = ctx.data.write().await;
    let db = data.get_mut::<Db>().ok_or("Unable to get database")?;
    let guild = msg.guild_id.ok_or("Unable to get guild where command was sent")?;
    increment_sit_counter(db, &msg.author, guild)?;
    if let Some(user) = with {
        increment_sit_counter(db, user, guild)?;
    }

    Ok(())
}

#[command]
pub async fn sit(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    if args.is_empty() {
        sit_internal(ctx, msg, None).await
    } else {
        let arg = args.single::<String>()?;
        match arg.to_lowercase().as_str() {
            "with" => {
                if msg.mentions.len() != 1 {
                    Err(if msg.mentions.is_empty() {
                        "No one to sit with!"
                    } else {
                        "Can only `sit with` one person!"
                    }.into())
                } else {
                    sit_internal(ctx, msg, Some(&msg.mentions[0])).await
                }
            },
            "count" | "check" => {
                sit_check(ctx, msg).await
            }
            _ => {
                Err(format!("Unknown option {}", arg).into())
            }
        }
    }
}
