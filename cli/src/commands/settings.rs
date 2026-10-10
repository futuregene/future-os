//! `future config get` / `future config set` — read and change the global agent
//! settings document (`~/.future/agent/settings.json`).
//!
//! Both work with the Agent stopped, like `future config` and `future auth`:
//! settings are read from disk whenever they are needed (new sessions, model
//! resolution, the compaction/retry policy at engine start), so there is no
//! in-memory copy a running Agent would have to be told about.
//!
//! `get` reads through the typed [`Settings`] loader, so it reports the
//! *effective* value of every key — including the ones the file omits.
//! `set` edits the JSON document in place and validates only the value it
//! writes, so keys this build does not know about, and the file's own
//! formatting, survive untouched.

use crate::output::Output;
use future_agent::Settings;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Every key `set` accepts, in the order `get` prints them. Anything else is
/// refused rather than written, so a typo can never create a key the Agent
/// silently ignores.
const KEYS: &[&str] = &[
    "compaction.enabled",
    "compaction.reserve_tokens",
    "compaction.keep_recent_tokens",
    "retry.enabled",
    "retry.max_retries",
    "retry.base_delay_ms",
    "retry.provider.max_retry_delay_ms",
    "maxTurns",
    "defaultPermissionLevel",
    "defaultModel",
];

const PERMISSION_LEVELS: [&str; 3] = ["all", "workspace", "none"];

/// `~/.future/agent/settings.json`, honouring `FUTURE_HOME` exactly like the
/// Agent's own loader.
fn settings_file() -> PathBuf {
    future_agent::utils::default_config_dir().join("settings.json")
}

fn unknown_key(key: &str) -> String {
    format!("unknown setting: {key}\nsettable keys: {}", KEYS.join(", "))
}

fn parse_bool(raw: &str) -> Result<bool, String> {
    match raw.to_ascii_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("expected true or false, got {raw:?}")),
    }
}

fn parse_int(key: &str, raw: &str, min: i32) -> Result<i32, String> {
    let value: i32 = raw
        .parse()
        .map_err(|_| format!("{key} must be an integer, got {raw:?}"))?;
    if value < min {
        return Err(format!("{key} must be at least {min}, got {value}"));
    }
    Ok(value)
}

/// Types a raw string for `key`, rejecting anything the Agent could not read
/// back. This is the only validation a `set` performs.
fn parse_value(key: &str, raw: &str) -> Result<Value, String> {
    match key {
        "compaction.enabled" | "retry.enabled" => Ok(json!(parse_bool(raw)?)),
        // Zero turns means "unlimited", so only negatives are meaningless.
        "maxTurns" => Ok(json!(parse_int(key, raw, 0)?)),
        "compaction.reserve_tokens" | "compaction.keep_recent_tokens" => {
            Ok(json!(parse_int(key, raw, 1)?))
        }
        "retry.max_retries" | "retry.base_delay_ms" | "retry.provider.max_retry_delay_ms" => {
            Ok(json!(parse_int(key, raw, 0)?))
        }
        "defaultPermissionLevel" => {
            if !PERMISSION_LEVELS.contains(&raw) {
                return Err(format!(
                    "defaultPermissionLevel must be one of {}, got {raw:?}",
                    PERMISSION_LEVELS.join(", ")
                ));
            }
            Ok(json!(raw))
        }
        "defaultModel" => {
            // Empty is the documented "no explicit choice" state.
            if raw.contains(char::is_whitespace) {
                return Err(format!("defaultModel must not contain whitespace: {raw:?}"));
            }
            Ok(json!(raw))
        }
        _ => Err(unknown_key(key)),
    }
}

/// The effective value of `key`, defaults included.
fn effective_value(settings: &Settings, key: &str) -> Result<Value, String> {
    Ok(match key {
        "compaction.enabled" => json!(settings.compaction_enabled()),
        "compaction.reserve_tokens" => json!(settings.compaction_reserve_tokens()),
        "compaction.keep_recent_tokens" => json!(settings.compaction_keep_recent_tokens()),
        "retry.enabled" => json!(settings.retry_enabled()),
        "retry.max_retries" => json!(settings.retry_max_retries()),
        "retry.base_delay_ms" => json!(settings.retry_base_delay_ms()),
        "retry.provider.max_retry_delay_ms" => json!(settings.provider_max_retry_delay_ms()),
        "maxTurns" => json!(settings.max_turns),
        "defaultPermissionLevel" => json!(settings.default_permission_level),
        "defaultModel" => json!(settings.default_model),
        _ => return Err(unknown_key(key)),
    })
}

