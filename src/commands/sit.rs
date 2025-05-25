use super::autonick::check_nick_user_key;
use super::birthday::is_birthday_today;
use crate::db::{Db, UserKey};
use crate::{CommandResult, ShuttleItemsContainer};
use anyhow::anyhow;
use enum_utils::TryFromRepr;
use image::ImageFormat;
use image::{
    error, imageops::FilterType, DynamicImage, GenericImage, GenericImageView, ImageResult, Pixel,
};
use rand::{self, distr::StandardUniform, prelude::Distribution, Rng};
use serde::{Deserialize, Serialize};
use serenity::all::{
    CommandInteraction, Context, CreateAttachment, CreateEmbed, CreateInteractionResponseFollowup,
    EditInteractionResponse, GuildId, MessageBuilder, ResolvedTarget, ResolvedValue, User, UserId,
};
use std::convert::TryInto;
use std::io::Cursor;
use tracing::debug;

const SIT_ONE: (u32, u32) = (385, 64);
const SIT_WITH: (u32, u32, u32, u32) = (240, 64, 580, 64);
const HAT_OFFSET: (u32, u32) = (48, 64);

#[derive(Default, Clone, Copy, Debug, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "jouch_orientation")]
pub enum JouchOrientation {
    #[default]
    Normal,
    UpsideDown,
    RotatedLeft,
    RotatedRight,
}

impl JouchOrientation {
    pub fn to_emotes(&self) -> &str {
        match self {
            JouchOrientation::Normal => {
                "<:jouchup1:1117080763565879397><:jouchup2:1117080764572520449>"
            }
            JouchOrientation::UpsideDown => {
                "<:jouchdn1:1117080756309721139><:jouchdn2:1117080758612410401> "
            }
            JouchOrientation::RotatedLeft => {
                "<:jouchl1:1117080760185270366>\n<:jouchl2:1117080761615519814>"
            }
            JouchOrientation::RotatedRight => {
                "<:jouchr1:1117079201321861150>\n<:jouchr2:1117079202890530906>"
            }
        }
    }
}

#[derive(TryFromRepr)]
#[repr(u8)]
enum RankSortBy {
    Sits,
    Flips,
}

impl Distribution<JouchOrientation> for StandardUniform {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> JouchOrientation {
        match rng.random_range(0..=3) {
            1 => JouchOrientation::UpsideDown,
            2 => JouchOrientation::RotatedLeft,
            3 => JouchOrientation::RotatedRight,
            _ => JouchOrientation::Normal,
        }
    }
}

async fn get_face(
    ctx: &Context,
    user: &User,
    guild: Option<GuildId>,
) -> CommandResult<DynamicImage> {
    let buffer = reqwest::get(if let Some(guild) = guild {
        guild.member(ctx, user.id).await?.face()
    } else {
        user.face()
    })
    .await?
    .bytes()
    .await?;

    Ok(if let Some(img) = webp::Decoder::new(&buffer).decode() {
        img.to_image()
    } else {
        image::load_from_memory(&buffer)?
    }
    .resize(128, 128, FilterType::CatmullRom))
}

