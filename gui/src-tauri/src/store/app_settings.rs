use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::db::*;
use super::util::*;

/// Desktop-app preferences stored locally in the GUI database. These are
/// distinct from the agent's own configuration (models/providers/auth).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    /// Auto-approve every incoming permission request without prompting.
    pub auto_approve: bool,
    /// Model identifiers (`provider/id`) hidden from the model picker.
    pub hidden_models: Vec<String>,
    /// Remote control: whether it should be running.
    pub remote_enabled: bool,
    /// Remote control: pairing id (isolation unit / subject prefix).
    pub remote_pair_id: String,
    /// Remote control: NATS **client-port** URL the GUI backend connects to
    /// (e.g. `nats://localhost:4222` locally, or the online relay later). Not
    /// the browser ws:// port.
    pub remote_nats_url: String,
    /// Show the model's thinking/reasoning content in the chat. Off by default.
    pub show_thinking: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAppSettingsInput {
    pub auto_approve: Option<bool>,
    pub hidden_models: Option<Vec<String>>,
    pub remote_enabled: Option<bool>,
    pub remote_pair_id: Option<String>,
    pub remote_nats_url: Option<String>,
    pub show_thinking: Option<bool>,
}

const KEY_AUTO_APPROVE: &str = "auto_approve";
const KEY_HIDDEN_MODELS: &str = "hidden_models";
const KEY_REMOTE_ENABLED: &str = "remote_enabled";
const KEY_REMOTE_PAIR_ID: &str = "remote_pair_id";
const KEY_REMOTE_NATS_URL: &str = "remote_nats_url";
const KEY_SHOW_THINKING: &str = "show_thinking";
const DEFAULT_REMOTE_PAIR_ID: &str = "DEVPAIR";
const DEFAULT_REMOTE_NATS_URL: &str = "nats://localhost:4222";

pub fn get_app_settings() -> Result<AppSettings, crate::AppError> {
    let conn = connect()?;
    read_app_settings(&conn)
}

pub fn update_app_settings(input: UpdateAppSettingsInput) -> Result<AppSettings, crate::AppError> {
    let conn = connect()?;
    let now = now_millis();

    if let Some(auto_approve) = input.auto_approve {
        write_value(
            &conn,
            KEY_AUTO_APPROVE,
            if auto_approve { "true" } else { "false" },
            now,
        )?;
    }
    if let Some(hidden_models) = input.hidden_models {
        let json = serde_json::to_string(&hidden_models)?;
        write_value(&conn, KEY_HIDDEN_MODELS, &json, now)?;
    }
    if let Some(remote_enabled) = input.remote_enabled {
        write_value(
            &conn,
            KEY_REMOTE_ENABLED,
            if remote_enabled { "true" } else { "false" },
            now,
        )?;
    }
    if let Some(remote_pair_id) = input.remote_pair_id {
        write_value(&conn, KEY_REMOTE_PAIR_ID, &remote_pair_id, now)?;
    }
    if let Some(remote_nats_url) = input.remote_nats_url {
        write_value(&conn, KEY_REMOTE_NATS_URL, &remote_nats_url, now)?;
    }
    if let Some(show_thinking) = input.show_thinking {
        write_value(
            &conn,
            KEY_SHOW_THINKING,
            if show_thinking { "true" } else { "false" },
            now,
        )?;
    }

    read_app_settings(&conn)
}

fn read_app_settings(conn: &Connection) -> Result<AppSettings, crate::AppError> {
    let auto_approve = read_value(conn, KEY_AUTO_APPROVE)?
        .map(|value| value == "true")
        .unwrap_or(false);
    let hidden_models = read_value(conn, KEY_HIDDEN_MODELS)?
        .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
        .unwrap_or_default();
    let remote_enabled = read_value(conn, KEY_REMOTE_ENABLED)?
        .map(|value| value == "true")
        .unwrap_or(false);
    let remote_pair_id = read_value(conn, KEY_REMOTE_PAIR_ID)?
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_REMOTE_PAIR_ID.to_string());
    let remote_nats_url = read_value(conn, KEY_REMOTE_NATS_URL)?
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_REMOTE_NATS_URL.to_string());
    let show_thinking = read_value(conn, KEY_SHOW_THINKING)?
        .map(|value| value == "true")
        .unwrap_or(false);
    Ok(AppSettings {
        auto_approve,
        hidden_models,
        remote_enabled,
        remote_pair_id,
        remote_nats_url,
        show_thinking,
    })
}

fn read_value(conn: &Connection, key: &str) -> Result<Option<String>, crate::AppError> {
    conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
    .map_err(crate::AppError::from)
}

fn write_value(conn: &Connection, key: &str, value: &str, now: i64) -> Result<(), crate::AppError> {
    conn.execute(
        "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![key, value, now],
    )?;
    Ok(())
}
