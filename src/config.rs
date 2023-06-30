use anyhow::bail;
use serde::{Serialize, Deserialize};
use serenity::prelude::TypeMapKey;
use shuttle_persist::{PersistError, PersistInstance};

use crate::canned_responses::ResponseTable;

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub nick_interval: u64,
    pub canned_response_table: ResponseTable,
}

impl Config {
    pub(crate) fn load(persist: &PersistInstance) -> anyhow::Result<Self> {
        Ok(match persist.load::<Self>("config") {
            Ok(config) => config,
            Err(PersistError::Open(_)) => Default::default(),
            Err(_) => bail!("Unable to load config!"),
        })
    }

    pub(crate) fn save(&self, persist: &PersistInstance) -> anyhow::Result<()> {
        Ok(persist.save("config", self)?)
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
