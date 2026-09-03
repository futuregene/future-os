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
    /// `"sandbox"` (the available OS sandbox wraps shell commands; tools ask).
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
    /// The user closed the skill-onboarding banner on the new-conversation
    /// screen. Off by default (the banner shows until dismissed).
    pub skill_guide_dismissed: bool,
    /// The user acknowledged the Skills nav-entry intro bubble (去看看 /
    /// 知道了 / click-outside). Off by default; once set, the bubble and its
    /// blue dot never show again (until app data is wiped).
    pub skill_intro_dismissed: bool,
    /// Play a completion bell + request window attention when an agent run
    /// finishes. On by default.
    pub bell_on_complete: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAppSettingsInput {
    pub approval_tier: Option<String>,
    pub hidden_models: Option<Vec<String>>,
    pub show_thinking: Option<bool>,
    pub auto_upgrade_skills: Option<bool>,
    pub auto_connect_remote: Option<bool>,
    pub skill_guide_dismissed: Option<bool>,
    pub skill_intro_dismissed: Option<bool>,
    pub bell_on_complete: Option<bool>,
}

const KEY_APPROVAL_TIER: &str = "approval_tier";
const KEY_HIDDEN_MODELS: &str = "hidden_models";
const KEY_SHOW_THINKING: &str = "show_thinking";
const KEY_AUTO_UPGRADE_SKILLS: &str = "auto_upgrade_skills";
const KEY_AUTO_CONNECT_REMOTE: &str = "auto_connect_remote";
const KEY_SKILL_GUIDE_DISMISSED: &str = "skill_guide_dismissed";
const KEY_SKILL_INTRO_DISMISSED: &str = "skill_intro_dismissed";
const KEY_BELL_ON_COMPLETE: &str = "bell_on_complete";
const KEY_DEVICE_ID: &str = "device_id";

/// Atomically install the Desktop-wide device identity. The caller supplies a
/// legacy or freshly generated candidate, but SQLite decides the winner when
/// multiple Desktop processes start for the first time concurrently.
pub fn get_or_create_device_id(candidate: &str) -> Result<String, crate::AppError> {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return Err("device id cannot be empty".to_string().into());
    }
    let mut conn = connect()?;
    // Device identity is needed by early control-plane paths (including remote
    // pairing tests and reconnects), so do not require the full application
    // schema initializer to have won the startup race first.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_settings (
             key TEXT PRIMARY KEY,
             value TEXT NOT NULL,
             updated_at INTEGER NOT NULL
         )",
    )?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if let Some(existing) = read_value(&tx, KEY_DEVICE_ID)?
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        tx.commit()?;
        return Ok(existing);
    }
    write_value(&tx, KEY_DEVICE_ID, candidate, now_millis())?;
    tx.commit()?;
    Ok(candidate.to_string())
}

pub(super) fn read_device_id(conn: &Connection) -> Result<Option<String>, crate::AppError> {
    read_value(conn, KEY_DEVICE_ID)
}

