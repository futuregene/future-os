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
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAppSettingsInput {
    pub auto_approve: Option<bool>,
    pub hidden_models: Option<Vec<String>>,
}

const KEY_AUTO_APPROVE: &str = "auto_approve";
const KEY_HIDDEN_MODELS: &str = "hidden_models";

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

    read_app_settings(&conn)
}

fn read_app_settings(conn: &Connection) -> Result<AppSettings, crate::AppError> {
    let auto_approve = read_value(conn, KEY_AUTO_APPROVE)?
        .map(|value| value == "true")
        .unwrap_or(false);
    let hidden_models = read_value(conn, KEY_HIDDEN_MODELS)?
        .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
        .unwrap_or_default();
    Ok(AppSettings {
        auto_approve,
        hidden_models,
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