// basically stolen from copy_from, but with blending the source & target pixels rather than replacement & limiting to a circle.
fn blend(
    target: &mut DynamicImage,
    source: &DynamicImage,
    x: u32,
    y: u32,
    circle: bool,
) -> ImageResult<()> {
    // Do bounds checking here so we can use the non-bounds-checking
    // functions to copy pixels.
    if target.width() < source.width() + x || target.height() < source.height() + y {
        return Err(error::ImageError::Parameter(
            error::ParameterError::from_kind(error::ParameterErrorKind::DimensionMismatch),
        ));
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

pub async fn increment_sit_counter(db: &mut Db, user: &User, guild: GuildId) -> CommandResult {
    let key = UserKey {
        user: user.id.into(),
        guild: guild.into(),
    };
    db.increment(&key, "sit_count").await?;

    Ok(())
}

pub async fn increment_flip_counter(db: &mut Db, user: &User, guild: GuildId) -> CommandResult {
    let key = UserKey {
        user: user.id.into(),
        guild: guild.into(),
    };
    db.increment(&key, "flip_count").await?;

    Ok(())
}

struct RankingData {
    name: String,
    sit_count: i32,
    flip_count: i32,
}

async fn sit_check(
    ctx: &Context,
    user: &User,
    guild: Option<GuildId>,
    users: &Vec<User>,
    sort_by: RankSortBy,
) -> CommandResult<CreateEmbed> {
    let data = ctx.data.read().await;
    let db = data.get::<Db>().ok_or(anyhow!("Unable to get database"))?;

    // Name, sit_count, flip_count
    let mut sit_data: Vec<RankingData> = Vec::new();

    let title = if let Some(guild) = guild {
        if users.is_empty() {
            let users = db
                .get_users(
                    guild,
                    match sort_by {
                        RankSortBy::Sits => "ORDER BY sit_count DESC, flip_count DESC LIMIT 10",
                        RankSortBy::Flips => "ORDER BY flip_count DESC, sit_count DESC LIMIT 10",
                    },
                )
                .await?;

            for user_data in users {
                let user = Into::<UserId>::into(user_data.id.user as u64)
                    .to_user(ctx)
                    .await?;
                let name = user.nick_in(ctx, guild).await.unwrap_or(user.name);
                sit_data.push(RankingData {
                    name,
                    sit_count: user_data.sit_count,
                    flip_count: user_data.flip_count,
                });
            }

            "Sit Leaderboard"
        } else {
            // TODO - there should be a more efficient way to do this, i.e. with a single query
            for user in users {
                let key = UserKey {
                    user: user.id.into(),
                    guild: guild.into(),
                };
                let name = user.nick_in(ctx, guild).await.unwrap_or(user.name.clone());
                let (sit_count, flip_count) = db
                    .read(&key)
                    .await?
                    .map(|data| (data.sit_count, data.flip_count))
                    .unwrap_or_default();
                sit_data.push(RankingData {
                    name,
                    sit_count,
                    flip_count,
                });
            }
            "Sit Data For Users"
        }
    } else {
        // TODO - there should be a more efficient way to do this, i.e. with a single query
        // get any guild that this user has data in.
        let guilds = db.get_user_guilds(Some(user.id)).await?;

        debug!(
            "Checking user {} sit data in guilds: {:#?}",
            user.id, guilds
        );

        for guild in guilds {
            let user_key = UserKey {
                user: user.id.into(),
                guild: guild.into(),
            };
            let (sit_count, flip_count) = db
                .read(&user_key)
                .await?
                .map(|data| (data.sit_count, data.flip_count))
                .unwrap_or_default();
            let name = if let Some(name) = guild.name(&ctx.cache) {
                name
            } else {
                guild.to_partial_guild(&ctx.http).await?.name
            };
            sit_data.push(RankingData {
                name,
                sit_count,
                flip_count,
            });
        }

        "Sit data in all servers"
    };

    let mut embed = CreateEmbed::default();

    for data in sit_data {
        if data.sit_count <= 0 && data.flip_count <= 0 {
            // We have neither sit nore flip data, so don't display at all.
            continue;
        }
        let mut msg = MessageBuilder::new();
        if data.sit_count > 0 {
            msg.push("Times on The Jouch: ")
                .push_line(data.sit_count.to_string());
        }
        if data.flip_count > 0 {
            msg.push("Flips of The Jouch: ")
                .push_line(data.flip_count.to_string());
        }
        embed = embed.field(data.name, msg.build(), false);
    }
    embed = embed.title(title);

    Ok(embed)
}

async fn sit_internal(
    ctx: &Context,
    user: &User,
    guild: Option<GuildId>,
    with: Option<&User>,
) -> CommandResult<Vec<u8>> {
    let assets_dir = {
        ctx.data
            .read()
            .await
            .get::<ShuttleItemsContainer>()
            .ok_or(anyhow!("Unable to get config!"))?
            .assets_dir
            .clone()
    };

    let base_image_path = if with.is_some() {
        assets_dir.join("jouch-0002.png")
    } else {
        assets_dir.join("jouch-0001.png")
    };

    let mut base_image = image::ImageReader::open(base_image_path)?.decode()?;

    let party_hat_image =
        image::ImageReader::open(assets_dir.join("party-hat-0001.png"))?.decode()?;

    let user_avatar = get_face(ctx, user, guild).await?;

    if let Some(other) = with {
        let with_avatar = get_face(ctx, other, guild).await?;

        blend(&mut base_image, &user_avatar, SIT_WITH.0, SIT_WITH.1, true)?;
        blend(&mut base_image, &with_avatar, SIT_WITH.2, SIT_WITH.3, true)?;

        if let Some(guild) = guild {
            if is_birthday_today(
                ctx,
                UserKey {
                    user: user.id.into(),
                    guild: guild.into(),
                },
            )
            .await?
            {
                blend(
                    &mut base_image,
                    &party_hat_image,
                    SIT_WITH.0 + HAT_OFFSET.0,
                    SIT_WITH.1 - HAT_OFFSET.1,
                    false,
                )?;
            }
            if is_birthday_today(
                ctx,
                UserKey {
                    user: user.id.into(),
                    guild: guild.into(),
                },
            )
            .await?
            {
                blend(
                    &mut base_image,
                    &party_hat_image,
                    SIT_WITH.2 + HAT_OFFSET.0,
                    SIT_WITH.3 - HAT_OFFSET.1,
                    false,
                )?;
            }
        }
    } else {
        blend(&mut base_image, &user_avatar, SIT_ONE.0, SIT_ONE.1, true)?;

        if let Some(guild) = guild {
            if is_birthday_today(
                ctx,
                UserKey {
                    user: user.id.into(),
                    guild: guild.into(),
                },
            )
            .await?
            {
                blend(
                    &mut base_image,
                    &party_hat_image,
                    SIT_ONE.0 + HAT_OFFSET.0,
                    SIT_ONE.1 - HAT_OFFSET.1,
                    false,
                )?;
            }
        }
    }

    // rotate output image based on current orientation in guild
    let orientation = if let Some(guild) = guild {
        let data = ctx.data.read().await;
        let db = data.get::<Db>().ok_or(anyhow!("Unable to get database"))?;
        // Not critical; don't raise an error if it isn't available.
        db.read_guild(guild)
            .await
            .unwrap_or_default()
            .map(|data| data.jouch_orientation)
            .unwrap_or_default()
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
    base_image.write_to(&mut Cursor::new(&mut image_bytes), ImageFormat::Png)?;

    if let Some(guild) = guild {
        let mut data = ctx.data.write().await;
        let db = data
            .get_mut::<Db>()
            .ok_or(anyhow!("Unable to get database"))?;
        increment_sit_counter(db, user, guild).await?;
        let _ = check_nick_user_key(
            ctx,
            &UserKey {
                user: user.id.into(),
                guild: guild.into(),
            },
            db,
        )
        .await;
        if let Some(user) = with {
            increment_sit_counter(db, user, guild).await?;
            let _ = check_nick_user_key(
                ctx,
                &UserKey {
                    user: user.id.into(),
                    guild: guild.into(),
                },
                db,
            )
            .await;
        }
    }

    Ok(image_bytes)
}

pub async fn sit(ctx: &Context, command: &CommandInteraction) -> CommandResult {
    if let Some(user_arg) = command.data.options().first() {
        if let ResolvedValue::User(user, _) = user_arg.value {
            let image_bytes =
                sit_internal(ctx, &command.user, command.guild_id, Some(user)).await?;

            let files = CreateAttachment::bytes(image_bytes, "jouch.png");

            // command.channel_id.send_files(&ctx.http, files, |m|{ m }).await?;
            command
                .create_followup(
                    &ctx,
                    CreateInteractionResponseFollowup::new().add_file(files),
                )
                .await?;

            Ok(())
        } else {
            Err(anyhow!("Couldn't find your friend! (Argument invalid)"))
        }
    } else {
        let image_bytes = sit_internal(ctx, &command.user, command.guild_id, None).await?;
        let file = CreateAttachment::bytes(image_bytes, "jouch.png");

        command
            .create_followup(
                &ctx.http,
                CreateInteractionResponseFollowup::new().add_file(file),
            )
            .await?;

        Ok(())
    }
}

pub async fn rank(ctx: &Context, command: &CommandInteraction) -> CommandResult {
    let mut users = Vec::new();

    let mut sort_by = RankSortBy::Sits;

    for arg in &command.data.options() {
        if let ResolvedValue::User(user, _) = arg.value {
            users.push(user.to_owned());
        } else if let ResolvedValue::Integer(as_int) = arg.value {
            match arg.name {
                "sort" => {
                    sort_by = (as_int as u8)
                        .try_into()
                        .map_err(|_| anyhow!("Invalid sort value passed!"))?
                }
                _ => return Err(anyhow!("Unknown/unimplemented option {}", arg.name).into()),
            };
        }
    }

    let embed = sit_check(ctx, &command.user, command.guild_id, &users, sort_by).await?;

    command
        .edit_response(&ctx, EditInteractionResponse::new().add_embed(embed))
        .await?;

    Ok(())
}

pub async fn flip(ctx: &Context, command: &CommandInteraction) -> CommandResult {
    // TODO - weight this to prefer orientations other than current (and potentially to add rare "orientations" in the future)
    let new_orientation: JouchOrientation = rand::random();

    if let Some(guild) = command.guild_id {
        let mut data = ctx.data.write().await;
        let db = data
            .get_mut::<Db>()
            .ok_or(anyhow!("Unable to get database"))?;
        increment_flip_counter(db, &command.user, guild).await?;
        db.update_guild(guild, "jouch_orientation", new_orientation)
            .await?; 
        
        let _ = check_nick_user_key(
            ctx,
            &UserKey {
                user: command.user.id.into(),
                guild: guild.into(),
            },
            db,
        )
        .await;
    }

    let emote = new_orientation.to_emotes().to_owned();

    if let Some(ResolvedTarget::Message(ref msg)) = command.data.target() {
        let mut builder = MessageBuilder::new();
        builder.push(emote);
        builder.push("︵╰(°□°╰) ← ");
        builder.mention(&command.user);
        msg.reply(ctx, builder.build()).await?;
        command.delete_response(&ctx).await?;
    } else {
        command
            .edit_response(
                &ctx,
                EditInteractionResponse::new().content(emote + "︵╰(°□°╰)"),
            )
            .await?;
    }

    Ok(())
}

pub async fn rectify(ctx: &Context, command: &CommandInteraction) -> CommandResult {
    let new_orientation: JouchOrientation = JouchOrientation::Normal;

    if let Some(guild) = command.guild_id {
        let mut data = ctx.data.write().await;
        let db = data
            .get_mut::<Db>()
            .ok_or(anyhow!("Unable to get database"))?;
        db.update_guild(guild, "jouch_orientation", new_orientation)
            .await?;
    }

    let emote = new_orientation.to_emotes().to_owned();

    command
        .edit_response(
            &ctx,
            EditInteractionResponse::new().content(emote + "ノ( ˙ - ˙ ノ)"),
        )
        .await?;

    Ok(())
}
