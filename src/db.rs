use std::collections::{HashMap, HashSet};
use chrono::{DateTime, FixedOffset};
use rustbreak::{deser::Ron, PathDatabase, error};
use serenity::framework::standard::CommandResult;
use serenity::prelude::TypeMapKey;
use serenity::model::id::{UserId,GuildId};
use serde::{Serialize, Deserialize};

use crate::commands::birthday::BirthdayPrivacy;

type DbType = PathDatabase<HashMap<u64, GuildData>, Ron>;

#[derive(Eq, PartialEq, Debug, Serialize, Deserialize, Clone, Default, Hash)]
pub struct UserKey {
    pub user: UserId,
    pub guild: GuildId,
}

pub struct Db {
    db: DbType,
}

impl Db {
    pub fn new() -> error::Result<Db> {
        Ok(Db { 
            db: DbType::load_from_path_or_default("jouch_db.ron".into())?,
        })
    }

    pub fn update<T, R>(&mut self, user_key: &UserKey, task: T) -> error::Result<R> 
    where 
        T: FnOnce(&mut UserData) -> R
    {
        let r = self.db.write(|db| { 
            let guild_entry = db.entry(user_key.guild.into()).or_default();
            let user_entry = guild_entry.users.entry(user_key.user.into()).or_default();
            task(user_entry)
        })?;
        self.db.save()?;
        Ok(r)
    }

    pub fn read_guild<T, R>(&self, guild: GuildId, task: T) -> error::Result<Option<R>> 
    where 
        T: FnOnce(&GuildData) -> R
    {
        Ok(self.db.read(|db| { 
            if let Some(guild_entry) = db.get(&guild.into()) {
                return Some(task(guild_entry));
            }
            return None;
        })?)
    }

    pub fn read<T, R>(&self, user_key: &UserKey, task: T) -> error::Result<Option<R>> 
    where 
        T: FnOnce(&UserData) -> R
    {
        Ok(self.db.read(|db| { 
            if let Some(guild_entry) = db.get(&user_key.guild.into()) {
                if let Some(user_entry) = guild_entry.users.get(&user_key.user.into()) {
                    return Some(task(user_entry));
                }
            }
            return None;
        })?)
    }

    pub fn get_users(&self, guild: GuildId) -> CommandResult<HashSet<UserKey>> {
        self.db.read(|db| {
            let guild_entry = db.get(&guild.into()).ok_or("Unable to find guild in database")?;
            
            Ok(guild_entry.users.keys().map(|u|{
                UserKey {
                    user: (*u).into(), 
                    guild,
                }
            }).collect())
        })?
    }

    pub fn get_guilds(&self) -> CommandResult<HashSet<GuildId>> {
        self.db.read(|db| {
            Ok(db.keys().map(|g|{(*g).into()}).collect())
        })?
    }

    pub fn foreach<T>(&self, guild: GuildId, mut task: T) -> error::Result<()> 
    where 
        T: FnMut(&u64, &UserData)
    {
        Ok(self.db.read(|db| {
            if let Some(guild_entry) = db.get(&guild.into()) {
                for (user_key, user_data) in &guild_entry.users {
                    task(&user_key,&user_data);
                }
            }
        })?)
    }
}

impl TypeMapKey for Db {
    type Value = Db;
}

#[derive(Eq, PartialEq, Debug, Serialize, Deserialize, Clone, Default)]
pub struct UserData {
    pub birthday: Option<DateTime<FixedOffset>>,
    pub birthday_privacy: Option<BirthdayPrivacy>,
    pub auto_nick: Option<String>,
}

#[derive(Eq, PartialEq, Debug, Serialize, Deserialize, Clone, Default)]
pub struct GuildData {
    pub users: HashMap<u64,UserData>,
    pub birthday_announce_channel: Option<u64>,
    pub birthday_announce_when_none: Option<bool>,
}
