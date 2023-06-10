use std::io::Cursor;
use image::{GenericImage,GenericImageView,DynamicImage,ImageResult,error,imageops::FilterType,Pixel,ImageOutputFormat};
use rand::{distributions::Standard, prelude::Distribution, self, Rng};
use serenity::builder::CreateEmbed;
use serenity::model::application::interaction::application_command::{ApplicationCommandInteraction, CommandDataOptionValue};
use serenity::model::prelude::*;
use serenity::prelude::*;
use serde::{Serialize, Deserialize};
use serenity::utils::MessageBuilder;
use crate::db::{Db, UserKey};
use crate::CommandResult;
use super::autonick::check_nick_user;
use super::birthday::is_birthday_today;

const SIT_ONE: (u32, u32) = (385, 64);
const SIT_WITH: (u32, u32, u32, u32) = (240, 64, 580, 64);
const HAT_OFFSET: (u32, u32) = (48, 64);

#[derive(Default, Clone, Copy, Debug, Serialize, Deserialize)]
pub enum JouchOrientation {
    #[default] Normal,
    UpsideDown,
    RotatedLeft,
    RotatedRight,
}

impl Distribution<JouchOrientation> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> JouchOrientation {
        match rng.gen_range(0..=3) {
            1 => JouchOrientation::UpsideDown,
            2 => JouchOrientation::RotatedLeft,
            3 => JouchOrientation::RotatedRight,
            _ => JouchOrientation::Normal,
        }
    }
}

async fn get_face(ctx: &Context, user: &User, guild: Option<GuildId>) -> CommandResult<DynamicImage> {
    let buffer = reqwest::get(if let Some(guild) = guild {
        guild.member(ctx, user.id).await?.face()
    } else {
        user.face()
    }).await?.bytes().await?;

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

pub fn increment_flip_counter(db: &mut Db, user: &User, guild: GuildId) -> CommandResult {
    let key = UserKey {
        user: user.id,
        guild,
    };

    db.update(&key, |data| { 
        data.flip_count = Some(data.flip_count.unwrap_or_default() + 1);
    })?;

    Ok(())
}

async fn sit_check(ctx: &Context, user: &User, guild: Option<GuildId>, users: &Vec<User>) -> CommandResult<CreateEmbed> {
    let data = ctx.data.read().await;
    let db = data.get::<Db>().ok_or("Unable to get database")?;

    // Name, sit_count, flip_count
    let mut sit_data: Vec<(String, Option<u64>, Option<u64>)> = Vec::new();

    let title = if let Some(guild) = guild {
        if users.is_empty() {
            let users = db.get_users(guild)?;

            for user_key in &users {
                let (sit_count, flip_count) = db.read(user_key, |data|{ (data.sit_count, data.flip_count) })?.unwrap_or_default();
                let user = user_key.user.to_user(ctx).await?;
                let name = user.nick_in(ctx, guild).await.unwrap_or(user.name);
                sit_data.push((name, sit_count, flip_count));
            };
            // TODO - add option to sort by flips instead of sits, or include both in sorting somehow.
            // (will probably separate the leaderboard/check functionality into its own command anyway, so will take care of it then)
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
                let (sit_count, flip_count) = db.read(&key, |data| {
                    (data.sit_count, data.flip_count)
                })?.unwrap_or_default();
                sit_data.push((name, sit_count, flip_count));
            }
            "Sit Data For Users"
        }
    } else {
        let guilds = db.get_guilds()?;

        for guild in guilds {
            let user_key = UserKey { user: user.id, guild };
            let (sit_count, flip_count) = db.read(&user_key, |data|{ (data.sit_count, data.flip_count) })?.unwrap_or_default();
                let name = if let Some(name) = guild.name(&ctx.cache) {
                    name
                } else {
                    guild.to_partial_guild(&ctx.http).await?.name
                };
                sit_data.push((name, sit_count, flip_count));
        }

        "Sit data in all servers"
    };

    let mut embed = CreateEmbed::default();

    for (user, sit_count, flip_count) in sit_data {
        if sit_count.is_none() && flip_count.is_none() {
            // We have neither sit nore flip data, so don't display at all.
            continue;
        }
        let mut msg = MessageBuilder::new();
        if let Some(sit_count) = sit_count {
            msg.push("Times on The Jouch: ").push_line(sit_count);
        }
        if let Some(flip_count) = flip_count {
            msg.push("Flips of The Jouch: ").push_line(flip_count);
        }
        embed.field(user, msg.build(), false);
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

    let user_avatar = get_face(ctx, user, guild).await?;

    if let Some(other) = with {
        let with_avatar = get_face(ctx, other, guild).await?;

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

    // rotate output image based on current orientation in guild
    let orientation = if let Some(guild) = guild {
        let data = ctx.data.read().await;
        let db = data.get::<Db>().ok_or("Unable to get database")?;
        db.read_guild(guild, |data|{
            data.jouch_orientation
        }).unwrap_or_default().unwrap_or_default() // Not critical; don't raise an error if it isn't available.
    } else {
        Default::default()
    };
    base_image = match orientation {
        JouchOrientation::Normal => base_image,
        JouchOrientation::UpsideDown => base_image.rotate180(),
        JouchOrientation::RotatedLeft => base_image.rotate270(),
        JouchOrientation::RotatedRight => base_image.rotate90(),
    };

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

pub async fn sit(ctx: &Context, command: &ApplicationCommandInteraction) -> CommandResult {
    if let Some(subcommand) = command.data.options.first() {
        match subcommand.name.as_str() {
            "with" => {
                if let Some(user_arg) = subcommand.options.first() {
                    if let Some(CommandDataOptionValue::User(user, _)) = &user_arg.resolved {
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
                    if let Some(CommandDataOptionValue::User(user, _)) = &user_arg.resolved {
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

pub async fn flip(ctx: &Context, command: &ApplicationCommandInteraction) -> CommandResult {
    // TODO - weight this to prefer orientations other than current (and potentially to add rare "orientations" in the future)
    let new_orientation: JouchOrientation = rand::random();

    if let Some(guild) = command.guild_id {
        let mut data = ctx.data.write().await;
        let db = data.get_mut::<Db>().ok_or("Unable to get database")?;
        db.update_guild(guild, |data|{
            data.jouch_orientation = new_orientation;
        })?;
        increment_flip_counter(db, &command.user, guild)?;
    }

    let emote = match new_orientation {
        JouchOrientation::Normal => "<:jouchup1:1117080763565879397><:jouchup2:1117080764572520449>",
        JouchOrientation::UpsideDown => "<:jouchdn1:1117080756309721139><:jouchdn2:1117080758612410401> ",
        JouchOrientation::RotatedLeft => "<:jouchl1:1117080760185270366>\n<:jouchl2:1117080761615519814>",
        JouchOrientation::RotatedRight => "<:jouchr1:1117079201321861150>\n<:jouchr2:1117079202890530906>",
    }.to_owned();

    command.edit_original_interaction_response(&ctx.http, |r| {
        r.content(emote + "︵╰(°□°╰)")
    }).await?;

    Ok(())
}
