//! The desktop app's local settings document.
//!
//! These preferences live in the `app_settings` table of
//! `~/.future/app/app.db`, which the Tauri desktop app owns. This crate is the
//! single source of truth for them so that both writers — the desktop backend
//! (`desktop/src-tauri/src/store/app_settings.rs`) and
//! `future desktop settings` in the CLI — agree on the keys, their defaults,
//! their validation and the database location. Adding a key here is what makes
//! it reachable from both.
//!
//! Nothing here is the *agent's* configuration: models, providers, auth and the
//! global agent settings document are separate files under `~/.future/agent/`
//! and stay owned by the agent and `future config`.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Desktop-app preferences, as the Tauri command layer serializes them
/// (camelCase).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    /// Approval tier: `"off"` (fully open, default), `"manual"` (ask), or
    /// `"sandbox"` (the available OS sandbox wraps shell commands; tools ask).
    pub approval_tier: String,
    /// Model identifiers (`provider/id`) hidden from the model picker.
    pub hidden_models: Vec<String>,
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
    /// Generate and save a title after the first successful answer, without compaction.
    /// On by default; later turns never trigger this preference.
    pub auto_title_first_turn: bool,
    /// UI language mirrored for title generation when the webview is suspended.
    pub title_language: String,
    /// Use the community-edition UI: Future is configured like another
    /// built-in provider and account/billing details stay out of the footer.
    pub community_edition: bool,
    /// Recommend at most one uninstalled skill when the user sends a message
    /// (PRD v1.6 §3). **On by default**; the user can turn it off in Settings.
    pub skill_recommend: bool,
}

/// A partial update: only `Some` fields are written.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAppSettingsInput {
    pub approval_tier: Option<String>,
    pub hidden_models: Option<Vec<String>>,
    pub auto_upgrade_skills: Option<bool>,
    pub auto_connect_remote: Option<bool>,
    pub skill_guide_dismissed: Option<bool>,
    pub skill_intro_dismissed: Option<bool>,
    pub bell_on_complete: Option<bool>,
    #[serde(alias = "autoCompactFirstTurn")]
    pub auto_title_first_turn: Option<bool>,
    pub title_language: Option<String>,
    pub community_edition: Option<bool>,
    pub skill_recommend: Option<bool>,
}

pub const KEY_APPROVAL_TIER: &str = "approval_tier";
pub const KEY_HIDDEN_MODELS: &str = "hidden_models";
pub const KEY_AUTO_UPGRADE_SKILLS: &str = "auto_upgrade_skills";
pub const KEY_AUTO_CONNECT_REMOTE: &str = "auto_connect_remote";
pub const KEY_SKILL_GUIDE_DISMISSED: &str = "skill_guide_dismissed";
pub const KEY_SKILL_INTRO_DISMISSED: &str = "skill_intro_dismissed";
pub const KEY_BELL_ON_COMPLETE: &str = "bell_on_complete";
// Retain the original stored key so existing opt-ins survive the behavior fix.
// This preference now generates titles only; it never requests compaction.
pub const KEY_AUTO_TITLE_FIRST_TURN: &str = "auto_compact_first_turn";
pub const KEY_TITLE_LANGUAGE: &str = "title_language";
pub const KEY_COMMUNITY_EDITION: &str = "community_edition";
pub const KEY_SKILL_RECOMMEND: &str = "skill_recommend";
pub const KEY_DEVICE_ID: &str = "device_id";

/// Errors from reading, validating or writing the settings document.
#[derive(Debug)]
pub enum Error {
    Database(rusqlite::Error),
    Json(serde_json::Error),
    Io(std::io::Error),
    /// A key this crate does not know, or a value it cannot accept. The message
    /// is written for a user.
    Invalid(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Database(error) => write!(formatter, "{error}"),
            Error::Json(error) => write!(formatter, "{error}"),
            Error::Io(error) => write!(formatter, "{error}"),
            Error::Invalid(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Database(error) => Some(error),
            Error::Json(error) => Some(error),
            Error::Io(error) => Some(error),
            Error::Invalid(_) => None,
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        Error::Database(error)
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Error::Json(error)
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Error::Io(error)
    }
}

/// How a settable key's text form is parsed by `future desktop settings set`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    Bool,
    /// `off` | `manual` | `sandbox`.
    Tier,
    /// `en` | `zh`.
    Language,
    /// A JSON array of model ids, or a comma-separated list.
    Models,
}

