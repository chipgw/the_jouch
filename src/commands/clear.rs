use anyhow::anyhow;
use serenity::all::{CommandInteraction, Context, GetMessages, ResolvedTarget};

pub async fn clear_from(ctx: &Context, command: &CommandInteraction) -> crate::CommandResult<()> {
    if let Some(ResolvedTarget::Message(msg)) = command.data.target() {
        let mut messages = msg
            .channel_id
            .messages(&ctx.http, GetMessages::new().after(msg.id).limit(100))
            .await?;
        messages.insert(0, msg.clone());

        msg.channel_id.delete_messages(&ctx.http, messages).await?;

        Ok(())
    } else {
        Err(anyhow!("Message not a reply to starting message"))
    }
}
