use ron::{de::from_reader,ser::{to_writer_pretty, PrettyConfig}};
use std::{fs::File, io::{Error,ErrorKind}, path::Path};
use serde::{Serialize,Deserialize};
use serenity::prelude::TypeMapKey;

use crate::canned_responses::ResponseTable;

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    pub token: String,
    pub app_id: u64,
    #[serde(default = "crate::commands::autonick::default_interval")]
    pub nick_interval: u64,
    #[serde(default)]
    pub canned_response_table: ResponseTable,
    #[serde(default)]
    pub testing_guild_id: Option<u64>,
}

const CONFIG_PATH: &str = "config.ron";

impl Config {
    pub fn load() -> std::io::Result<Self>  {
        if Path::new(CONFIG_PATH).exists() {
            let f = File::open(CONFIG_PATH)?;
            let config: Config = from_reader(f).map_err(|e| {Error::new(ErrorKind::Other, e)})?;
            Ok(config)
        } else {
            Ok(Default::default())
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let f = File::create(CONFIG_PATH)?;
        
        to_writer_pretty(f, &self, PrettyConfig::default())
            .map_err(|e| {Error::new(ErrorKind::Other, e)})?;

        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            nick_interval: crate::commands::autonick::DEFAULT_INTERVAL,
            canned_response_table: Default::default(),
            testing_guild_id: None,
            // To be filled in by deployer
            token: "".into(),
            app_id: 0,
        }
    }
}

impl TypeMapKey for Config {
    type Value = Config;
}
