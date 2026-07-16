use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::db::*;
use super::util::*;

/// Desktop-app preferences stored locally in the GUI database. These are
/// distinct from the agent's own configuration (models/providers/auth).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    /// Approval tier: `"off"` (fully open, default), `"manual"` (ask), or
    /// `"sandbox"` (macOS Seatbelt wraps shell commands; tools ask).
    pub approval_tier: String,
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
    /// Show the model's thinking/reasoning content in the chat. On by default.
    pub show_thinking: bool,
    /// Silently upgrade installed skills to their latest catalogue version on
    /// app open (and immediately when toggled on). Off by default.
    pub auto_upgrade_skills: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAppSettingsInput {
    pub approval_tier: Option<String>,
    pub hidden_models: Option<Vec<String>>,
    pub remote_enabled: Option<bool>,
    pub remote_pair_id: Option<String>,
    pub remote_nats_url: Option<String>,
    pub show_thinking: Option<bool>,
    pub auto_upgrade_skills: Option<bool>,
}

const KEY_APPROVAL_TIER: &str = "approval_tier";
const KEY_HIDDEN_MODELS: &str = "hidden_models";
const KEY_REMOTE_ENABLED: &str = "remote_enabled";
const KEY_REMOTE_PAIR_ID: &str = "remote_pair_id";
const KEY_REMOTE_NATS_URL: &str = "remote_nats_url";
const KEY_SHOW_THINKING: &str = "show_thinking";
const KEY_AUTO_UPGRADE_SKILLS: &str = "auto_upgrade_skills";
const DEFAULT_REMOTE_PAIR_ID: &str = "DEVPAIR";
const DEFAULT_REMOTE_NATS_URL: &str = "nats://localhost:4222";

pub fn get_app_settings() -> Result<AppSettings, crate::AppError> {
    let conn = connect()?;
    read_app_settings(&conn)
}

pub fn update_app_settings(input: UpdateAppSettingsInput) -> Result<AppSettings, crate::AppError> {
    let mut conn = connect()?;
    let tx = conn.transaction()?;
    let now = now_millis();

    if let Some(approval_tier) = input.approval_tier {
        write_value(&tx, KEY_APPROVAL_TIER, &normalize_tier(&approval_tier), now)?;
    }
    if let Some(hidden_models) = input.hidden_models {
        let json = serde_json::to_string(&hidden_models)?;
        write_value(&tx, KEY_HIDDEN_MODELS, &json, now)?;
    }
    if let Some(remote_enabled) = input.remote_enabled {
        write_value(
            &tx,
            KEY_REMOTE_ENABLED,
            if remote_enabled { "true" } else { "false" },
            now,
        )?;
    }
    if let Some(remote_pair_id) = input.remote_pair_id {
        write_value(&tx, KEY_REMOTE_PAIR_ID, &remote_pair_id, now)?;
    }
    if let Some(remote_nats_url) = input.remote_nats_url {
        write_value(&tx, KEY_REMOTE_NATS_URL, &remote_nats_url, now)?;
    }
    if let Some(show_thinking) = input.show_thinking {
        write_value(
            &tx,
            KEY_SHOW_THINKING,
            if show_thinking { "true" } else { "false" },
            now,
        )?;
    }
    if let Some(auto_upgrade_skills) = input.auto_upgrade_skills {
        write_value(
            &tx,
            KEY_AUTO_UPGRADE_SKILLS,
            if auto_upgrade_skills { "true" } else { "false" },
            now,
        )?;
    }

    let settings = read_app_settings(&tx)?;
    tx.commit()?;
    Ok(settings)
}

fn read_app_settings(conn: &Connection) -> Result<AppSettings, crate::AppError> {
    let approval_tier = read_value(conn, KEY_APPROVAL_TIER)?
        .map(|value| normalize_tier(&value))
        .unwrap_or_else(|| "off".to_string());
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
        .unwrap_or(true);
    let auto_upgrade_skills = read_value(conn, KEY_AUTO_UPGRADE_SKILLS)?
        .map(|value| value == "true")
        .unwrap_or(false);
    Ok(AppSettings {
        approval_tier,
        hidden_models,
        remote_enabled,
        remote_pair_id,
        remote_nats_url,
        show_thinking,
        auto_upgrade_skills,
    })
}

/// Clamp a tier string to the known set; anything unknown falls back to the
/// default `"off"`.
fn normalize_tier(value: &str) -> String {
    match value {
        "off" | "sandbox" | "manual" => value.to_string(),
        _ => "off".to_string(),
    }
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
