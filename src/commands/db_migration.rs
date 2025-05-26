use crate::{
    canned_responses::ResponseTable,
    commands::{birthday::BirthdayPrivacy, sit::JouchOrientation},
    db::Db,
};
use anyhow::anyhow;
use anyhow::bail;
use chrono::{DateTime, FixedOffset};
use ron::{
    de::from_bytes,
    ser::{to_string_pretty, PrettyConfig},
};
use serde::{Deserialize, Serialize};
use serenity::{
    all::{
        ButtonStyle, CommandInteraction, Context, CreateActionRow, CreateButton,
        CreateInteractionResponse, CreateInteractionResponseFollowup,
        CreateInteractionResponseMessage, EditMessage, ResolvedValue,
    },
    builder::CreateAttachment,
    json::json,
};
use sqlx::QueryBuilder;
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
    pub nick_interval: i64,
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
        let db = data.get::<Db>().ok_or(anyhow!("Unable to get database"))?;

        let mut guilds = db.get_guilds().await?;
        // It's possible that a guild has users but lacks a row in the guilds table,
        // so we also add any guilds found referenced in the users table
        guilds.extend(db.get_user_guilds(None).await?);

        let mut db_data: DbData = DbData::new();

        for guild in guilds {
            let guild_data_db = db.read_guild(guild).await?.unwrap_or_default();
            let users = db.read_users(guild, "").await?;
            let guild_data = GuildData {
                users: users
                    .iter()
                    .map(|user_data| {
                        (
                            user_data.id.user as u64,
                            UserData {
                                birthday: user_data.birthday,
                                birthday_privacy: user_data.birthday_privacy,
                                auto_nick: user_data.auto_nick.clone(),
                                sit_count: Some(user_data.sit_count as u64),
                                flip_count: Some(user_data.flip_count as u64),
                            },
                        )
                    })
                    .collect(),
                birthday_announce_channel: guild_data_db
                    .birthday_announce_channel
                    .map(|x| x as u64),
                birthday_announce_when_none: guild_data_db.birthday_announce_when_none,
                canned_response_table: guild_data_db.canned_response_table,
                jouch_orientation: guild_data_db.jouch_orientation,
            };

            db_data.insert(guild.into(), guild_data);
        }

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

                        let data = ctx.data.read().await;
                        let db = data
                            .get::<Db>()
                            .ok_or(anyhow!("Unable to get database"))?
                            .pool();

                        sqlx::query("TRUNCATE guilds").execute(db).await?;
                        sqlx::query("TRUNCATE users").execute(db).await?;

                        let mut guilds_insert_query = QueryBuilder::new(
                                "INSERT INTO guilds (id, birthday_announce_channel, birthday_announce_when_none, canned_response_table, jouch_orientation) ");

                        guilds_insert_query.push_values(
                            db_data.iter(),
                            |mut b, (id, guild_data)| {
                                b.push_bind(*id as i64)
                                    .push_bind(
                                        guild_data.birthday_announce_channel.map(|x| x as i64),
                                    )
                                    .push_bind(guild_data.birthday_announce_when_none)
                                    .push_bind(json!(guild_data.canned_response_table))
                                    .push_bind(guild_data.jouch_orientation);
                            },
                        );

                        guilds_insert_query.build().execute(db).await?;

                        let mut users_insert_query = QueryBuilder::new(
                            "INSERT INTO users (user_id, guild_id, birthday, birthday_privacy, auto_nick, sit_count, flip_count) ");

                        for (guild_id, guild_data) in &db_data {
                            // TODO - this could end up getting too big for a single query
                            users_insert_query.push_values(
                                guild_data.users.iter(),
                                |mut b, (user_id, user_data)| {
                                    b.push_bind(*user_id as i64)
                                        .push_bind(*guild_id as i64)
                                        .push_bind(user_data.birthday)
                                        .push_bind(user_data.birthday_privacy)
                                        .push_bind(user_data.auto_nick.clone())
                                        .push_bind(user_data.sit_count.unwrap_or_default() as i64)
                                        .push_bind(user_data.flip_count.unwrap_or_default() as i64);
                                },
                            );

                            users_insert_query.build().execute(db).await?;
                            // leave query ready for the next loop.
                            users_insert_query.reset();
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
                        let db = data.get::<Db>().ok_or(anyhow!("Unable to get database"))?;

                        debug!("Updated Config to: {:#?}", config);

                        config.save(db.pool()).await?;

                        debug!(
                            "Config as saved in shuttle persisted storage: {:#?}",
                            crate::config::Config::load(db.pool()).await
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
