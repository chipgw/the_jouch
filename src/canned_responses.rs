use std::convert::TryFrom;
use serenity::{framework::standard::{CommandResult, macros::hook}, model::channel::{Message, ReactionType}, prelude::*};
use serde::{Deserialize, Serialize};

use crate::{config::Config, db::Db};

#[derive(Eq, PartialEq, Serialize, Deserialize, Clone, Debug)]
pub enum Response {
    Reply(String),
    Reaction(String),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ResponseData {
    pub triggers: Vec<String>,
    pub response: Response,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ResponseTable {
    map: Vec<ResponseData>,
}

impl ResponseTable {
    pub fn process<'a>(&self, message: &String) -> Vec<Response> {
        let mut responses = Vec::new();
        for data in &self.map {
            if data.triggers.iter().any(|a| message.contains(a)) {
                responses.push(data.response.clone());
            }
        }
        responses
    }
}

async fn handle_responses(responses: Vec<Response>, ctx: &Context, msg: &Message) -> CommandResult {
    for response in responses {
        let result = match &response {
            Response::Reaction(emote) => msg.react(ctx, ReactionType::try_from(emote.as_str())?).await.err(),
            Response::Reply(reply) => msg.reply(ctx, reply).await.err(),
        };
        // handle the error here so we can continue to go through any other responses
        if let Some(err) = result {
            println!("Error processing canned response {:?}: {:?}", response, err);
        }
    }

    Ok(())
}

async fn process_internal(ctx: &Context, msg: &Message) -> CommandResult {
    let data = ctx.data.read().await;
    let db = data.get::<Db>().ok_or("Unable to get database")?;
    let config = data.get::<Config>().ok_or("Unable to get config")?;

    if let Some(guild) = msg.guild_id {
        let result = db.read_guild(guild, |guild| {
            if let Some(table) = &guild.canned_response_table {
                Some(table.process(&msg.content))
            } else {
                None
            }
        })?.unwrap_or_default();

        if let Some(responses) = result {
            return handle_responses(responses, ctx, msg).await;
        }
    }

    // if we reached this point the guild didn't have a response table so we use the bot's default table
    handle_responses(config.canned_response_table.process(&msg.content), ctx, msg).await
}


#[hook]
pub async fn process(ctx: &Context, msg: &Message) {
    if let Err(err) = process_internal(ctx, msg).await {
        println!("Error processing canned responses: {:?}", err);
    }
}

impl Default for ResponseTable {
    fn default() -> Self {
        Self { map: vec![
            ResponseData {
                triggers: vec!["heresy".into(), "heretic".into(), "heretical".into()],
                response: Response::Reply("Heresy has no place on The Jouch".into()),
            },
        ] }
    }
}
