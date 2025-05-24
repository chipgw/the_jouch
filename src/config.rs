use serde::{Deserialize, Serialize};
use serenity::{json::json, prelude::TypeMapKey};
use sqlx::{prelude::FromRow, PgPool};

use crate::canned_responses::ResponseTable;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct Config {
    pub nick_interval: i64,
    #[sqlx(json)]
    pub canned_response_table: ResponseTable,
}

impl Config {
    pub(crate) async fn load(db: &PgPool) -> anyhow::Result<Self> {
        Ok(sqlx::query_as("SELECT * FROM config")
            .fetch_optional(db)
            .await?
            .unwrap_or_default())
    }

    pub(crate) async fn save(&self, db: &PgPool) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO config(nick_interval, canned_response_table)
            VALUES ($1, $2)
            ON CONFLICT (id)
            DO UPDATE SET
            nick_interval = $1,
            canned_response_table = $2",
        )
        .bind(self.nick_interval)
        .bind(json!(self.canned_response_table))
        .execute(db)
        .await?;

        Ok(())
    }
}
impl Default for Config {
    fn default() -> Self {
        Self {
            nick_interval: crate::commands::autonick::DEFAULT_INTERVAL,
            canned_response_table: Default::default(),
        }
    }
}

impl TypeMapKey for Config {
    type Value = Config;
}
