use serenity::framework::standard::{macros::command, CommandResult};
use serenity::model::prelude::*;
use serenity::prelude::*;

#[command]
#[required_permissions("ADMINISTRATOR")]
pub async fn clear_from(ctx: &Context, msg: &Message) -> CommandResult {
    if let Some(ref referenced_msg) = msg.referenced_message {
        let mut messages = referenced_msg.channel_id.messages(&ctx.http, |retriever| {
            retriever.after(referenced_msg.id).limit(100)
        }).await?;
        messages.insert(0, *referenced_msg.to_owned());

        referenced_msg.channel_id.delete_messages(&ctx.http, messages).await?;

        Ok(())
    } else {
        Err("Message not a reply to starting message".into())
    }
}
