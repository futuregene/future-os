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
    /// Show the model's thinking/reasoning content in the chat. On by default.
    pub show_thinking: bool,
    /// Silently upgrade installed skills to their latest catalogue version on
    /// app open (and immediately when toggled on). On by default.
    pub auto_upgrade_skills: bool,
    /// Auto-connect the single paired remote device on app launch. Off by
    /// default. Remote control is a dev-only feature, so this is only consulted
    /// on non-release builds (see the startup auto-connect in `lib.rs`).
    pub auto_connect_remote: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAppSettingsInput {
    pub approval_tier: Option<String>,
    pub hidden_models: Option<Vec<String>>,
    pub show_thinking: Option<bool>,
    pub auto_upgrade_skills: Option<bool>,
    pub auto_connect_remote: Option<bool>,
}

const KEY_APPROVAL_TIER: &str = "approval_tier";
const KEY_HIDDEN_MODELS: &str = "hidden_models";
const KEY_SHOW_THINKING: &str = "show_thinking";
const KEY_AUTO_UPGRADE_SKILLS: &str = "auto_upgrade_skills";
const KEY_AUTO_CONNECT_REMOTE: &str = "auto_connect_remote";

pub fn get_app_settings() -> Result<AppSettings, crate::AppError> {
    let conn = connect()?;
    read_app_settings(&conn)
}

pub fn update_app_settings(input: UpdateAppSettingsInput) -> Result<AppSettings, crate::AppError> {
    let mut conn = connect()?;
    let tx = conn.transaction()?;
    let now = now_millis();

    if let Some(approval_tier) = input.approval_tier {
        let tier = normalize_tier(&approval_tier);
        write_value(&tx, KEY_APPROVAL_TIER, &tier, now)?;
    }
    if let Some(hidden_models) = input.hidden_models {
        let json = serde_json::to_string(&hidden_models)?;
        write_value(&tx, KEY_HIDDEN_MODELS, &json, now)?;
    }
    if let Some(show_thinking) = input.show_thinking {
        let value = if show_thinking { "true" } else { "false" };
        write_value(&tx, KEY_SHOW_THINKING, value, now)?;
    }
    if let Some(auto_upgrade_skills) = input.auto_upgrade_skills {
        let value = if auto_upgrade_skills { "true" } else { "false" };
        write_value(&tx, KEY_AUTO_UPGRADE_SKILLS, value, now)?;
    }
    if let Some(auto_connect_remote) = input.auto_connect_remote {
        let value = if auto_connect_remote { "true" } else { "false" };
        write_value(&tx, KEY_AUTO_CONNECT_REMOTE, value, now)?;
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
    let show_thinking = read_value(conn, KEY_SHOW_THINKING)?
        .map(|value| value == "true")
        .unwrap_or(true);
    let auto_upgrade_skills = read_value(conn, KEY_AUTO_UPGRADE_SKILLS)?
        .map(|value| value == "true")
        .unwrap_or(true); // On by default — keeps skills current without manual intervention.
    let auto_connect_remote = read_value(conn, KEY_AUTO_CONNECT_REMOTE)?
        .map(|value| value == "true")
        .unwrap_or(false); // Off by default — remote auto-connect is opt-in.
    Ok(AppSettings {
        approval_tier,
        hidden_models,
        show_thinking,
        auto_upgrade_skills,
        auto_connect_remote,
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

/// Upsert one settings row. Hoisted so the call site stays a single line —
/// rustfmt's multi-line `)?;` layout strands the `?` error edge on its own
/// (uncoverable) line.
const UPSERT_SQL: &str = "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3)
     ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at";

fn write_value(conn: &Connection, key: &str, value: &str, now: i64) -> Result<(), crate::AppError> {
    conn.execute(UPSERT_SQL, params![key, value, now])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::db::test_support::{guarded_conn, memory_conn};

    fn full_input() -> UpdateAppSettingsInput {
        UpdateAppSettingsInput {
            approval_tier: Some("sandbox".to_string()),
            hidden_models: Some(vec!["openai/gpt-x".to_string()]),
            show_thinking: Some(false),
            auto_upgrade_skills: Some(false),
            auto_connect_remote: Some(true),
        }
    }

    #[test]
    fn defaults_apply_on_a_fresh_database() {
        let (_home, conn) = guarded_conn("settings_defaults");
        drop(conn);
        let settings = get_app_settings().expect("get settings");
        assert_eq!(settings.approval_tier, "off");
        assert!(settings.hidden_models.is_empty());
        assert!(settings.show_thinking);
        assert!(settings.auto_upgrade_skills);
        assert!(!settings.auto_connect_remote);
    }

    #[test]
    fn update_round_trips_every_field() {
        let (_home, conn) = guarded_conn("settings_update");
        drop(conn);

        let updated = update_app_settings(full_input()).expect("update");
        assert_eq!(updated.approval_tier, "sandbox");
        assert_eq!(updated.hidden_models, vec!["openai/gpt-x".to_string()]);
        assert!(!updated.show_thinking);
        assert!(!updated.auto_upgrade_skills);
        assert!(updated.auto_connect_remote);

        // Persisted across connections.
        assert_eq!(
            get_app_settings().expect("get").approval_tier,
            "sandbox"
        );
    }

    #[test]
    fn update_normalizes_an_unknown_tier() {
        let (_home, conn) = guarded_conn("settings_tier");
        drop(conn);
        let updated = update_app_settings(UpdateAppSettingsInput {
            approval_tier: Some("permissive".to_string()),
            hidden_models: None,
            show_thinking: None,
            auto_upgrade_skills: None,
            auto_connect_remote: None,
        })
        .expect("update");
        assert_eq!(updated.approval_tier, "off");
    }

    #[test]
    fn read_repairs_corrupt_stored_values() {
        let conn = memory_conn();
        // An unknown tier string normalizes to the default…
        write_value(&conn, KEY_APPROVAL_TIER, "weird", 1).expect("write tier");
        // …corrupt JSON decodes to the empty list…
        write_value(&conn, KEY_HIDDEN_MODELS, "{not json", 1).expect("write models");
        // …and non-"true" booleans read as false.
        write_value(&conn, KEY_SHOW_THINKING, "yes", 1).expect("write thinking");
        write_value(&conn, KEY_AUTO_UPGRADE_SKILLS, "0", 1).expect("write upgrade");
        write_value(&conn, KEY_AUTO_CONNECT_REMOTE, "true", 1).expect("write remote");

        let settings = read_app_settings(&conn).expect("read");
        assert_eq!(settings.approval_tier, "off");
        assert!(settings.hidden_models.is_empty());
        assert!(!settings.show_thinking);
        assert!(!settings.auto_upgrade_skills);
        assert!(settings.auto_connect_remote);
    }

    #[test]
    fn update_with_all_fields_absent_is_a_noop() {
        let (_home, conn) = guarded_conn("settings_noop");
        drop(conn);
        let settings = update_app_settings(UpdateAppSettingsInput {
            approval_tier: None,
            hidden_models: None,
            show_thinking: None,
            auto_upgrade_skills: None,
            auto_connect_remote: None,
        })
        .expect("noop update");
        assert_eq!(settings.approval_tier, "off", "defaults survive a noop");
    }

    #[test]
    fn normalize_tier_keeps_known_values() {
        for tier in ["off", "sandbox", "manual"] {
            assert_eq!(normalize_tier(tier), tier);
        }
        assert_eq!(normalize_tier("anything-else"), "off");
    }
}
