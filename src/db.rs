use std::collections::HashSet;
use chrono::{DateTime, FixedOffset};
use mongodb::bson::{Document, doc, to_bson};
use mongodb::options::UpdateModifications;
use mongodb::{Collection, Database, Cursor};
use serenity::prelude::TypeMapKey;
use serenity::model::id::{UserId,GuildId};
use serde::{Serialize, Deserialize};
use anyhow::anyhow;

use crate::{commands::{sit::JouchOrientation, birthday::BirthdayPrivacy}, canned_responses::ResponseTable};

#[derive(Eq, PartialEq, Debug, Serialize, Deserialize, Clone, Default)]
pub struct UserKey {
    pub guild: GuildId,
    pub user: UserId,
}

#[derive(Eq, PartialEq, Debug, Serialize, Deserialize, Clone, Default)]
pub struct GuildKey { 
    pub guild: GuildId,
}

impl Into<Document> for &GuildKey {
    fn into(self) -> Document {
        doc!{"_id.guild": self.guild.0.to_string()}
    }
}

impl Into<Option<Document>> for &GuildKey {
    fn into(self) -> Option<Document> {
        Some(self.into())
    }
}

impl Into<Document> for &UserKey {
    fn into(self) -> Document {
        doc!{"_id": {"guild": self.guild.0.to_string(), "user": self.user.0.to_string()}}
    }
}
impl Into<Option<Document>> for &UserKey {
    fn into(self) -> Option<Document> {
        Some(self.into())
    }
}

pub struct Db {
    _db: Database,
    user_collection: Collection<UserData>,
    guild_collection: Collection<GuildData>,
}

impl Db {
    pub fn new(database: Database) -> Db {
        Db {
            user_collection: database.collection("users"),
            guild_collection: database.collection("guilds"),
            _db: database,
        }
    }

    pub async fn update<T>(&mut self, user_key: &UserKey, task: T) -> anyhow::Result<UserData> 
    where 
        T: Into<UpdateModifications>
    {
        if self.user_collection.find_one(user_key, None).await?.is_none() {
            let new_user_data = UserData::default_with_key(user_key);
            
            self.user_collection.insert_one(new_user_data, None).await?;
        };
        
        self.user_collection.update_one(user_key.into(), task, None).await?;

        self.user_collection.find_one(user_key, None).await?.ok_or(anyhow!("error getting user after update!"))
    }

    pub async fn update_guild<T>(&mut self, guild: GuildId, task: T) -> anyhow::Result<GuildData>
    where 
        T: Into<UpdateModifications>
    {
        let ref guild_key = GuildKey{guild};

        if self.guild_collection.find_one(guild_key, None).await?.is_none() {
            let new_guild_data = GuildData::default_with_key(&guild_key);
            
            self.guild_collection.insert_one(new_guild_data, None).await?;
        };
        
        self.guild_collection.update_one(guild_key.into(), task, None).await?;

        self.guild_collection.find_one(guild_key, None).await?.ok_or(anyhow!("error getting guild after update!"))
    }

    pub async fn read_guild(&self, guild: GuildId) -> anyhow::Result<Option<GuildData>> {
        Ok(self.guild_collection.find_one(&GuildKey{guild}, None).await?)
    }

    pub async fn read(&self, user_key: &UserKey) -> anyhow::Result<Option<UserData>> {
        Ok(self.user_collection.find_one(user_key, None).await?)
    }

    pub async fn get_users(&self, guild: GuildId) -> anyhow::Result<Cursor<UserData>> {
        Ok(self.user_collection.find(&GuildKey{guild}, None).await?)
    }

    // get all guilds with an entry in guild collection.
    pub async fn get_guilds(&self) -> anyhow::Result<HashSet<GuildId>> {
        let v = self.guild_collection.distinct("_id.guild", None, None).await?;
        Ok(v.iter().map(|a|{a.as_str().unwrap().parse::<u64>().unwrap().into()}).collect())
    }
    // get all guilds with a user (specified or any) in user collection
    pub async fn get_user_guilds(&self, user: Option<UserId>) -> anyhow::Result<HashSet<GuildId>> {
        let filter = user.and_then(|u|{Some(doc!{"_id.user": to_bson(&u).ok()?})});
        let v = self.user_collection.distinct("_id.guild", filter, None).await?;
        Ok(v.iter().map(|a|{a.as_str().unwrap().parse::<u64>().unwrap().into()}).collect())
    }

    pub async fn foreach<T>(&self, guild: GuildId, mut task: T) -> anyhow::Result<()> 
    where 
        T: FnMut(&UserData)
    {
        let mut users = self.user_collection.find(&GuildKey{guild}, None).await?;
        while users.advance().await? {
            let user_data = users.deserialize_current()?;
            task(&user_data);
        };
        Ok(())
    }
}

impl TypeMapKey for Db {
    type Value = Db;
}

#[derive(Eq, PartialEq, Debug, Serialize, Deserialize, Clone, Default)]
pub struct UserData {
    pub _id: UserKey,
    pub birthday: Option<DateTime<FixedOffset>>,
    pub birthday_privacy: Option<BirthdayPrivacy>,
    pub auto_nick: Option<String>,
    pub sit_count: Option<i64>,
    pub flip_count: Option<i64>,
}

impl UserData {
    pub fn default_with_key(user_key: &UserKey) -> Self {
        Self {
            _id: user_key.clone(),
            ..Default::default()
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct GuildData {
    pub _id: GuildKey,
    pub birthday_announce_channel: Option<u64>,
    pub birthday_announce_when_none: Option<bool>,
    pub canned_response_table: Option<ResponseTable>,
    #[serde(default)]
    pub jouch_orientation: JouchOrientation,
}

impl GuildData {
    pub fn default_with_key(guild_key: &GuildKey) -> Self {
        Self {
            _id: guild_key.clone(),
            ..Default::default()
        }
    }
}