impl ValueKind {
    /// A short description of the accepted values, for help output.
    pub fn accepted(self) -> &'static str {
        match self {
            ValueKind::Bool => "true|false",
            ValueKind::Tier => "off|manual|sandbox",
            ValueKind::Language => "en|zh",
            ValueKind::Models => "JSON array or comma-separated model ids",
        }
    }

    fn parse(self, raw: &str) -> Result<serde_json::Value, String> {
        match self {
            ValueKind::Bool => parse_bool(raw).map(|value| serde_json::json!(value)),
            ValueKind::Tier => {
                if !matches!(raw, "off" | "manual" | "sandbox") {
                    return Err(format!(
                        "approvalTier must be one of off, manual, sandbox, got {raw:?}"
                    ));
                }
                Ok(serde_json::json!(raw))
            }
            ValueKind::Language => {
                if !matches!(raw, "en" | "zh") {
                    return Err(format!("titleLanguage must be en or zh, got {raw:?}"));
                }
                Ok(serde_json::json!(raw))
            }
            ValueKind::Models => parse_models(raw).map(|models| serde_json::json!(models)),
        }
    }
}

/// Every user-settable key, in the order `get` prints them. `key` is the
/// camelCase spelling the desktop API uses; `stored` is the row name in
/// `app_settings`.
#[derive(Debug, Clone, Copy)]
pub struct Setting {
    pub key: &'static str,
    pub stored: &'static str,
    pub kind: ValueKind,
    pub help: &'static str,
}

pub const SETTINGS: &[Setting] = &[
    Setting {
        key: "approvalTier",
        stored: KEY_APPROVAL_TIER,
        kind: ValueKind::Tier,
        help: "Approval tier for file access and shell commands",
    },
    Setting {
        key: "hiddenModels",
        stored: KEY_HIDDEN_MODELS,
        kind: ValueKind::Models,
        help: "Model ids hidden from the model picker",
    },
    Setting {
        key: "autoUpgradeSkills",
        stored: KEY_AUTO_UPGRADE_SKILLS,
        kind: ValueKind::Bool,
        help: "Upgrade installed skills on app open",
    },
    Setting {
        key: "autoConnectRemote",
        stored: KEY_AUTO_CONNECT_REMOTE,
        kind: ValueKind::Bool,
        help: "Auto-connect the paired remote device on launch",
    },
    Setting {
        key: "skillGuideDismissed",
        stored: KEY_SKILL_GUIDE_DISMISSED,
        kind: ValueKind::Bool,
        help: "Skills onboarding banner dismissed",
    },
    Setting {
        key: "skillIntroDismissed",
        stored: KEY_SKILL_INTRO_DISMISSED,
        kind: ValueKind::Bool,
        help: "Skills nav-entry intro bubble acknowledged",
    },
    Setting {
        key: "bellOnComplete",
        stored: KEY_BELL_ON_COMPLETE,
        kind: ValueKind::Bool,
        help: "Bell and window attention when a run finishes",
    },
    Setting {
        key: "autoTitleFirstTurn",
        stored: KEY_AUTO_TITLE_FIRST_TURN,
        kind: ValueKind::Bool,
        help: "Generate a title after the first answer",
    },
    Setting {
        key: "titleLanguage",
        stored: KEY_TITLE_LANGUAGE,
        kind: ValueKind::Language,
        help: "Language used for generated titles",
    },
    Setting {
        key: "communityEdition",
        stored: KEY_COMMUNITY_EDITION,
        kind: ValueKind::Bool,
        help: "Use the community-edition UI",
    },
    Setting {
        key: "skillRecommend",
        stored: KEY_SKILL_RECOMMEND,
        kind: ValueKind::Bool,
        help: "Recommend one uninstalled skill per conversation",
    },
];

/// The `Setting` for a key, or `None` when this crate does not know it.
pub fn setting(key: &str) -> Option<&'static Setting> {
    SETTINGS.iter().find(|setting| setting.key == key)
}

/// The error shown for a key that is not settable.
pub fn unknown_key(key: &str) -> String {
    let keys: Vec<&str> = SETTINGS.iter().map(|setting| setting.key).collect();
    format!("unknown setting: {key}\nsettable keys: {}", keys.join(", "))
}

