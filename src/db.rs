use crate::canned_responses::ResponseTable;
use crate::commands::{birthday::BirthdayPrivacy, sit::JouchOrientation};
use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use serenity::all::{GuildId, UserId};
use serenity::prelude::TypeMapKey;
use sqlx::{Encode, FromRow, PgPool, Postgres, QueryBuilder};
use std::collections::HashSet;
use tracing::debug;
#[derive(Eq, PartialEq, Debug, Serialize, Deserialize, Clone, Default, FromRow)]
pub struct UserKey {
    #[sqlx(rename = "guild_id")]
    pub guild: i64,
    #[sqlx(rename = "user_id")]
    pub user: i64,
}

pub struct Db {
    db: PgPool,
}

impl Db {
    pub fn new(database: PgPool) -> Db {
        Db { db: database }
    }

    pub async fn update<'q, T>(
        &self,
        user_key: &UserKey,
        field: &str,
        value: T,
    ) -> anyhow::Result<UserData>
    where
        T: 'q + Encode<'q, Postgres> + sqlx::Type<Postgres>,
    {
        let mut query = QueryBuilder::new(&format!(
            "INSERT INTO users(guild_id, user_id, {field}) VALUES ("
        ));
        query
            .separated(",")
            .push_bind(user_key.guild)
            .push_bind(user_key.user)
            .push_bind(value);
        query.push(&format!(") ON CONFLICT (guild_id, user_id) DO UPDATE SET {field} = EXCLUDED.{field} RETURNING *"));

        debug!("query: {}", query.sql());

        Ok(query.build_query_as().fetch_one(&self.db).await?)
    }

    pub async fn increment(&self, user_key: &UserKey, field: &str) -> anyhow::Result<UserData> {
        let mut query = QueryBuilder::new(&format!(
            "INSERT INTO users(guild_id, user_id, {field}) VALUES ("
        ));
        query
            .separated(",")
            .push_bind(user_key.guild)
            .push_bind(user_key.user)
            .push(1);
        query.push(&format!(") ON CONFLICT (guild_id, user_id) DO UPDATE SET {field} = users.{field} + 1 RETURNING *"));

        debug!("query: {}", query.sql());

        Ok(query.build_query_as().fetch_one(&self.db).await?)
    }

    pub async fn update_guild<'q, T>(
        &self,
        guild: GuildId,
        field: &str,
        value: T,
    ) -> anyhow::Result<GuildData>
    where
        T: 'q + Encode<'q, Postgres> + sqlx::Type<Postgres>,
    {
        let mut query = QueryBuilder::new(&format!("INSERT INTO guilds(id, {field}) VALUES ("));
        query
            .separated(",")
            .push_bind(guild.get() as i64)
            .push_bind(value);
        query.push(&format!(
            ") ON CONFLICT (id) DO UPDATE SET {field} = EXCLUDED.{field} RETURNING *"
        ));

        let query_str = query.sql();

        debug!("query: {query_str}");

        Ok(query.build_query_as().fetch_one(&self.db).await?)
    }

    pub async fn read_guild(&self, guild: GuildId) -> anyhow::Result<Option<GuildData>> {
        Ok(sqlx::query_as("SELECT * FROM guilds WHERE id = $1")
            .bind(guild.get() as i64)
            .fetch_optional(&self.db)
            .await?)
    }

    pub async fn read(&self, user_key: &UserKey) -> anyhow::Result<Option<UserData>> {
        Ok(
            sqlx::query_as("SELECT * FROM users WHERE guild_id = $1 AND user_id = $2")
                .bind(user_key.guild)
                .bind(user_key.user)
                .fetch_optional(&self.db)
                .await?,
        )
    }

    pub async fn get_users(
        &self,
        guild: GuildId,
        extra_query: &str,
    ) -> anyhow::Result<Vec<UserData>> {
        Ok(
            sqlx::query_as(&("SELECT * FROM users WHERE guild_id = $1 ".to_owned() + extra_query))
                .bind(guild.get() as i64)
                .fetch_all(&self.db)
                .await?,
        )
    }

    // get all guilds with an entry in guild collection.
    pub async fn get_guilds(&self) -> anyhow::Result<HashSet<GuildId>> {
        Ok(sqlx::query_scalar("SELECT id FROM guilds")
            .fetch_all(&self.db)
            .await?
            .into_iter()
            .map(|a: i64| (a as u64).into())
            .collect())
    }
    // get all guilds with a user (specified or any) in user collection
    pub async fn get_user_guilds(&self, user: Option<UserId>) -> anyhow::Result<HashSet<GuildId>> {
        Ok(if let Some(user) = user {
            sqlx::query_scalar("SELECT DISTINCT guild_id FROM users WHERE user_id = $1")
                .bind(user.get() as i64)
        } else {
            sqlx::query_scalar("SELECT DISTINCT guild_id FROM users")
        }
        .fetch_all(&self.db)
        .await?
        .into_iter()
        .map(|a: i64| (a as u64).into())
        .collect())
    }

    pub fn pool(&self) -> &PgPool {
        &self.db
    }
}

impl TypeMapKey for Db {
    type Value = Db;
}

#[derive(Eq, PartialEq, Debug, Serialize, Deserialize, Clone, Default, FromRow)]
pub struct UserData {
    #[sqlx(flatten)]
    pub id: UserKey,
    pub birthday: Option<DateTime<FixedOffset>>,
    pub birthday_privacy: Option<BirthdayPrivacy>,
    pub auto_nick: Option<String>,
    pub sit_count: i32,
    pub flip_count: i32,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, FromRow)]
pub struct GuildData {
    pub id: i64,
    pub birthday_announce_channel: Option<i64>,
    pub birthday_announce_when_none: Option<bool>,
    #[sqlx(json(nullable))]
    pub canned_response_table: Option<ResponseTable>,
    #[serde(default)]
    pub jouch_orientation: JouchOrientation,
}
