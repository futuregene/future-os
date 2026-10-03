//! Desktop app settings: the desktop-specific half of the settings document.
//!
//! The schema itself — the settable keys, their defaults, value validation and
//! the row helpers — lives in the `future-app-settings` crate, shared with
//! `future desktop settings` in the CLI so the two writers cannot drift apart.
//! This module owns what is desktop-only: the connection pool, the device
//! identity, and the change notifications published after a commit.

use rusqlite::Connection;

use super::db::*;
use super::util::now_millis;
use future_app_settings::KEY_DEVICE_ID;

pub use future_app_settings::{AppSettings, UpdateAppSettingsInput};

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
    future_app_settings::ensure_table(&conn)?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if let Some(existing) = future_app_settings::read_value(&tx, KEY_DEVICE_ID)?
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        tx.commit()?;
        return Ok(existing);
    }
    future_app_settings::write_value(&tx, KEY_DEVICE_ID, candidate, now_millis())?;
    tx.commit()?;
    Ok(candidate.to_string())
}

pub(super) fn read_device_id(conn: &Connection) -> Result<Option<String>, crate::AppError> {
    Ok(future_app_settings::read_value(conn, KEY_DEVICE_ID)?)
}

pub(super) fn restore_device_id(
    conn: &Connection,
    device_id: Option<&str>,
) -> Result<(), crate::AppError> {
    if let Some(device_id) = device_id {
        future_app_settings::write_value(conn, KEY_DEVICE_ID, device_id, now_millis())?;
    }
    Ok(())
}

pub fn get_app_settings() -> Result<AppSettings, crate::AppError> {
    let conn = connect()?;
    Ok(future_app_settings::read(&conn)?)
}

