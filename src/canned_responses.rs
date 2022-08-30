use std::convert::TryFrom;
use serenity::{framework::standard::{CommandResult, macros::hook}, model::channel::{Message, ReactionType}, prelude::*};
use serde::{Deserialize, Serialize};

use crate::{config::Config, db::Db};

#[derive(Eq, PartialEq, Serialize, Deserialize, Clone, Debug)]
pub enum Response {
    Reply(String),
    Reaction(String),
}

#[derive(Eq, PartialEq, Serialize, Deserialize, Clone, Debug)]
pub enum Trigger {
    FullMatch(String),
    StartsWith(String),
    EndsWith(String),
    RepeatedCharacter(char),
}

impl From<&str> for Trigger {
    fn from(other: &str) -> Self {
        Self::FullMatch(other.into())
    }
}

impl PartialEq<&str> for Trigger {
    fn eq(&self, other: &&str) -> bool {
        match self {
            Trigger::FullMatch(word) => word == other,
            Trigger::StartsWith(pat) => other.starts_with(pat),
            Trigger::EndsWith(pat) => other.ends_with(pat),
            Trigger::RepeatedCharacter(a) => other.chars().all(|ref b| a == b),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ResponseData {
    pub triggers: Vec<Trigger>,
    pub responses: Vec<Response>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ResponseTable {
    map: Vec<ResponseData>,
}

impl ResponseTable {
    pub fn process<'a>(&self, words: &Vec<&'a str>) -> Vec<Response> {
        let mut responses = Vec::new();
        for data in &self.map {
            if data.triggers.iter().any(|a| words.into_iter().any(|b| { a == b })) {
                responses.extend_from_slice(&data.responses);
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

    let message_lower = msg.content.to_lowercase();
    let words = message_lower
        .split(|c: char| { !c.is_alphabetic() })
        .filter(|s| !s.is_empty())
        .collect();

    if let Some(guild) = msg.guild_id {
        let result = db.read_guild(guild, |guild| {
            guild.canned_response_table.as_ref().map(|table|{
                table.process(&words)
            })
        })?.unwrap_or_default();

        if let Some(responses) = result {
            return handle_responses(responses, ctx, msg).await;
        }
    }

    // if we reached this point the guild didn't have a response table so we use the bot's default table
    handle_responses(config.canned_response_table.process(&words), ctx, msg).await
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
                triggers: vec![Trigger::FullMatch("heresy".into()), Trigger::StartsWith("heretic".into()), Trigger::FullMatch("heresies".into())],
                responses: vec![Response::Reply("Heresy has no place on The Jouch".into()), Response::Reaction("<:bythepope:881212318707482674>".into())],
            },
        ] }
    }
}
