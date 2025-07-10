use anyhow::anyhow;
use rand::distr::{weighted::WeightedIndex, Distribution};
use serde::{Deserialize, Serialize};
use serenity::{
    model::channel::{Message, ReactionType},
    prelude::*,
};
use std::{convert::TryFrom, ops::ControlFlow};
use tracing::error;

use crate::CommandResult;
use crate::{config::Config, db::Db};

#[derive(Eq, PartialEq, Serialize, Deserialize, Clone, Debug)]
pub enum Response {
    Reply(String),
    Reaction(String),
    // list of responses with the weight at which they should be chosen
    RandomChance(Vec<(u64, Response)>),
    // indended for use in RandomChance responses, to make it possible for it to have a chance of not responding at all
    NoResponse,
}

impl Response {
    fn flatten(&self) -> CommandResult<&Self> {
        if let Response::RandomChance(items) = self {
            let index = WeightedIndex::new(items.iter().map(|(x, _)| x))?;
            let (_, chosen) = &items[index.sample(&mut rand::rng())];
            // technically allows nesting, but why would you want to?
            chosen.flatten()
        } else {
            Ok(self)
        }
    }
}

#[derive(Eq, PartialEq, Serialize, Deserialize, Clone, Debug)]
pub enum Trigger {
    FullMatch(String),
    StartsWith(String),
    EndsWith(String),
    // number represents minumum length the word should be
    RepeatedCharacter(char, usize),
    // requires a chain of consecutive words to match each respective trigger
    ConsecutiveWords(Vec<Trigger>),
}

impl From<&str> for Trigger {
    fn from(other: &str) -> Self {
        Self::FullMatch(other.into())
    }
}

impl Trigger {
    fn process_word(&self, other: &str, acc: usize) -> (bool, usize) {
        match self {
            Trigger::FullMatch(word) => (word == other, 0),
            Trigger::StartsWith(pat) => (other.starts_with(pat), 0),
            Trigger::EndsWith(pat) => (other.ends_with(pat), 0),
            Trigger::RepeatedCharacter(a, min) => {
                (other.len() >= *min && other.chars().all(|ref b| a == b), 0)
            }
            Trigger::ConsecutiveWords(triggers) => {
                // accumulator is ignored for this since nested Trigger::ConsecutiveWords are not allowed.
                // TODO - block nested Trigger::ConsecutiveWords somehow.
                if triggers[acc].process_word(other, 0).0 {
                    (triggers.len() == acc + 1, acc + 1)
                } else {
                    (false, 0)
                }
            }
        }
    }

    fn process<'a>(&self, words: &Vec<&'a str>) -> bool {
        words
            .iter()
            .try_fold(0, |acc, other| {
                let (matched, acc) = self.process_word(other, acc);

                if matched {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(acc)
                }
            })
            .is_break()
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
            if data.triggers.iter().any(|a| a.process(words)) {
                responses.extend_from_slice(&data.responses);
            }
        }
        responses
    }
}

async fn handle_response(response: &Response, ctx: &Context, msg: &Message) -> CommandResult {
    match response.flatten()? {
        Response::Reaction(emote) => {
            msg.react(ctx, ReactionType::try_from(emote.as_str())?)
                .await?;
        }
        Response::Reply(reply) => {
            msg.reply(ctx, reply).await?;
        }
        Response::RandomChance(_) => {
            return Err(anyhow!("Response::RandomChance failed to flatten!"))
        }
        Response::NoResponse => (),
    };
    Ok(())
}

async fn handle_responses(responses: Vec<Response>, ctx: &Context, msg: &Message) -> CommandResult {
    for response in responses {
        // handle the error here so we can continue to go through any other responses
        if let Err(err) = handle_response(&response, ctx, msg).await {
            error!("Error processing canned response {:?}: {:?}", response, err);
        }
    }

    Ok(())
}

pub async fn process(ctx: &Context, msg: &Message) -> CommandResult {
    let data = ctx.data.read().await;
    let db = data.get::<Db>().ok_or(anyhow!("Unable to get database"))?;
    let config = data
        .get::<Config>()
        .ok_or(anyhow!("Unable to get config"))?;

    let message_lower = msg.content.to_lowercase();
    let words = message_lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect();

    if let Some(guild) = msg.guild_id {
        let result = db.read_guild(guild).await?.and_then(|guild| {
            guild
                .canned_response_table
                .as_ref()
                .map(|table| table.process(&words))
        });

        if let Some(responses) = result {
            return handle_responses(responses, ctx, msg).await;
        }
    }

    // if we reached this point the guild didn't have a response table so we use the bot's default table
    handle_responses(config.canned_response_table.process(&words), ctx, msg).await
}

impl Default for ResponseTable {
    fn default() -> Self {
        Self {
            map: vec![ResponseData {
                triggers: vec![
                    Trigger::FullMatch("heresy".into()),
                    Trigger::StartsWith("heretic".into()),
                    Trigger::FullMatch("heresies".into()),
                ],
                responses: vec![
                    Response::Reply("Heresy has no place on The Jouch".into()),
                    Response::Reaction("<:bythepope:881212318707482674>".into()),
                ],
            }],
        }
    }
}
