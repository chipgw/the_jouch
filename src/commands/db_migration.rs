use crate::{
    canned_responses::ResponseTable,
    commands::{birthday::BirthdayPrivacy, sit::JouchOrientation},
    db::{Db, UserKey},
    ShuttleItemsContainer,
};
use anyhow::anyhow;
use anyhow::bail;
use chrono::{DateTime, FixedOffset};
use mongodb::bson::{doc, to_bson};
use ron::{de::from_bytes,ser::{to_string_pretty, PrettyConfig}};
use serde::{Deserialize, Serialize};
use serenity::{all::{
    ButtonStyle, CommandInteraction, Context, CreateActionRow, CreateButton,
    CreateInteractionResponse, CreateInteractionResponseFollowup, CreateInteractionResponseMessage,
    EditMessage, GuildId, ResolvedValue, UserId,
}, builder::CreateAttachment};
use std::collections::HashMap;
use tracing::debug;

// only the canned_response_table & nick_interval are actually imported, 
// but the other members exist for legacy reasons; they are handled with shuttle secrets.
#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub token: String,
    #[serde(default)]
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

pub async fn migrate(ctx: &Context, command: &CommandInteraction) -> anyhow::Result<()> {
    if command.data.options.len() == 0 {
        // If no arguments are supplied, we generate files that can be uploaded later
        let data = ctx.data.read().await;
        let db = data
            .get::<Db>()
            .ok_or(anyhow!("Unable to get database"))?;
        
        let guilds = db.get_guilds().await?;
        
        let mut db_data: DbData = DbData::new();
        
        for guild in guilds {
            let guild_data_db = db.read_guild(guild).await?.unwrap();
            let mut guild_data = GuildData {
                users: HashMap::new(),
                birthday_announce_channel: guild_data_db.birthday_announce_channel,
                birthday_announce_when_none: guild_data_db.birthday_announce_when_none,
                canned_response_table: guild_data_db.canned_response_table,
                jouch_orientation: guild_data_db.jouch_orientation,
            };
            db.foreach(guild, |user_data| {
                guild_data.users.insert(user_data._id.user.into(), UserData {
                    birthday: user_data.birthday,
                    birthday_privacy: user_data.birthday_privacy,
                    auto_nick: user_data.auto_nick.clone(),
                    sit_count: user_data.sit_count.map(|u| u as u64),
                    flip_count: user_data.flip_count.map(|u| u as u64),
                });
            }).await?;
            
            db_data.insert(guild.into(), guild_data);
        };
        
        let out_db = to_string_pretty(&db_data, PrettyConfig::default())?;
        
        let config = data
            .get::<crate::config::Config>()
            .ok_or(anyhow!("Unable to get config"))?;
        
        let out_cfg = to_string_pretty(config, PrettyConfig::default())?;

        command
            .create_followup(
                &ctx,
                CreateInteractionResponseFollowup::new()
                    .add_file(CreateAttachment::bytes(out_db, "jouch_db.ron"))
                    .add_file(CreateAttachment::bytes(out_cfg, "config.ron")),
            )
            .await?;
        
        Ok(())
    } else {
        let mut msg = command
            .create_followup(
                &ctx,
                CreateInteractionResponseFollowup::new()
                    .content("Are you sure? This will overwrite any existing data.")
                    .ephemeral(true)
                    .components(vec![CreateActionRow::Buttons(vec![
                        CreateButton::new("migrate_confirm")
                            .style(ButtonStyle::Danger)
                            .label("Import"),
                        CreateButton::new("migrate_cancel")
                            .style(ButtonStyle::Secondary)
                            .label("Cancel"),
                    ])]),
            )
            .await?;
        // TODO - should this timeout?
        if let Some(interaction) = msg.await_component_interaction(&ctx).await {
            let confirmed = interaction.data.custom_id == "migrate_confirm";
            interaction
                .create_response(
                    &ctx,
                    CreateInteractionResponse::UpdateMessage(
                        CreateInteractionResponseMessage::new()
                            .content(if confirmed {
                                "Proccessing..."
                            } else {
                                "Canceled."
                            })
                            .components(vec![]),
                    ),
                )
                .await?;
    
            if !confirmed {
                return Ok(());
            }
        }
    
        for arg in &command.data.options() {
            if let ResolvedValue::Attachment(attachment) = arg.value {
                let data = attachment.download().await?;
                match arg.name {
                    "jouch_db" => {
                        let db_data: DbData = from_bytes(&data)?;
    
                        let mut data = ctx.data.write().await;
                        let db = data
                            .get_mut::<Db>()
                            .ok_or(anyhow!("Unable to get database"))?;
    
                        for (guild_id, guild_data) in &db_data {
                            let guild_id = GuildId::from(*guild_id);
                            db.update_guild(guild_id, doc!{"$set": {
                                "birthday_announce_channel": to_bson(&guild_data.birthday_announce_channel)?,
                                "birthday_announce_when_none": to_bson(&guild_data.birthday_announce_when_none)?,
                                "canned_response_table": to_bson(&guild_data.canned_response_table)?,
                                "jouch_orientation": to_bson(&guild_data.jouch_orientation)?,
                            }}).await?;
    
                            for (user_id, user_data) in &guild_data.users {
                                db.update(
                                    &UserKey {
                                        user: UserId::from(*user_id),
                                        guild: guild_id,
                                    },
                                    doc! {"$set": {
                                        "birthday": to_bson(&user_data.birthday)?,
                                        "birthday_privacy": to_bson(&user_data.birthday_privacy)?,
                                        "auto_nick": to_bson(&user_data.auto_nick)?,
                                        "sit_count": to_bson(&user_data.sit_count)?,
                                        "flip_count": to_bson(&user_data.flip_count)?,
                                    }},
                                )
                                .await?;
                            }
                        }
                    }
                    "config" => {
                        let old_config: Config = from_bytes(&data)?;
    
                        let mut data = ctx.data.write().await;
    
                        {
                            let config = data
                                .get_mut::<crate::config::Config>()
                                .ok_or(anyhow!("Unable to get config"))?;
    
                            config.canned_response_table = old_config.canned_response_table;
                            config.nick_interval = old_config.nick_interval;
                        }
    
                        let config = data
                            .get::<crate::config::Config>()
                            .ok_or(anyhow!("Unable to get config"))?;
                        let shuttle_items = data
                            .get::<ShuttleItemsContainer>()
                            .ok_or(anyhow!("Unable to get ShuttleItemsContainer!"))?;
    
                        debug!("Updated Config to: {:#?}", config);
    
                        config.save(&shuttle_items.persist)?;
    
                        debug!(
                            "Config as saved in shuttle persisted storage: {:#?}",
                            crate::config::Config::load(&shuttle_items.persist)
                        );
                    }
                    _ => bail!("option {} not recognized!", arg.name),
                }
            }
        }
        msg.edit(&ctx, EditMessage::new().content("Complete."))
            .await?;
        Ok(())
    }
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
    pub users: HashMap<u64, UserData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub birthday_announce_channel: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub birthday_announce_when_none: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canned_response_table: Option<ResponseTable>,
    #[serde(default)]
    pub jouch_orientation: JouchOrientation,
}