/// `~/.future/app/` — the directory the desktop app owns. Resolves the home
/// directory the same way the desktop backend does: `HOME` first, then
/// `USERPROFILE`, and only when the value is non-empty and absolute.
pub fn app_dir() -> Result<PathBuf, Error> {
    let home = [
        std::env::var("HOME").ok(),
        std::env::var("USERPROFILE").ok(),
    ]
    .into_iter()
    .flatten()
    .find(|value| !value.is_empty() && std::path::Path::new(value).is_absolute())
    .ok_or_else(|| Error::Invalid("HOME/USERPROFILE environment variable is not set.".into()))?;
    Ok(PathBuf::from(home).join(".future").join("app"))
}

/// `~/.future/app/app.db`.
pub fn app_db_path() -> Result<PathBuf, Error> {
    Ok(app_dir()?.join("app.db"))
}

/// Create the `app_settings` table if it does not exist. Cheap and idempotent,
/// so a first write from the CLI does not require the desktop app to have run.
pub fn ensure_table(conn: &Connection) -> Result<(), Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_settings (
             key TEXT PRIMARY KEY,
             value TEXT NOT NULL,
             updated_at INTEGER NOT NULL
         )",
    )?;
    Ok(())
}

fn open_at(path: &std::path::Path) -> Result<Connection, Error> {
    let conn = Connection::open(path)?;
    // The desktop app may hold a write lock; wait rather than failing.
    conn.execute_batch("PRAGMA busy_timeout = 5000;")?;
    Ok(conn)
}

/// Open `~/.future/app/app.db` for reading. Returns `None` when the desktop app
/// has never run, so a reader reports defaults instead of creating a database.
pub fn connect_existing() -> Result<Option<Connection>, Error> {
    let path = app_db_path()?;
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(open_at(&path)?))
}

/// Open `~/.future/app/app.db` for writing, creating the directory, the file and
/// the `app_settings` table when they do not exist yet.
pub fn connect() -> Result<Connection, Error> {
    let path = app_db_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = open_at(&path)?;
    ensure_table(&conn)?;
    Ok(conn)
}

/// Read one raw row value.
pub fn read_value(conn: &Connection, key: &str) -> Result<Option<String>, Error> {
    conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
    .map_err(Error::from)
}

/// Upsert one settings row. `now` is a Unix-millis timestamp supplied by the
/// caller so this module stays clock-free and testable.
pub fn write_value(conn: &Connection, key: &str, value: &str, now: i64) -> Result<(), Error> {
    conn.execute(
        "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![key, value, now],
    )?;
    Ok(())
}

/// Clamp a tier string to the known set; anything unknown falls back to the
/// default `"off"`.
pub fn normalize_tier(value: &str) -> String {
    match value {
        "off" | "sandbox" | "manual" => value.to_string(),
        _ => "off".to_string(),
    }
}

/// The effective defaults, before anything has been written. A reader with no
/// database at all reports these instead of creating one.
pub fn defaults() -> AppSettings {
    AppSettings {
        approval_tier: "off".to_string(),
        hidden_models: Vec::new(),
        auto_upgrade_skills: true,
        auto_connect_remote: false,
        skill_guide_dismissed: false,
        skill_intro_dismissed: false,
        bell_on_complete: true,
        auto_title_first_turn: true,
        title_language: "en".to_string(),
        community_edition: false,
        skill_recommend: true,
    }
}

/// Whether the `app_settings` table exists. A reader uses this to tell "the app
/// has never stored settings" from "the store is unreadable": the former is the
/// documented defaults, the latter must be reported (see the desktop's
/// `remote_host` tests).
pub fn table_exists(conn: &Connection) -> Result<bool, Error> {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'app_settings'",
        [],
        |row| row.get::<_, i64>(0),
    )
    .map(|count| count > 0)
    .map_err(Error::from)
}

