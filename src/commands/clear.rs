use serenity::framework::standard::{macros::command, CommandResult};
use serenity::model::interactions::application_command::{ApplicationCommandInteraction, ResolvedTarget};
use serenity::model::prelude::*;
use serenity::prelude::*;

#[command]
#[required_permissions("ADMINISTRATOR")]
pub async fn clear_from(ctx: &Context, msg: &Message) -> CommandResult {
    if let Some(ref referenced_msg) = msg.referenced_message {
        clear_from_internal(ctx, &referenced_msg).await
    } else {
        Err("Message not a reply to starting message".into())
    }
}

async fn clear_from_internal(ctx: &Context, msg: &Message) -> CommandResult {
    let mut messages = msg.channel_id.messages(&ctx.http, |retriever| {
        retriever.after(msg.id).limit(100)
    }).await?;
    messages.insert(0, msg.to_owned());

    msg.channel_id.delete_messages(&ctx.http, messages).await?;

    Ok(())
}

pub async fn clear_from_slashcommand(ctx: &Context, command: &ApplicationCommandInteraction) -> CommandResult {
    if let Some(ResolvedTarget::Message(ref referenced_msg)) = command.data.target() {
        clear_from_internal(ctx, &referenced_msg).await?;

        command.create_interaction_response(&ctx.http, |r| {
            r.kind(InteractionResponseType::ChannelMessageWithSource)
            .interaction_response_data(|d| {
                d.content("Messages deleted.").ephemeral(true)
            })
        }).await?;

        Ok(())
    } else {
        Err("Message not a reply to starting message".into())
    }
}
