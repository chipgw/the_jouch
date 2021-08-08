use serenity::framework::standard::{macros::command, CommandResult};
use serenity::http::Http;
use serenity::model::prelude::*;
use serenity::prelude::*;
use crate::config::Config;

#[command]
#[required_permissions("ADMINISTRATOR")]
pub async fn clear_from(ctx: &Context, msg: &Message) -> CommandResult {
    if let Some(ref referenced_msg) = msg.referenced_message {
        let data = ctx.data.read().await;
        let config = data.get::<Config>().ok_or("Unable to get config")?;

        let http = Http::new_with_token(&config.token);
        let mut messages = referenced_msg.channel_id.messages(&http, |retriever| {
            retriever.after(referenced_msg.id).limit(100)
        }).await?;
        messages.insert(0, *referenced_msg.to_owned());

        referenced_msg.channel_id.delete_messages(http, messages).await?;

        Ok(())
    } else {
        Err("Message not a reply to starting message".into())
    }
}