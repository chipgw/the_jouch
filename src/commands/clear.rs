
use serenity::model::application::interaction::application_command::{ApplicationCommandInteraction, ResolvedTarget};
use serenity::prelude::*;
use anyhow::anyhow;

pub async fn clear_from(ctx: &Context, command: &ApplicationCommandInteraction) -> crate::CommandResult<()> {
    if let Some(ResolvedTarget::Message(ref msg)) = command.data.target() {
        let mut messages = msg.channel_id.messages(&ctx.http, |retriever| {
            retriever.after(msg.id).limit(100)
        }).await?;
        messages.insert(0, *msg.clone());

        msg.channel_id.delete_messages(&ctx.http, messages).await?;

        Ok(())
    } else {
        Err(anyhow!("Message not a reply to starting message"))
    }
}
