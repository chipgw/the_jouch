use std::collections::HashMap;

use chrono::{DateTime, FixedOffset};
use rustbreak::{deser::Ron, PathDatabase, error};
use serenity::prelude::TypeMapKey;
use serenity::model::id::{UserId,GuildId};
use serde::{Serialize, Deserialize};
use merge::Merge;

type DbType = PathDatabase<HashMap<u128, UserData>, Ron>;

// Would have made a struct but it won't work with the database/serialization of HashMap
#[derive(Eq, PartialEq, Debug, Serialize, Deserialize, Clone, Default)]
pub struct UserKey {
    pub user: UserId,
    pub guild: GuildId,
}

impl From<UserKey> for u128 {
    fn from(key: UserKey) -> Self {
        (u64::from(key.user) as u128) << 64 | u64::from(key.guild) as u128
    }
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

    pub fn update<T, R>(&mut self, user_key: UserKey, task: T) -> error::Result<R> 
    where 
        T: FnOnce(&mut UserData) -> R
    {
        let r = self.db.write(|db| { 
            let entry = db.entry(user_key.into()).or_default();
            task(entry)
        })?;
        self.db.save()?;
        Ok(r)
    }
}

impl TypeMapKey for Db {
    type Value = Db;
}

#[derive(Eq, PartialEq, Debug, Serialize, Deserialize, Clone, Default, Merge)]
pub struct UserData {
    pub birthday: Option<DateTime<FixedOffset>>,
    pub auto_nick: Option<String>,
}
