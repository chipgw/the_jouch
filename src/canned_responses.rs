use anyhow::anyhow;
use core::slice;
use rand::distr::{weighted::WeightedIndex, Distribution};
use serde::{Deserialize, Serialize};
use serenity::{
    model::channel::{Message, ReactionType},
    prelude::*,
};
use std::convert::TryFrom;
use tracing::error;

use crate::config::Config;
use crate::db::Db;
use crate::CommandResult;

#[derive(Eq, PartialEq, Serialize, Deserialize, Clone, Debug)]
pub enum Response {
    Reply(String),
    Reaction(String),
    // list of responses with the weight at which they should be chosen
    RandomChance(Vec<(u64, Response)>),
    Multiple(Vec<Response>),
    // indended for use in RandomChance responses, to make it possible for it to have a chance of not responding at all
    NoResponse,
}

impl Response {
    fn flatten(&self) -> CommandResult<Vec<&Self>> {
        match self {
            Response::RandomChance(items) => {
                let index = WeightedIndex::new(items.iter().map(|(x, _)| x))?;
                let (_, chosen) = &items[index.sample(&mut rand::rng())];
                chosen.flatten()
            }
            Response::Multiple(responses) => {
                let mut out_responses = Vec::<&Self>::new();
                for response in responses {
                    out_responses.append(&mut response.flatten()?);
                }
                Ok(out_responses)
            }
            _ => Ok(vec![self]),
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
    // triggers if any of these trigger
    MultipleOptions(Vec<Trigger>),
}

impl From<&str> for Trigger {
    fn from(other: &str) -> Self {
        Self::FullMatch(other.into())
    }
}

impl Trigger {
    fn process(&self, words: &[&str]) -> bool {
        match self {
            Trigger::FullMatch(word) => words.iter().any(|other| other == word),
            Trigger::StartsWith(pat) => words.iter().any(|other| other.starts_with(pat)),
            Trigger::EndsWith(pat) => words.iter().any(|other| other.ends_with(pat)),
            Trigger::RepeatedCharacter(a, min) => words
                .iter()
                .any(|other| other.len() >= *min && other.chars().all(|ref b| a == b)),
            Trigger::ConsecutiveWords(triggers) => words.windows(triggers.len()).any(|window| {
                window
                    .iter()
                    .zip(triggers.iter())
                    .all(|(a, b)| b.process(slice::from_ref(a)))
            }),
            Trigger::MultipleOptions(triggers) => triggers.iter().any(|t| t.process(words)),
        }
    }

    fn as_key(&self) -> String {
        match self {
            Trigger::FullMatch(val) => val.to_owned(),
            Trigger::StartsWith(val) => val.to_owned() + "*",
            Trigger::EndsWith(val) => "*".to_owned() + val,
            Trigger::RepeatedCharacter(char, num) => char.to_string().repeat(*num),
            Trigger::ConsecutiveWords(triggers) => triggers
                .iter()
                .map(|v| v.as_key())
                .collect::<Vec<String>>()
                .join(" "),
            Trigger::MultipleOptions(triggers) => {
                // will return a blank value if empty, which will migration will interpret as an error
                triggers.first().map(|v| v.as_key()).unwrap_or_default()
            }
        }
    }
}

// A version of the struct that allows for backwards compatability
#[derive(Deserialize, Debug)]
#[serde(untagged)]
enum ResponseDataMigration {
    Version1 {
        triggers: Vec<Trigger>,
        responses: Vec<Response>,
        #[serde(default)]
        track_by: String,
    },
    Version2 {
        trigger: Trigger,
        response: Response,
        track_by: String,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(try_from = "ResponseDataMigration")]
pub struct ResponseData {
    pub trigger: Trigger,
    pub response: Response,
    pub track_by: String,
}

impl TryFrom<ResponseDataMigration> for ResponseData {
    type Error = String;

    fn try_from(value: ResponseDataMigration) -> Result<Self, Self::Error> {
        Ok(match value {
            ResponseDataMigration::Version1 {
                mut triggers,
                mut responses,
                track_by,
            } => {
                let trigger = if triggers.len() == 1 {
                    triggers.pop().unwrap()
                } else {
                    Trigger::MultipleOptions(triggers)
                };
                let response = if responses.len() == 1 {
                    responses.pop().unwrap()
                } else {
                    Response::Multiple(responses)
                };
                let track_by = if track_by.is_empty() {
                    trigger.as_key()
                } else {
                    track_by
                };
                if track_by.is_empty() || track_by == "*" {
                    // a trigger that returns a blank string or only an asterisk from as_key() would be unusable anyway
                    return Err(format!(
                        "Unable to generate track_by for trigger: `{:?}`, is it empty?",
                        trigger
                    ));
                }
                Self {
                    trigger,
                    response,
                    track_by,
                }
            }
            ResponseDataMigration::Version2 {
                trigger,
                response,
                track_by,
            } => Self {
                trigger,
                response,
                track_by,
            },
        })
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ResponseTable {
    map: Vec<ResponseData>,
}

impl ResponseTable {
    pub fn process(&self, words: &[&str]) -> Vec<&ResponseData> {
        self.map
            .iter()
            .filter(|data| data.trigger.process(words))
            .collect()
    }
}

async fn handle_response(response: &Response, ctx: &Context, msg: &Message) -> CommandResult<bool> {
    match response {
        Response::Reaction(emote) => {
            msg.react(ctx, ReactionType::try_from(emote.as_str())?)
                .await?;
            Ok(true)
        }
        Response::Reply(reply) => {
            msg.reply(ctx, reply).await?;
            Ok(true)
        }
        Response::RandomChance(_) => Err(anyhow!(
            "Response::RandomChance was passed to handle_response not flattened!"
        )),
        Response::NoResponse => Ok(false),
        Response::Multiple(_) => Err(anyhow!(
            "Response::Multiple was passed to handle_response not flattened!"
        )),
    }
}

async fn handle_responses(
    responses: Vec<&ResponseData>,
    ctx: &Context,
    msg: &Message,
) -> CommandResult {
    for response_data in responses {
        let mut any = false;
        for response in &response_data.response.flatten()? {
            match handle_response(response, ctx, msg).await {
                Ok(triggered) => any |= triggered,
                Err(err) => error!("Error processing canned response {:?}: {:?}", response, err),
            }
        }
        if any {
            // TODO - decide whether it's actually desireable for errors to propogate up the chain from here or whether we should just log and continue
            let data = ctx.data.read().await;
            let db = data.get::<Db>().ok_or(anyhow!("Unable to get database"))?;

            db.increment_reaction(
                &crate::db::UserKey {
                    guild: msg.guild_id.unwrap_or_default().into(),
                    user: msg.author.id.into(),
                },
                &response_data.track_by,
            )
            .await?;
        }
    }

    Ok(())
}

pub async fn process(ctx: &Context, msg: &Message) -> CommandResult {
    let message_lower = msg.content.to_lowercase();
    let words: Vec<&str> = message_lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect();

    let data = ctx.data.read().await;

    if let Some(guild) = msg.guild_id {
        let db = data.get::<Db>().ok_or(anyhow!("Unable to get database"))?;

        let response_table = db
            .read_guild(guild)
            .await?
            .and_then(|guild| guild.canned_response_table);

        if let Some(response_table) = response_table {
            return handle_responses(response_table.process(&words), ctx, msg).await;
        }
    }

    // if we reached this point the guild didn't have a response table so we use the bot's default table
    let config = data
        .get::<Config>()
        .ok_or(anyhow!("Unable to get config"))?;
    handle_responses(config.canned_response_table.process(&words), ctx, msg).await
}

impl Default for ResponseTable {
    fn default() -> Self {
        Self {
            map: vec![ResponseData {
                trigger: Trigger::MultipleOptions(vec![
                    Trigger::FullMatch("heresy".into()),
                    Trigger::StartsWith("heretic".into()),
                    Trigger::FullMatch("heresies".into()),
                ]),
                response: Response::Multiple(vec![
                    Response::Reply("Heresy has no place on The Jouch".into()),
                    Response::Reaction("<:bythepope:881212318707482674>".into()),
                ]),
                track_by: "heresy".into(),
            }],
        }
    }
}
