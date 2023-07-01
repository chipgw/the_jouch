use std::collections::HashMap;
use anyhow::bail;
use chrono::{DateTime, FixedOffset};
use mongodb::bson::{doc, to_bson};
use serenity::{model::{id::{UserId,GuildId}, application::interaction::application_command::ApplicationCommandInteraction, prelude::{interaction::{application_command::CommandDataOptionValue, InteractionResponseType}}}, prelude::Context};
use serde::{Serialize, Deserialize};
use ron::de::from_bytes;
use anyhow::anyhow;
use tracing::debug;
use crate::{commands::{sit::JouchOrientation, birthday::BirthdayPrivacy}, canned_responses::ResponseTable, db::{Db, UserKey}, ShuttleItemsContainer};

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

impl Default for Config {
    fn default() -> Self {
        Self {
            nick_interval: crate::commands::autonick::DEFAULT_INTERVAL,
            canned_response_table: Default::default(),
            testing_guild_id: None,
            token: "".into(),
            app_id: 0,
        }
    }
}

type DbData = HashMap<u64, GuildData>;

pub async fn migrate(ctx: &Context, command: &ApplicationCommandInteraction) -> anyhow::Result<()> {
    if command.data.options.len() == 0 {
        bail!("No arguments supplied!");
    }

    let mut msg = command.create_followup_message(&ctx, |r|{
        r.content("Are you sure? This will overwrite any existing data.");
        r.ephemeral(true);
        r.components(|c|{
            c.create_action_row(|row|{
                row.create_button(|b|{
                    b.style(serenity::model::prelude::component::ButtonStyle::Danger).label("Import").custom_id("migrate_confirm")
                }).create_button(|b|{
                    b.style(serenity::model::prelude::component::ButtonStyle::Secondary).label("Cancel").custom_id("migrate_cancel")
                })
            })
        })
    }).await?;
    // TODO - should this timeout?
    if let Some(interaction) = msg.await_component_interaction(&ctx).await {
        let confirmed = interaction.data.custom_id == "migrate_confirm";
        interaction.create_interaction_response(&ctx, |r|{
            r.kind(InteractionResponseType::UpdateMessage).interaction_response_data(|d|{
                d.content(if confirmed {
                    "Proccessing..."
                } else {
                    "Canceled."
                })
                .components(|c|{ c })
            })
        }).await?;

        if !confirmed {
            return Ok(());
        }
    }

    for arg in &command.data.options {
        if let Some(CommandDataOptionValue::Attachment(attachment)) = &arg.resolved {
            let data = attachment.download().await?;
            match arg.name.as_str() {
                "jouch_db" => {
                    let db_data: DbData = from_bytes(&data)?;

                    let mut data = ctx.data.write().await;
                    let db = data.get_mut::<Db>().ok_or(anyhow!("Unable to get database"))?;

                    for (guild_id, guild_data) in &db_data {
                        let guild_id = GuildId::from(*guild_id);
                        db.update_guild(guild_id, doc!{"$set": {
                            "birthday_announce_channel": to_bson(&guild_data.birthday_announce_channel)?,
                            "birthday_announce_when_none": to_bson(&guild_data.birthday_announce_when_none)?,
                            "canned_response_table": to_bson(&guild_data.canned_response_table)?,
                            "jouch_orientation": to_bson(&guild_data.jouch_orientation)?,
                        }}).await?;

                        for (user_id, user_data) in &guild_data.users {
                            db.update(&UserKey{user: UserId::from(*user_id), guild: guild_id}, doc!{"$set": {
                                "birthday": to_bson(&user_data.birthday)?,
                                "birthday_privacy": to_bson(&user_data.birthday_privacy)?,
                                "auto_nick": to_bson(&user_data.auto_nick)?,
                                "sit_count": to_bson(&user_data.sit_count)?,
                                "flip_count": to_bson(&user_data.flip_count)?,
                            }}).await?;
                        }
                    }
                },
                "config" => {
                    let old_config: Config = from_bytes(&data)?;
                    
                    let mut data = ctx.data.write().await;
                    
                    {
                        let config = data.get_mut::<crate::config::Config>().ok_or(anyhow!("Unable to get config"))?;

                        config.canned_response_table = old_config.canned_response_table;
                        config.nick_interval = old_config.nick_interval;
                    }

                    let config = data.get::<crate::config::Config>().ok_or(anyhow!("Unable to get config"))?;
                    let shuttle_items = data.get::<ShuttleItemsContainer>().ok_or(anyhow!("Unable to get ShuttleItemsContainer!"))?;

                    debug!("Updated Config to: {:#?}", config);

                    config.save(&shuttle_items.persist)?;

                    debug!("Config as saved in shuttle persisted storage: {:#?}", crate::config::Config::load(&shuttle_items.persist));
                },
                _ => bail!("option {} not recognized!", arg.name),
            }
        }
    }
    msg.edit(&ctx, |r|{
        r.content("Complete.")
    }).await?;

    Ok(())
}


#[derive(Eq, PartialEq, Debug, Serialize, Deserialize, Clone, Default)]
pub struct UserData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub birthday: Option<DateTime<FixedOffset>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub birthday_privacy: Option<BirthdayPrivacy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_nick: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sit_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flip_count: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct GuildData {
    pub users: HashMap<u64,UserData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub birthday_announce_channel: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub birthday_announce_when_none: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canned_response_table: Option<ResponseTable>,
    #[serde(default)]
    pub jouch_orientation: JouchOrientation,
}