/// How one value is rendered for a human. JSON output uses it unchanged.
fn display(value: &Value, key: &str) -> String {
    match value {
        Value::Null => "(unset)".to_string(),
        Value::String(text) if text.is_empty() && key == "defaultModel" => {
            "(not chosen)".to_string()
        }
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// The settings the Agent would actually use: file values, defaults elsewhere.
fn load_effective() -> Result<Settings, String> {
    let path = settings_file();
    future_agent::load_settings(&path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))
}

/// Read the settings document as raw JSON. A missing file is `{}`; a file that
/// is not a JSON object is refused rather than overwritten.
fn read_document(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let document: Value = serde_json::from_str(&text)
        .map_err(|error| format!("{} is not valid JSON: {error}", path.display()))?;
    if !document.is_object() {
        return Err(format!("{} must contain a JSON object", path.display()));
    }
    Ok(document)
}

/// Write `value` at the dotted `key` inside `document`, creating the
/// intermediate objects it needs. Refuses to replace a non-object parent.
fn insert(document: &mut Value, key: &str, value: Value) -> Result<(), String> {
    let mut segments: Vec<&str> = key.split('.').collect();
    let leaf = segments.pop().expect("a key has at least one segment");
    let mut cursor = document;
    for segment in segments {
        let object = cursor
            .as_object_mut()
            .ok_or_else(|| format!("cannot set {key}: {segment} is not an object"))?;
        cursor = object
            .entry(segment.to_string())
            .or_insert_with(|| json!({}));
    }
    let object = cursor
        .as_object_mut()
        .ok_or_else(|| format!("cannot set {key}: its parent is not an object"))?;
    object.insert(leaf.to_string(), value);
    Ok(())
}

/// Write the document back in the same shape the Agent's own `Settings::save`
/// produces (pretty, no trailing newline), so a later Agent write does not
/// show up as a spurious diff.
fn write_document(path: &Path, document: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(document).map_err(|error| error.to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    std::fs::write(path, text)
        .map_err(|error| format!("could not write {}: {error}", path.display()))
}

/// `future config get [<key>] [--json]`
pub fn get(args: &[String], out: &Output) -> Result<(), String> {
    let mut json_out = false;
    let mut key: Option<String> = None;
    for arg in args {
        match arg.as_str() {
            "--json" => json_out = true,
            other if other.starts_with('-') => return Err(format!("unknown option: {other}")),
            other => {
                if key.replace(other.to_string()).is_some() {
                    return Err("only one setting key may be requested".into());
                }
            }
        }
    }
    let settings = load_effective()?;
    if let Some(key) = key {
        let value = effective_value(&settings, &key)?;
        if json_out {
            out.log(&serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?);
        } else {
            out.log(&display(&value, &key));
        }
        return Ok(());
    }
    if json_out {
        let document = serde_json::to_value(&settings).map_err(|error| error.to_string())?;
        out.log(&serde_json::to_string_pretty(&document).map_err(|e| e.to_string())?);
        return Ok(());
    }
    out.log(&format!("Settings file: {}", settings_file().display()));
    out.log("");
    for key in KEYS {
        out.log(&format!(
            "{key} = {}",
            display(&effective_value(&settings, key)?, key)
        ));
    }
    Ok(())
}

/// `future config set <key> <value> [--json]`
pub fn set(args: &[String], out: &Output) -> Result<(), String> {
    let mut json_out = false;
    let mut positional: Vec<&str> = Vec::new();
    for arg in args {
        match arg.as_str() {
            "--json" => json_out = true,
            // A single leading dash is a value (a negative number, say), never
            // an option, so `set maxTurns -1` fails on the range, not parsing.
            other if other.starts_with("--") => return Err(format!("unknown option: {other}")),
            other => positional.push(other),
        }
    }
    let mut positional = positional.into_iter();
    let key = positional.next().ok_or("a setting key is required")?;
    let raw = positional.next().ok_or("a setting value is required")?;
    if positional.next().is_some() {
        return Err("too many arguments: expected <key> <value>".into());
    }
    if !KEYS.contains(&key) {
        return Err(unknown_key(key));
    }
    let value = parse_value(key, raw)?;
    let path = settings_file();
    let mut document = read_document(&path)?;
    insert(&mut document, key, value.clone())?;
    write_document(&path, &document)?;
    if json_out {
        out.log(
            &serde_json::to_string_pretty(&json!({
                "key": key,
                "value": value,
                "path": path.display().to_string(),
            }))
            .map_err(|e| e.to_string())?,
        );
    } else {
        out.log(&format!(
            "{key} = {} (written to {})",
            display(&value, key),
            path.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env::EnvGuard;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    /// A throwaway home with `FUTURE_HOME` pointed at it. The guard restores
    /// the previous value when the fixture drops, so tests stay isolated.
    struct Home {
        dir: tempfile::TempDir,
        _env: EnvGuard,
    }

    impl Home {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            let env = EnvGuard::set(&[("FUTURE_HOME", dir.path().as_os_str().to_owned())]);
            Home { dir, _env: env }
        }
        fn settings(&self) -> PathBuf {
            self.dir.path().join("agent").join("settings.json")
        }
        fn write(&self, text: &str) {
            std::fs::create_dir_all(self.dir.path().join("agent")).expect("mkdir");
            std::fs::write(self.settings(), text).expect("write");
        }
        fn read(&self) -> String {
            std::fs::read_to_string(self.settings()).expect("read")
        }
    }

    fn text(captured: std::sync::Arc<std::sync::Mutex<Vec<u8>>>) -> String {
        String::from_utf8(captured.lock().unwrap().clone()).expect("utf8")
    }

    /// The help text is the only place a user learns the key names, and two
    /// other help strings send readers to `config get --help` for them, so a key
    /// that exists in the table but is missing from the help is unreachable
    /// documentation. This caught exactly that: the key list lived in
    /// `CONFIG_HELP` only, while `config set --help` said "Run `future config get
    /// --help` for the settable keys".
    #[test]
    fn config_keys_are_documented_where_the_help_points() {
        for help in [crate::help::CONFIG_GET_HELP, crate::help::CONFIG_HELP] {
            for key in KEYS {
                assert!(
                    help.contains(key),
                    "{key} is settable but absent from a help text that promises it"
                );
            }
        }
        // The pointer must keep existing, or the test above stops meaning
        // anything: CONFIG_SET_HELP tells the reader where the list is.
        assert!(crate::help::CONFIG_SET_HELP.contains("future config get --help"));
    }

    /// `KEYS` is the single source of truth: every key must have a reader, a
    /// writer and a round trip through the Agent's own loader, so a key added
    /// to one list but not the others fails here instead of at runtime.
    #[test]
    fn every_key_round_trips_through_its_reader_and_writer() {
        let samples = [
            ("compaction.enabled", "false"),
            ("compaction.reserve_tokens", "4096"),
            ("compaction.keep_recent_tokens", "4096"),
            ("retry.enabled", "false"),
            ("retry.max_retries", "1"),
            ("retry.base_delay_ms", "10"),
            ("retry.provider.max_retry_delay_ms", "10"),
            ("maxTurns", "3"),
            ("defaultPermissionLevel", "workspace"),
            ("defaultModel", "future/x"),
        ];
        assert_eq!(samples.len(), KEYS.len(), "KEYS and the samples drifted");
        let mut seen = std::collections::HashSet::new();
        for key in KEYS {
            assert!(seen.insert(*key), "duplicate key {key}");
        }
        for (key, raw) in samples {
            assert!(KEYS.contains(&key), "{key} is missing from KEYS");
            let expected = parse_value(key, raw).unwrap();
            let mut document = json!({});
            insert(&mut document, key, expected.clone()).unwrap();
            let settings: Settings = serde_json::from_value(document).unwrap();
            assert_eq!(effective_value(&settings, key).unwrap(), expected, "{key}");
        }
    }

    #[test]
    fn values_are_validated_before_anything_is_written() {
        assert!(parse_value("compaction.enabled", "yes").is_err());
        assert!(parse_value("maxTurns", "-1").is_err());
        assert!(parse_value("maxTurns", "many").is_err());
        assert!(parse_value("compaction.reserve_tokens", "0").is_err());
        assert!(parse_value("retry.max_retries", "-1").is_err());
        assert!(parse_value("defaultPermissionLevel", "sometimes").is_err());
        assert!(parse_value("defaultModel", "two words").is_err());
        assert!(parse_value("notAKey", "1").is_err());
        // The camelCase spelling of a nested key is not the file's spelling.
        assert!(parse_value("compaction.reserveTokens", "1").is_err());
        assert_eq!(parse_value("maxTurns", "0").unwrap(), json!(0));
        assert_eq!(parse_value("retry.base_delay_ms", "0").unwrap(), json!(0));
        assert_eq!(parse_value("defaultModel", "").unwrap(), json!(""));
    }

    #[test]
    fn setting_a_key_preserves_unrelated_and_unknown_content() {
        let mut document = json!({
            "compaction": {"reserve_tokens": 8192},
            "unknownFutureKey": {"kept": true},
            "defaultModel": "future/old"
        });
        insert(&mut document, "compaction.keep_recent_tokens", json!(1234)).unwrap();
        insert(&mut document, "maxTurns", json!(7)).unwrap();
        assert_eq!(document["compaction"]["reserve_tokens"], 8192);
        assert_eq!(document["compaction"]["keep_recent_tokens"], 1234);
        assert_eq!(document["unknownFutureKey"]["kept"], true);
        assert_eq!(document["defaultModel"], "future/old");
        assert_eq!(document["maxTurns"], 7);
    }

    #[test]
    fn a_parent_that_is_not_an_object_is_refused_not_overwritten() {
        let mut document = json!({"compaction": false});
        assert!(insert(&mut document, "compaction.enabled", json!(true)).is_err());
        assert_eq!(document["compaction"], false);
    }

    #[tokio::test]
    async fn the_flat_listing_shows_effective_values_including_defaults() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let (out, captured) = Output::memory();
        get(&args(&[]), &out).unwrap();
        let listed = text(captured.out);
        assert!(
            listed.contains(&format!("Settings file: {}", home.settings().display())),
            "{listed}"
        );
        assert!(listed.contains("compaction.enabled = true"), "{listed}");
        assert!(
            listed.contains("compaction.reserve_tokens = 16384"),
            "{listed}"
        );
        assert!(listed.contains("retry.max_retries = 3"), "{listed}");
        assert!(listed.contains("maxTurns = 0"), "{listed}");
        assert!(listed.contains("defaultPermissionLevel = all"), "{listed}");
        assert!(listed.contains("defaultModel = (not chosen)"), "{listed}");
        // Reading a missing file reports the defaults; it must not create one.
        assert!(!home.settings().exists());
    }

    #[tokio::test]
    async fn a_single_key_reads_bare_for_scripting() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        home.write(r#"{"maxTurns": 12, "defaultModel": "future/pro"}"#);
        for (values, expected) in [
            (vec!["maxTurns"], "12\n"),
            (vec!["maxTurns", "--json"], "12\n"),
            (vec!["defaultModel"], "future/pro\n"),
            // A key the file omits still reports its effective value. The
            // provider block itself is optional, so this one stays unset.
            (vec!["retry.provider.max_retry_delay_ms"], "(unset)\n"),
        ] {
            let (out, captured) = Output::memory();
            get(&args(&values), &out).unwrap();
            assert_eq!(text(captured.out), expected, "{values:?}");
        }
    }

    #[tokio::test]
    async fn json_get_returns_the_effective_document() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        home.write(r#"{"compaction": {"reserve_tokens": 4096}}"#);
        let (out, captured) = Output::memory();
        get(&args(&["--json"]), &out).unwrap();
        let value: Value = serde_json::from_str(&text(captured.out)).unwrap();
        assert_eq!(value["compaction"]["reserve_tokens"], 4096);
        // Defaults for the omitted keys are materialised in the view.
        assert_eq!(value["compaction"]["keep_recent_tokens"], 20000);
        assert_eq!(value["maxTurns"], 0);
    }

    #[tokio::test]
    async fn set_writes_a_minimal_document_and_keeps_what_it_did_not_touch() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        home.write(r#"{"unknown": 1, "compaction": {"reserve_tokens": 8192}}"#);
        let (out, captured) = Output::memory();
        set(&args(&["compaction.enabled", "false"]), &out).unwrap();
        assert!(
            text(captured.out).contains("compaction.enabled = false"),
            "confirmation names the value"
        );
        let document: Value = serde_json::from_str(&home.read()).unwrap();
        assert_eq!(document["compaction"]["enabled"], false);
        assert_eq!(document["compaction"]["reserve_tokens"], 8192);
        assert_eq!(document["unknown"], 1);
        // No trailing newline: the Agent's own writer produces none either.
        assert!(!home.read().ends_with('\n'));

        let (out, captured) = Output::memory();
        set(&args(&["maxTurns", "5", "--json"]), &out).unwrap();
        let summary: Value = serde_json::from_str(&text(captured.out)).unwrap();
        assert_eq!(summary["key"], "maxTurns");
        assert_eq!(summary["value"], 5);
        assert!(summary["path"].as_str().unwrap().ends_with("settings.json"));
        let document: Value = serde_json::from_str(&home.read()).unwrap();
        assert_eq!(document["maxTurns"], 5);
        assert_eq!(document["unknown"], 1);
    }

    #[tokio::test]
    async fn set_creates_the_file_and_round_trips_through_the_agent_loader() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let (out, _captured) = Output::memory();
        set(&args(&["defaultPermissionLevel", "workspace"]), &out).unwrap();
        set(&args(&["defaultModel", "future/pro"]), &out).unwrap();
        let settings = future_agent::load_settings(&home.settings()).unwrap();
        assert_eq!(settings.default_permission_level, "workspace");
        assert_eq!(settings.default_model, "future/pro");
        // Keys that were never set keep their documented defaults.
        assert!(settings.compaction_enabled());
        assert_eq!(settings.retry_max_retries(), 3);
        assert_eq!(settings.retry_base_delay_ms(), 2000);
        // No provider block was written, so the provider-level cap stays unset
        // (the Agent's own fallback applies only inside an existing block).
        assert_eq!(settings.provider_max_retry_delay_ms(), None);
    }

    #[tokio::test]
    async fn bad_arguments_never_touch_the_file() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        home.write(r#"{"maxTurns": 1}"#);
        let before = home.read();
        let (out, _captured) = Output::memory();
        for values in [
            vec!["maxTurns"],
            vec![],
            vec!["maxTurns", "5", "extra"],
            vec!["notAKey", "1"],
            vec!["maxTurns", "many"],
            vec!["maxTurns", "-1"],
            vec!["maxTurns", "5", "--nope"],
        ] {
            assert!(set(&args(&values), &out).is_err(), "{values:?}");
        }
        assert_eq!(home.read(), before);
    }

    #[tokio::test]
    async fn a_broken_or_non_object_file_is_refused() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let (out, _captured) = Output::memory();
        home.write("not json");
        assert!(load_effective().is_err());
        assert!(get(&args(&[]), &out).is_err());
        assert!(set(&args(&["maxTurns", "5"]), &out).is_err());
        assert_eq!(home.read(), "not json");

        home.write("[1, 2]");
        assert!(read_document(&home.settings()).is_err());
        assert!(get(&args(&[]), &out).is_err());
    }

    #[tokio::test]
    async fn unknown_options_and_extra_keys_are_refused_on_get() {
        let _guard = crate::test_env::lock_env().await;
        let _home = Home::new();
        let (out, _captured) = Output::memory();
        assert!(get(&args(&["--nope"]), &out).is_err());
        assert!(get(&args(&["maxTurns", "defaultModel"]), &out).is_err());
        assert!(get(&args(&["notAKey"]), &out).is_err());
    }

    #[tokio::test]
    async fn empty_model_clears_the_choice_and_prints_as_unchosen() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        home.write(r#"{"defaultModel": "future/old"}"#);
        let (out, _captured) = Output::memory();
        set(&args(&["defaultModel", ""]), &out).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&home.read()).unwrap()["defaultModel"],
            ""
        );
        let (out, captured) = Output::memory();
        get(&args(&["defaultModel"]), &out).unwrap();
        assert_eq!(text(captured.out), "(not chosen)\n");
    }
}