/// The effective settings: stored values, documented defaults elsewhere.
///
/// A table that is missing is an **error**, not defaults: the desktop app's
/// phone-facing handlers rely on a store fault being reported so a settings
/// screen never renders defaults as if they were saved values. A caller that
/// owns the "app has never written settings" case checks [`table_exists`]
/// first (the CLI does, with [`defaults`]).
pub fn read(conn: &Connection) -> Result<AppSettings, Error> {
    let approval_tier = read_value(conn, KEY_APPROVAL_TIER)?
        .map(|value| normalize_tier(&value))
        .unwrap_or_else(|| "off".to_string());
    let hidden_models = read_value(conn, KEY_HIDDEN_MODELS)?
        .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
        .unwrap_or_default();
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
    let auto_title_first_turn = read_value(conn, KEY_AUTO_TITLE_FIRST_TURN)?
        .map(|value| value == "true")
        .unwrap_or(true);
    let title_language = read_value(conn, KEY_TITLE_LANGUAGE)?
        .filter(|value| matches!(value.as_str(), "en" | "zh"))
        .unwrap_or_else(|| "en".to_string());
    let community_edition = read_value(conn, KEY_COMMUNITY_EDITION)?
        .map(|value| value == "true")
        .unwrap_or(false);
    let skill_recommend = read_value(conn, KEY_SKILL_RECOMMEND)?
        .map(|value| value == "true")
        .unwrap_or(true); // On by default (PRD v1.6 §3); the Settings toggle opts out.
    Ok(AppSettings {
        approval_tier,
        hidden_models,
        auto_upgrade_skills,
        auto_connect_remote,
        skill_guide_dismissed,
        skill_intro_dismissed,
        bell_on_complete,
        auto_title_first_turn,
        title_language,
        community_edition,
        skill_recommend,
    })
}

/// Write the `Some` fields of `input` and return the effective settings.
/// Runs inside the caller's transaction when one is open, so a rejected value
/// (an unsupported title language) rolls the whole update back.
pub fn apply(
    conn: &Connection,
    input: &UpdateAppSettingsInput,
    now: i64,
) -> Result<AppSettings, Error> {
    if let Some(approval_tier) = &input.approval_tier {
        let tier = normalize_tier(approval_tier);
        write_value(conn, KEY_APPROVAL_TIER, &tier, now)?;
    }
    if let Some(hidden_models) = &input.hidden_models {
        let json = serde_json::to_string(hidden_models)?;
        write_value(conn, KEY_HIDDEN_MODELS, &json, now)?;
    }
    if let Some(auto_upgrade_skills) = input.auto_upgrade_skills {
        write_value(
            conn,
            KEY_AUTO_UPGRADE_SKILLS,
            bool_str(auto_upgrade_skills),
            now,
        )?;
    }
    if let Some(auto_connect_remote) = input.auto_connect_remote {
        write_value(
            conn,
            KEY_AUTO_CONNECT_REMOTE,
            bool_str(auto_connect_remote),
            now,
        )?;
    }
    if let Some(skill_guide_dismissed) = input.skill_guide_dismissed {
        write_value(
            conn,
            KEY_SKILL_GUIDE_DISMISSED,
            bool_str(skill_guide_dismissed),
            now,
        )?;
    }
    if let Some(skill_intro_dismissed) = input.skill_intro_dismissed {
        write_value(
            conn,
            KEY_SKILL_INTRO_DISMISSED,
            bool_str(skill_intro_dismissed),
            now,
        )?;
    }
    if let Some(bell_on_complete) = input.bell_on_complete {
        write_value(conn, KEY_BELL_ON_COMPLETE, bool_str(bell_on_complete), now)?;
    }
    if let Some(auto_title_first_turn) = input.auto_title_first_turn {
        write_value(
            conn,
            KEY_AUTO_TITLE_FIRST_TURN,
            bool_str(auto_title_first_turn),
            now,
        )?;
    }
    if let Some(language) = &input.title_language {
        if !matches!(language.as_str(), "en" | "zh") {
            return Err(Error::Invalid("Unsupported title language".into()));
        }
        write_value(conn, KEY_TITLE_LANGUAGE, language, now)?;
    }
    if let Some(community_edition) = input.community_edition {
        write_value(
            conn,
            KEY_COMMUNITY_EDITION,
            bool_str(community_edition),
            now,
        )?;
    }
    if let Some(skill_recommend) = input.skill_recommend {
        write_value(conn, KEY_SKILL_RECOMMEND, bool_str(skill_recommend), now)?;
    }
    read(conn)
}