pub fn update_app_settings(input: UpdateAppSettingsInput) -> Result<AppSettings, crate::AppError> {
    let mut conn = connect()?;
    let tx = conn.transaction()?;
    let now = now_millis();
    let model_visibility_changed = input.hidden_models.is_some();

    let settings = future_app_settings::apply(&tx, &input, now)?;
    tx.commit()?;
    crate::agent_events::publish_invalidation("app_settings_changed");
    // Notify paired clients only after commit: their next model read must see
    // the new visibility. Reconnect also rereads the catalogue if this is lost.
    if model_visibility_changed {
        crate::agent_events::publish_event(
            "_global",
            "model_visibility_changed",
            "{}",
            "",
            -1,
            0,
            &format!("model-visibility-{now}"),
            "",
            -1,
            -1,
        );
    }
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::db::test_support::{guarded_conn, memory_conn};
    use future_app_settings::{
        read as read_settings, write_value, KEY_APPROVAL_TIER, KEY_AUTO_CONNECT_REMOTE,
        KEY_AUTO_TITLE_FIRST_TURN, KEY_AUTO_UPGRADE_SKILLS, KEY_BELL_ON_COMPLETE,
        KEY_COMMUNITY_EDITION, KEY_HIDDEN_MODELS,
    };

    fn full_input() -> UpdateAppSettingsInput {
        UpdateAppSettingsInput {
            approval_tier: Some("sandbox".to_string()),
            hidden_models: Some(vec!["openai/gpt-x".to_string()]),
            auto_upgrade_skills: Some(false),
            auto_connect_remote: Some(true),
            skill_guide_dismissed: Some(true),
            skill_intro_dismissed: Some(true),
            bell_on_complete: None,
            auto_title_first_turn: None,
            title_language: None,
            community_edition: Some(true),
            skill_recommend: None,
        }
    }

    #[test]
    fn defaults_apply_on_a_fresh_database() {
        let (_home, conn) = guarded_conn("settings_defaults");
        drop(conn);
        let settings = get_app_settings().expect("get settings");
        assert_eq!(settings.approval_tier, "off");
        assert!(settings.hidden_models.is_empty());
        assert!(settings.auto_upgrade_skills);
        assert!(!settings.auto_connect_remote);
        assert!(!settings.skill_guide_dismissed);
        assert!(!settings.skill_intro_dismissed);
        assert!(!settings.community_edition);
        // PRD v1.6 §3: recommendation is on unless the user opts out.
        assert!(settings.skill_recommend);
    }

    #[test]
    fn retired_thinking_preference_is_ignored_and_not_returned() {
        let (_home, conn) = guarded_conn("settings_retired_thinking");
        for value in ["false", "true", "invalid"] {
            // Existing installations can retain this unused key; removing a
            // preference does not require a destructive database migration.
            write_value(&conn, "show_thinking", value, 1).expect("legacy preference");
            let settings = serde_json::to_value(get_app_settings().expect("read settings"))
                .expect("serialize settings");
            assert!(settings.get("showThinking").is_none());

            let input = serde_json::from_value(serde_json::json!({
                "showThinking": false,
                "bellOnComplete": false
            }))
            .expect("legacy input remains readable");
            let updated = update_app_settings(input).expect("update settings");
            assert!(!updated.bell_on_complete);
            assert_eq!(
                future_app_settings::read_value(&conn, "show_thinking").expect("legacy key"),
                Some(value.to_string()),
                "the retired key must not be written"
            );
            assert!(serde_json::to_value(updated)
                .expect("serialize update")
                .get("showThinking")
                .is_none());
        }
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
        assert!(!updated.auto_upgrade_skills);
        assert!(updated.auto_connect_remote);
        assert!(updated.skill_guide_dismissed);
        assert!(updated.skill_intro_dismissed);
        assert!(updated.community_edition);

        // Persisted across connections.
        assert_eq!(get_app_settings().expect("get").approval_tier, "sandbox");
    }

    #[test]
    fn update_normalizes_an_unknown_tier() {
        let (_home, conn) = guarded_conn("settings_tier");
        drop(conn);
        let updated = update_app_settings(UpdateAppSettingsInput {
            approval_tier: Some("permissive".to_string()),
            ..Default::default()
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
    fn first_turn_title_defaults_on_and_persists_updates() {
        let (_home, conn) = guarded_conn("settings_first_turn_title");
        assert!(get_app_settings().expect("defaults").auto_title_first_turn);
        assert_eq!(get_app_settings().expect("defaults").title_language, "en");
        for enabled in [false, true] {
            write_value(
                &conn,
                KEY_AUTO_TITLE_FIRST_TURN,
                if enabled { "true" } else { "false" },
                1,
            )
            .expect("legacy preference");
            assert_eq!(
                get_app_settings()
                    .expect("legacy preference")
                    .auto_title_first_turn,
                enabled
            );
            // Updating an unrelated setting must not overwrite a saved choice.
            assert_eq!(
                update_app_settings(UpdateAppSettingsInput {
                    title_language: Some("zh".into()),
                    ..Default::default()
                })
                .expect("unrelated update")
                .auto_title_first_turn,
                enabled
            );
        }
        drop(conn);
        for enabled in [true, false] {
            let updated = update_app_settings(UpdateAppSettingsInput {
                auto_title_first_turn: Some(enabled),
                title_language: Some("zh".into()),
                ..Default::default()
            })
            .expect("update");
            assert_eq!(updated.auto_title_first_turn, enabled);
            assert_eq!(get_app_settings().expect("reload").title_language, "zh");
            assert_eq!(
                get_app_settings().expect("reload").auto_title_first_turn,
                enabled
            );
            assert_eq!(
                serde_json::to_value(updated).expect("serialize")["autoTitleFirstTurn"],
                enabled
            );
        }
    }

    #[test]
    fn read_repairs_corrupt_stored_values() {
        let conn = memory_conn();
        // An unknown tier string normalizes to the default…
        write_value(&conn, KEY_APPROVAL_TIER, "weird", 1).expect("write tier");
        // …corrupt JSON decodes to the empty list…
        write_value(&conn, KEY_HIDDEN_MODELS, "{not json", 1).expect("write models");
        // …and non-"true" booleans read as false.
        write_value(&conn, KEY_AUTO_UPGRADE_SKILLS, "0", 1).expect("write upgrade");
        write_value(&conn, KEY_AUTO_CONNECT_REMOTE, "true", 1).expect("write remote");
        write_value(&conn, KEY_BELL_ON_COMPLETE, "yes", 1).expect("write bell");
        write_value(&conn, KEY_COMMUNITY_EDITION, "true", 1).expect("write community edition");

        let settings = read_settings(&conn).expect("read");
        assert_eq!(settings.approval_tier, "off");
        assert!(settings.hidden_models.is_empty());
        assert!(!settings.auto_upgrade_skills);
        assert!(settings.auto_connect_remote);
        assert!(!settings.bell_on_complete);
        assert!(settings.community_edition);
    }

    #[test]
    fn update_with_all_fields_absent_is_a_noop() {
        let (_home, conn) = guarded_conn("settings_noop");
        drop(conn);
        let settings = update_app_settings(UpdateAppSettingsInput::default()).expect("noop update");
        assert_eq!(settings.approval_tier, "off", "defaults survive a noop");
    }

    #[test]
    fn update_records_false_dismissed_flags() {
        let (_home, conn) = guarded_conn("settings_dismissed_false");
        drop(conn);
        let updated = update_app_settings(UpdateAppSettingsInput {
            skill_guide_dismissed: Some(false),
            skill_intro_dismissed: Some(false),
            ..Default::default()
        })
        .expect("update");
        assert!(!updated.skill_guide_dismissed);
        assert!(!updated.skill_intro_dismissed);
    }

    #[test]
    fn auto_title_first_turn_round_trips_and_unknown_languages_are_refused() {
        let (_home, conn) = guarded_conn("settings_title_language");
        drop(conn);

        let settings = update_app_settings(UpdateAppSettingsInput {
            auto_title_first_turn: Some(false),
            ..Default::default()
        })
        .expect("write auto-title preference");
        assert!(!settings.auto_title_first_turn);
        assert!(!get_app_settings().expect("read back").auto_title_first_turn);

        // Only the languages the model prompt itself supports may be stored;
        // anything else would silently fall back to a mixed-language title.
        let error = update_app_settings(UpdateAppSettingsInput {
            title_language: Some("fr".to_string()),
            ..Default::default()
        })
        .expect_err("an unsupported title language must be refused")
        .to_string();
        assert!(error.contains("Unsupported title language"), "{error}");
        assert_eq!(get_app_settings().expect("read back").title_language, "en");
    }

    #[test]
    fn a_refused_language_rolls_back_earlier_fields() {
        let (_home, conn) = guarded_conn("settings_rollback");
        drop(conn);
        // The same transaction wraps every field, so a rejected language must
        // undo a bell change written earlier in the same update.
        assert!(update_app_settings(UpdateAppSettingsInput {
            bell_on_complete: Some(false),
            skill_intro_dismissed: Some(true),
            title_language: Some("fr".to_string()),
            ..Default::default()
        })
        .is_err());
        let settings = get_app_settings().expect("read back");
        assert!(settings.bell_on_complete, "the bell write must roll back");
        assert!(
            !settings.skill_intro_dismissed,
            "the dismissed flag must roll back too"
        );
        assert_eq!(settings.title_language, "en");
    }

    #[test]
    fn the_stored_title_language_key_is_the_legacy_row_name() {
        // The API name is autoTitleFirstTurn, but the stored row keeps its
        // original name so existing installations keep their opt-in.
        assert_eq!(KEY_AUTO_TITLE_FIRST_TURN, "auto_compact_first_turn");
    }
}
