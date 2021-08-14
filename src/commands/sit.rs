use image::{GenericImage,GenericImageView,DynamicImage,ImageResult,error,imageops::FilterType,Pixel,ImageOutputFormat};
use serenity::framework::standard::{macros::command, Args, CommandResult};
use serenity::model::prelude::*;
use serenity::prelude::*;

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
fn blend_circle(target: &mut DynamicImage, source: &DynamicImage, x: u32, y: u32) -> ImageResult<()>
{
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

#[command]
pub async fn sit(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    let with = if args.is_empty() {
        false
    } else {
        let arg = args.single::<String>()?;
        if arg.to_lowercase() == "with" {
            if msg.mentions.len() != 1 {
                return Err(if msg.mentions.is_empty() {
                    "No one to sit with!"
                } else {
                    "Can only `sit with` one person!"
                }.into());
            }
            true
        } else {
            return Err(format!("Unknown option {}", arg).into());
        }
    };

    let typing = msg.channel_id.start_typing(&ctx.http)?;

    let base_image_path = if with {
        "assets/jouch-0002.png"
    } else {
        "assets/jouch-0001.png"
    };

    let mut base_image = image::io::Reader::open(base_image_path)?.decode()?;

    let author_avatar = get_face(&msg.author).await?;

    if with {
        let with_avatar = get_face(&msg.mentions[0]).await?;

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

    Ok(())
}