fn bool_str(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

/// Parse `<key> <raw>` into a one-field update, the way
/// `future desktop settings set` accepts it. Unknown keys and unparsable
/// values are rejected before anything is written.
pub fn update_from_text(key: &str, raw: &str) -> Result<UpdateAppSettingsInput, Error> {
    let setting = setting(key).ok_or_else(|| Error::Invalid(unknown_key(key)))?;
    let value = setting.kind.parse(raw).map_err(Error::Invalid)?;
    let patch = serde_json::json!({ setting.key: value });
    serde_json::from_value::<UpdateAppSettingsInput>(patch)
        .map_err(|error| Error::Invalid(format!("{key}: {error}")))
}

impl AppSettings {
    /// The serialized value of one settable key, or `None` for a key this crate
    /// does not expose.
    pub fn value(&self, key: &str) -> Option<serde_json::Value> {
        serde_json::to_value(self).ok()?.get(key).cloned()
    }
}

fn parse_bool(raw: &str) -> Result<bool, String> {
    match raw.to_ascii_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("expected true or false, got {raw:?}")),
    }
}

fn parse_models(raw: &str) -> Result<Vec<String>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    if trimmed.starts_with('[') {
        return serde_json::from_str::<Vec<String>>(trimmed)
            .map_err(|error| format!("hiddenModels must be a JSON array of strings: {error}"));
    }
    Ok(trimmed
        .split(',')
        .map(|entry| entry.trim().to_string())
        .filter(|entry| !entry.is_empty())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory database");
        ensure_table(&conn).expect("create table");
        conn
    }

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
    fn tier_normalization_keeps_the_known_values_and_defaults_the_rest() {
        for tier in ["off", "sandbox", "manual"] {
            assert_eq!(normalize_tier(tier), tier);
        }
        assert_eq!(normalize_tier("anything-else"), "off");
    }

    /// An empty table (the schema exists, nothing written) equals the documented
    /// defaults exactly — the two must not drift.
    #[test]
    fn defaults_apply_on_a_fresh_database() {
        let settings = read(&conn()).expect("read defaults");
        assert_eq!(
            serde_json::to_value(&settings).expect("serialize"),
            serde_json::to_value(defaults()).expect("serialize"),
            "an empty table must equal the documented defaults"
        );
    }

    #[test]
    fn a_missing_table_is_a_read_error_without_creating_it() {
        let bare = Connection::open_in_memory().expect("in-memory database");
        assert!(!table_exists(&bare).expect("probe"));
        // The desktop's phone-facing handlers depend on this being an error:
        // an unreadable store must be reported, never rendered as defaults.
        let error = read(&bare).expect_err("a missing table must not default");
        assert!(error.to_string().contains("no such table"), "{error}");
        let table_count: i64 = bare
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'app_settings'",
                [],
                |row| row.get(0),
            )
            .expect("count tables");
        assert_eq!(table_count, 0, "a read must not create the table");
    }

    #[test]
    fn every_settable_key_round_trips_and_is_serialized() {
        // SETTINGS is the single list: a field added to the struct but not here
        // (or vice versa) fails this test instead of silently missing from the CLI.
        let serialized = serde_json::to_value(AppSettings::default()).expect("serialize");
        let fields: Vec<&str> = serialized
            .as_object()
            .expect("object")
            .keys()
            .map(String::as_str)
            .collect();
        let mut listed: Vec<&str> = SETTINGS.iter().map(|setting| setting.key).collect();
        let mut sorted_fields = fields.clone();
        sorted_fields.sort_unstable();
        listed.sort_unstable();
        assert_eq!(sorted_fields, listed, "SETTINGS and AppSettings drifted");

        for setting in SETTINGS {
            assert!(
                setting.value_sample().is_some(),
                "{} has no sample",
                setting.key
            );
        }
    }

    impl Setting {
        /// A value this key accepts, so tests can exercise every key.
        fn value_sample(self) -> Option<&'static str> {
            match self.kind {
                ValueKind::Bool => Some("false"),
                ValueKind::Tier => Some("manual"),
                ValueKind::Language => Some("zh"),
                ValueKind::Models => Some(r#"["future/x"]"#),
            }
        }
    }

    #[test]
    fn update_from_text_parses_every_key() {
        for setting in SETTINGS {
            let sample = setting.value_sample().expect("sample");
            let input = update_from_text(setting.key, sample).expect(setting.key);
            // Exactly one field must be set by a single-key update.
            let serialized = serde_json::json!({
                "approvalTier": input.approval_tier,
                "hiddenModels": input.hidden_models,
                "autoUpgradeSkills": input.auto_upgrade_skills,
                "autoConnectRemote": input.auto_connect_remote,
                "skillGuideDismissed": input.skill_guide_dismissed,
                "skillIntroDismissed": input.skill_intro_dismissed,
                "bellOnComplete": input.bell_on_complete,
                "autoTitleFirstTurn": input.auto_title_first_turn,
                "titleLanguage": input.title_language,
                "communityEdition": input.community_edition,
                "skillRecommend": input.skill_recommend,
            });
            let set: Vec<&str> = serialized
                .as_object()
                .expect("object")
                .iter()
                .filter(|(_, value)| !value.is_null())
                .map(|(key, _)| key.as_str())
                .collect();
            assert_eq!(set, vec![setting.key], "{}", setting.key);
        }
    }

    #[test]
    fn models_accept_a_json_array_or_a_comma_list() {
        let input = update_from_text("hiddenModels", r#"["a/b","c/d"]"#).expect("json array");
        assert_eq!(input.hidden_models.unwrap(), vec!["a/b", "c/d"]);
        let input = update_from_text("hiddenModels", "a/b, c/d ,").expect("comma list");
        assert_eq!(input.hidden_models.unwrap(), vec!["a/b", "c/d"]);
        let input = update_from_text("hiddenModels", "").expect("empty clears");
        assert_eq!(input.hidden_models.unwrap(), Vec::<String>::new());
        assert!(update_from_text("hiddenModels", "[1,2]").is_err());
    }

    #[test]
    fn invalid_keys_and_values_are_refused() {
        assert!(update_from_text("notAKey", "1").is_err());
        // The stored snake_case spelling is not the API spelling.
        assert!(update_from_text("bell_on_complete", "true").is_err());
        assert!(update_from_text("bellOnComplete", "yes").is_err());
        assert!(update_from_text("approvalTier", "sometimes").is_err());
        assert!(update_from_text("titleLanguage", "fr").is_err());
        assert_eq!(
            update_from_text("approvalTier", "sandbox")
                .unwrap()
                .approval_tier
                .unwrap(),
            "sandbox"
        );
    }

    #[test]
    fn apply_round_trips_every_field_and_persists() {
        let conn = conn();
        let updated = apply(&conn, &full_input(), 7).expect("apply");
        assert_eq!(updated.approval_tier, "sandbox");
        assert_eq!(updated.hidden_models, vec!["openai/gpt-x".to_string()]);
        assert!(!updated.auto_upgrade_skills);
        assert!(updated.auto_connect_remote);
        assert!(updated.skill_guide_dismissed);
        assert!(updated.skill_intro_dismissed);
        assert!(updated.community_edition);
        assert_eq!(read(&conn).expect("re-read").approval_tier, "sandbox");
        assert_eq!(
            read_value(&conn, KEY_APPROVAL_TIER).expect("row"),
            Some("sandbox".to_string())
        );
    }

    #[test]
    fn apply_normalizes_an_unknown_tier_and_refuses_a_bad_language() {
        let conn = conn();
        let updated = apply(
            &conn,
            &UpdateAppSettingsInput {
                approval_tier: Some("permissive".into()),
                ..Default::default()
            },
            1,
        )
        .expect("apply");
        assert_eq!(updated.approval_tier, "off");

        let error = apply(
            &conn,
            &UpdateAppSettingsInput {
                title_language: Some("fr".into()),
                ..Default::default()
            },
            1,
        )
        .expect_err("unsupported language");
        assert!(error.to_string().contains("Unsupported title language"));
        assert_eq!(read(&conn).expect("read back").title_language, "en");
    }

    #[test]
    fn an_unsupported_language_rolls_back_the_whole_update() {
        let conn = conn();
        // A transaction around `apply` (as the desktop backend and the CLI both
        // open) must roll back the earlier fields when the language is refused.
        let tx = conn.unchecked_transaction().expect("begin");
        let result = apply(
            &tx,
            &UpdateAppSettingsInput {
                bell_on_complete: Some(false),
                title_language: Some("fr".into()),
                ..Default::default()
            },
            1,
        );
        assert!(result.is_err());
        drop(tx);
        assert!(
            read(&conn).expect("read back").bell_on_complete,
            "the bell write must roll back with the failed update"
        );
    }

    #[test]
    fn corrupt_stored_values_read_as_defaults() {
        let conn = conn();
        write_value(&conn, KEY_APPROVAL_TIER, "weird", 1).expect("tier");
        write_value(&conn, KEY_HIDDEN_MODELS, "{not json", 1).expect("models");
        write_value(&conn, KEY_AUTO_UPGRADE_SKILLS, "0", 1).expect("upgrade");
        write_value(&conn, KEY_AUTO_CONNECT_REMOTE, "true", 1).expect("remote");
        write_value(&conn, KEY_BELL_ON_COMPLETE, "yes", 1).expect("bell");
        write_value(&conn, KEY_COMMUNITY_EDITION, "true", 1).expect("community");
        write_value(&conn, KEY_TITLE_LANGUAGE, "fr", 1).expect("language");

        let settings = read(&conn).expect("read");
        assert_eq!(settings.approval_tier, "off");
        assert!(settings.hidden_models.is_empty());
        assert!(!settings.auto_upgrade_skills);
        assert!(settings.auto_connect_remote);
        assert!(!settings.bell_on_complete);
        assert!(settings.community_edition);
        assert_eq!(settings.title_language, "en");
    }

    #[test]
    fn a_noop_update_preserves_every_default() {
        let conn = conn();
        let settings = apply(&conn, &UpdateAppSettingsInput::default(), 1).expect("noop");
        assert_eq!(settings.approval_tier, "off");
        assert!(settings.auto_upgrade_skills);
        assert!(settings.skill_recommend);
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM app_settings", [], |row| row
                .get::<_, i64>(0))
                .expect("count"),
            0,
            "a noop must not write rows"
        );
    }

    #[test]
    fn value_reads_one_settables_key_and_ignores_unknown_ones() {
        let settings = AppSettings {
            title_language: "zh".into(),
            hidden_models: vec!["future/x".into()],
            ..Default::default()
        };
        assert_eq!(
            settings.value("titleLanguage"),
            Some(serde_json::json!("zh"))
        );
        assert_eq!(
            settings.value("hiddenModels"),
            Some(serde_json::json!(["future/x"]))
        );
        assert_eq!(settings.value("notAKey"), None);
        // The internal device identity is not part of this document.
        assert_eq!(settings.value("deviceId"), None);
    }

    #[test]
    fn connect_creates_the_database_and_connect_existing_reads_it_back() {
        // Serialized: these tests mutate process environment.
        let _guard = ENV_LOCK.lock().unwrap_or_else(|poison| poison.into_inner());
        let previous_home = std::env::var("HOME").ok();
        let root = tempfile::tempdir().expect("tempdir");
        std::env::set_var("HOME", root.path());

        // Before the first write there is no database, and a reader must not
        // create one.
        assert!(connect_existing().expect("probe").is_none());
        assert!(!app_db_path().expect("path").exists());

        let conn = connect().expect("create");
        let updated = apply(
            &conn,
            &UpdateAppSettingsInput {
                bell_on_complete: Some(false),
                ..Default::default()
            },
            42,
        )
        .expect("write");
        assert!(!updated.bell_on_complete);
        drop(conn);

        // A second `connect` is idempotent, and the value survives it.
        let conn = connect().expect("reopen");
        assert!(!read(&conn).expect("read back").bell_on_complete);
        drop(conn);
        let conn = connect_existing().expect("existing").expect("present");
        assert_eq!(
            read_value(&conn, KEY_BELL_ON_COMPLETE).expect("row"),
            Some("false".to_string())
        );
        drop(conn);

        match previous_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn app_dir_and_db_path_follow_home_then_userprofile() {
        // Serialized: these tests mutate process environment.
        let _guard = ENV_LOCK.lock().unwrap_or_else(|poison| poison.into_inner());
        let previous_home = std::env::var("HOME").ok();
        let previous_profile = std::env::var("USERPROFILE").ok();
        let root = tempfile::tempdir().expect("tempdir");

        std::env::set_var("HOME", root.path());
        assert_eq!(app_dir().unwrap(), root.path().join(".future").join("app"));
        assert_eq!(
            app_db_path().unwrap(),
            root.path().join(".future/app/app.db")
        );

        // A relative or empty HOME is ignored in favour of USERPROFILE.
        std::env::set_var("HOME", "relative");
        std::env::set_var("USERPROFILE", root.path());
        assert_eq!(app_dir().unwrap(), root.path().join(".future").join("app"));

        std::env::set_var("HOME", "");
        std::env::set_var("USERPROFILE", "");
        assert!(app_dir().is_err(), "no usable home must be an error");

        match previous_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match previous_profile {
            Some(value) => std::env::set_var("USERPROFILE", value),
            None => std::env::remove_var("USERPROFILE"),
        }
    }

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
}
