use ron::{de::from_reader,ser::to_writer};
use std::{fs::File, io::{Error,ErrorKind}, path::Path};
use serde::{Serialize,Deserialize};
use serenity::prelude::TypeMapKey;

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    pub token: String,
    pub prefix: String,
}

const CONFIG_PATH: &str = "config.ron";

impl Config {
    pub fn load() -> std::io::Result<Self>  {
        if Path::new(CONFIG_PATH).exists() {
            let f = File::open(CONFIG_PATH)?;
            let config: Config = from_reader(f).map_err(|e| {Error::new(ErrorKind::Other, e)})?;
            Ok(config)
        } else {
            Ok(Config {
                token: String::new(),
                prefix: "~".into(),
            })
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let f = File::create(CONFIG_PATH)?;
        
        to_writer(f, &self).map_err(|e| {Error::new(ErrorKind::Other, e)})?;

        Ok(())
    }
}

impl TypeMapKey for Config {
    type Value = Config;
}