pub(super) fn restore_device_id(
    conn: &Connection,
    device_id: Option<&str>,
) -> Result<(), crate::AppError> {
    if let Some(device_id) = device_id {
        write_value(conn, KEY_DEVICE_ID, device_id, now_millis())?;
    }
    Ok(())
}

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
    if let Some(skill_guide_dismissed) = input.skill_guide_dismissed {
        let value = if skill_guide_dismissed {
            "true"
        } else {
            "false"
        };
        write_value(&tx, KEY_SKILL_GUIDE_DISMISSED, value, now)?;
    }
    if let Some(skill_intro_dismissed) = input.skill_intro_dismissed {
        let value = if skill_intro_dismissed {
            "true"
        } else {
            "false"
        };
        write_value(&tx, KEY_SKILL_INTRO_DISMISSED, value, now)?;
    }
    if let Some(bell_on_complete) = input.bell_on_complete {
        let value = if bell_on_complete { "true" } else { "false" };
        write_value(&tx, KEY_BELL_ON_COMPLETE, value, now)?;
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
    let skill_guide_dismissed = read_value(conn, KEY_SKILL_GUIDE_DISMISSED)?
        .map(|value| value == "true")
        .unwrap_or(false); // Off by default — the banner shows until dismissed.
    let skill_intro_dismissed = read_value(conn, KEY_SKILL_INTRO_DISMISSED)?
        .map(|value| value == "true")
        .unwrap_or(false); // Off by default — the bubble shows once until dismissed.
    let bell_on_complete = read_value(conn, KEY_BELL_ON_COMPLETE)?
        .map(|value| value == "true")
        .unwrap_or(true); // On by default — a finished run should get noticed.
    Ok(AppSettings {
        approval_tier,
        hidden_models,
        show_thinking,
        auto_upgrade_skills,
        auto_connect_remote,
        skill_guide_dismissed,
        skill_intro_dismissed,
        bell_on_complete,
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
            skill_guide_dismissed: Some(true),
            skill_intro_dismissed: Some(true),
            bell_on_complete: None,
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
        assert!(!settings.skill_guide_dismissed);
        assert!(!settings.skill_intro_dismissed);
    }

    #[test]
    fn device_identity_is_installed_once_and_reused() {
        let (_home, conn) = guarded_conn("settings_device_id");
        drop(conn);
        assert_eq!(
            get_or_create_device_id("desktop_first").expect("first"),
            "desktop_first"
        );
        assert_eq!(
            get_or_create_device_id("desktop_second").expect("reuse"),
            "desktop_first"
        );
        assert!(get_or_create_device_id("   ").is_err());
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
        assert!(updated.skill_guide_dismissed);
        assert!(updated.skill_intro_dismissed);

        // Persisted across connections.
        assert_eq!(get_app_settings().expect("get").approval_tier, "sandbox");
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
            skill_guide_dismissed: None,
            skill_intro_dismissed: None,
            bell_on_complete: None,
        })
        .expect("update");
        assert_eq!(updated.approval_tier, "off");
    }

    #[test]
    fn bell_on_complete_defaults_on_and_updates_off() {
        let (_home, conn) = guarded_conn("settings_bell");
        drop(conn);
        // Absent → on by default.
        let default = update_app_settings(UpdateAppSettingsInput::default()).expect("noop");
        assert!(default.bell_on_complete);
        // Explicit false → off, and persists across connections.
        let updated = update_app_settings(UpdateAppSettingsInput {
            bell_on_complete: Some(false),
            ..Default::default()
        })
        .expect("update");
        assert!(!updated.bell_on_complete);
        assert!(!get_app_settings().expect("re-read").bell_on_complete);
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
        write_value(&conn, KEY_BELL_ON_COMPLETE, "yes", 1).expect("write bell");

        let settings = read_app_settings(&conn).expect("read");
        assert_eq!(settings.approval_tier, "off");
        assert!(settings.hidden_models.is_empty());
        assert!(!settings.show_thinking);
        assert!(!settings.auto_upgrade_skills);
        assert!(settings.auto_connect_remote);
        assert!(!settings.bell_on_complete);
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
            skill_guide_dismissed: None,
            skill_intro_dismissed: None,
            bell_on_complete: None,
        })
        .expect("noop update");
        assert_eq!(settings.approval_tier, "off", "defaults survive a noop");
    }

    #[test]
    fn update_records_false_dismissed_flags() {
        let (_home, conn) = guarded_conn("settings_dismissed_false");
        drop(conn);
        let updated = update_app_settings(UpdateAppSettingsInput {
            approval_tier: None,
            hidden_models: None,
            show_thinking: None,
            auto_upgrade_skills: None,
            auto_connect_remote: None,
            skill_guide_dismissed: Some(false),
            skill_intro_dismissed: Some(false),
            bell_on_complete: None,
        })
        .expect("update");
        assert!(!updated.skill_guide_dismissed);
        assert!(!updated.skill_intro_dismissed);
    }

    #[test]
    fn normalize_tier_keeps_known_values() {
        for tier in ["off", "sandbox", "manual"] {
            assert_eq!(normalize_tier(tier), tier);
        }
        assert_eq!(normalize_tier("anything-else"), "off");
    }
}
