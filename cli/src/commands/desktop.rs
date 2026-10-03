//! `future desktop settings` — read and change the desktop app's local
//! settings (`~/.future/app/app.db`, the `app_settings` table).
//!
//! These preferences belong to the Tauri desktop app; the keys, defaults and
//! validation live in the `future-app-settings` crate so this command and the
//! app's own Settings screen cannot drift apart. They are separate from the
//! agent's settings document (`future config`) and from models/providers/auth,
//! which the desktop app and the agent share.
//!
//! Both subcommands work without a running desktop app: like the agent's
//! settings, these values are read from disk when they are used. A running app
//! notices a change the next time it reads them.

use crate::help;
use crate::output::Output;
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

/// `future desktop <command> [args]`.
pub fn desktop(command: Option<&str>, rest: &[String], out: &Output) -> Result<(), String> {
    match command {
        None | Some("--help" | "-h") => {
            out.log(help::DESKTOP_HELP);
            Ok(())
        }
        Some("settings") => settings(rest, out),
        Some(other) => Err(format!(
            "Unknown argument: {other}\nUsage: future desktop settings [get [<key>] | set <key> <value>]"
        )),
    }
}

/// `future desktop settings [get [<key>] | set <key> <value>]`.
fn settings(args: &[String], out: &Output) -> Result<(), String> {
    let help_flag = args
        .iter()
        .any(|argument| argument == "--help" || argument == "-h");
    match args.first().map(String::as_str) {
        Some("get") if help_flag => {
            out.log(help::DESKTOP_GET_HELP);
            Ok(())
        }
        Some("set") if help_flag => {
            out.log(help::DESKTOP_SET_HELP);
            Ok(())
        }
        Some("get") => get(&args[1..], out),
        Some("set") => set(&args[1..], out),
        _ if help_flag => {
            out.log(help::DESKTOP_HELP);
            Ok(())
        }
        // `future desktop settings [<key>] [--json]` reads, like `config get`.
        _ => get(args, out),
    }
}

/// How one value is rendered for a human. Strings print bare; everything else
/// (booleans, lists) is already valid JSON.
fn display(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn db_path() -> Result<std::path::PathBuf, String> {
    future_app_settings::app_db_path().map_err(|error| error.to_string())
}

/// The effective settings: stored values, or the documented defaults when the
/// desktop app has no store yet. A store that exists but cannot be read is an
/// error — the same distinction the app's own phone-facing handlers make, so
/// defaults are never passed off as saved values.
fn load() -> Result<future_app_settings::AppSettings, String> {
    let Some(conn) = future_app_settings::connect_existing().map_err(|error| error.to_string())?
    else {
        return Ok(future_app_settings::defaults());
    };
    if !future_app_settings::table_exists(&conn).map_err(|error| error.to_string())? {
        // The desktop app's connection created the file but no settings have
        // been written yet.
        return Ok(future_app_settings::defaults());
    }
    future_app_settings::read(&conn).map_err(|error| error.to_string())
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

/// `future desktop settings [<key>] [--json]`
fn get(args: &[String], out: &Output) -> Result<(), String> {
    let mut json_out = false;
    let mut key: Option<String> = None;
    for argument in args {
        match argument.as_str() {
            "--json" => json_out = true,
            other if other.starts_with('-') => return Err(format!("unknown option: {other}")),
            other => {
                if key.replace(other.to_string()).is_some() {
                    return Err("only one setting key may be requested".into());
                }
            }
        }
    }
    // An unknown key is refused even before the database is touched.
    if let Some(requested) = &key {
        future_app_settings::setting(requested)
            .ok_or_else(|| future_app_settings::unknown_key(requested))?;
    }
    let path = db_path()?;
    let settings = load()?;
    if let Some(key) = key {
        let value = settings.value(&key).unwrap_or(Value::Null);
        if json_out {
            out.log(&serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?);
        } else {
            out.log(&display(&value));
        }
        return Ok(());
    }
    if json_out {
        let document = serde_json::to_value(&settings).map_err(|error| error.to_string())?;
        out.log(&serde_json::to_string_pretty(&document).map_err(|error| error.to_string())?);
        return Ok(());
    }
    out.log(&format!("Settings database: {}", path.display()));
    out.log("");
    for setting in future_app_settings::SETTINGS {
        let value = settings.value(setting.key).unwrap_or(Value::Null);
        out.log(&format!("{} = {}", setting.key, display(&value)));
    }
    Ok(())
}

/// `future desktop settings set <key> <value> [--json]`
fn set(args: &[String], out: &Output) -> Result<(), String> {
    let mut json_out = false;
    let mut positional: Vec<&str> = Vec::new();
    for argument in args {
        match argument.as_str() {
            "--json" => json_out = true,
            // A single leading dash is a value in the general case; only `--`
            // options are refused, so a stray `-x` is reported as a bad value.
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
    // Parsing (and therefore validation) happens before the database is opened,
    // so a bad key or value leaves the file exactly as it was.
    let input =
        future_app_settings::update_from_text(key, raw).map_err(|error| error.to_string())?;
    let path = db_path()?;
    let mut conn = future_app_settings::connect().map_err(|error| error.to_string())?;
    let tx = conn.transaction().map_err(|error| error.to_string())?;
    let settings =
        future_app_settings::apply(&tx, &input, now_millis()).map_err(|error| error.to_string())?;
    tx.commit().map_err(|error| error.to_string())?;
    let value = settings.value(key).unwrap_or(Value::Null);
    if json_out {
        out.log(
            &serde_json::to_string_pretty(&json!({
                "key": key,
                "value": value,
                "path": path.display().to_string(),
            }))
            .map_err(|error| error.to_string())?,
        );
    } else {
        out.log(&format!(
            "{key} = {} (written to {})",
            display(&value),
            path.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env::EnvGuard;
    use std::path::PathBuf;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    /// A throwaway home with `HOME` pointed at it. The desktop settings live
    /// under the real home (not `FUTURE_HOME`), matching the app.
    struct Home {
        dir: tempfile::TempDir,
        _env: EnvGuard,
    }

    impl Home {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            let env = EnvGuard::set(&[("HOME", dir.path().as_os_str().to_owned())]);
            Home { dir, _env: env }
        }
        fn db(&self) -> PathBuf {
            self.dir.path().join(".future").join("app").join("app.db")
        }
    }

    fn text(captured: std::sync::Arc<std::sync::Mutex<Vec<u8>>>) -> String {
        String::from_utf8(captured.lock().unwrap().clone()).expect("utf8")
    }

    /// Every key is listed in the help text, so a key added to the shared
    /// schema cannot silently go undocumented.
    #[test]
    fn the_help_lists_every_settable_key() {
        for setting in future_app_settings::SETTINGS {
            assert!(
                help::DESKTOP_HELP.contains(setting.key),
                "{} is missing from the help",
                setting.key
            );
        }
    }

    #[tokio::test]
    async fn the_flat_listing_reports_defaults_and_never_creates_the_database() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let (out, captured) = Output::memory();
        get(&args(&[]), &out).unwrap();
        let listed = text(captured.out);
        assert!(
            listed.contains(&format!("Settings database: {}", home.db().display())),
            "{listed}"
        );
        assert!(listed.contains("approvalTier = off"), "{listed}");
        assert!(listed.contains("autoConnectRemote = false"), "{listed}");
        assert!(listed.contains("bellOnComplete = true"), "{listed}");
        assert!(listed.contains("titleLanguage = en"), "{listed}");
        assert!(listed.contains("hiddenModels = []"), "{listed}");
        // Reading reports the defaults; it must not create the database.
        assert!(!home.db().exists());
    }

    #[tokio::test]
    async fn an_uninitialized_database_reads_as_defaults() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        // The desktop app's own connection creates the file before the schema
        // is applied; reading it must report defaults, not an error.
        std::fs::create_dir_all(home.db().parent().expect("parent")).expect("mkdir");
        std::fs::write(home.db(), b"").expect("empty database");
        let (out, captured) = Output::memory();
        get(&args(&[]), &out).unwrap();
        assert!(text(captured.out).contains("approvalTier = off"));
        // A write fills the table in and is visible to the next read.
        let (out, _captured) = Output::memory();
        set(&args(&["bellOnComplete", "false"]), &out).unwrap();
        let (out, captured) = Output::memory();
        get(&args(&["bellOnComplete"]), &out).unwrap();
        assert_eq!(text(captured.out), "false\n");
    }

    #[tokio::test]
    async fn set_writes_and_a_single_key_reads_bare() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let (out, captured) = Output::memory();
        set(&args(&["approvalTier", "manual"]), &out).unwrap();
        assert!(
            text(captured.out).contains("approvalTier = manual"),
            "the confirmation names the value"
        );
        assert!(home.db().exists(), "a write creates the database");

        for (values, expected) in [
            (vec!["approvalTier"], "manual\n"),
            (vec!["approvalTier", "--json"], "\"manual\"\n"),
            // A key never written still reports its effective value.
            (vec!["bellOnComplete"], "true\n"),
            (vec!["hiddenModels"], "[]\n"),
        ] {
            let (out, captured) = Output::memory();
            get(&args(&values), &out).unwrap();
            assert_eq!(text(captured.out), expected, "{values:?}");
        }
    }

    #[tokio::test]
    async fn set_parses_lists_booleans_and_the_title_language() {
        let _guard = crate::test_env::lock_env().await;
        let _home = Home::new();
        let (out, _captured) = Output::memory();
        for values in [
            vec!["hiddenModels", "a/b, c/d"],
            vec!["bellOnComplete", "false"],
            vec!["titleLanguage", "zh"],
            vec!["communityEdition", "true"],
        ] {
            set(&args(&values), &out).unwrap_or_else(|error| panic!("{values:?}: {error}"));
        }
        for (values, expected) in [
            (vec!["hiddenModels"], "[\"a/b\",\"c/d\"]\n"),
            (vec!["bellOnComplete"], "false\n"),
            (vec!["titleLanguage"], "zh\n"),
            (vec!["communityEdition"], "true\n"),
        ] {
            let (out, captured) = Output::memory();
            get(&args(&values), &out).unwrap();
            assert_eq!(text(captured.out), expected, "{values:?}");
        }
    }

    #[tokio::test]
    async fn bad_arguments_never_create_the_database() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let (out, _captured) = Output::memory();
        for values in [
            vec!["notAKey", "1"],
            // The stored snake_case spelling is not the API spelling.
            vec!["bell_on_complete", "true"],
            vec!["bellOnComplete", "yes"],
            vec!["approvalTier", "sometimes"],
            vec!["titleLanguage", "fr"],
            vec!["hiddenModels", "[1,2]"],
            vec!["approvalTier"],
            vec![],
            vec!["approvalTier", "manual", "extra"],
            vec!["approvalTier", "manual", "--nope"],
        ] {
            assert!(set(&args(&values), &out).is_err(), "{values:?}");
        }
        assert!(
            !home.db().exists(),
            "a rejected write must not create a file"
        );
    }

    #[tokio::test]
    async fn json_get_returns_the_effective_document() {
        let _guard = crate::test_env::lock_env().await;
        let _home = Home::new();
        let (out, captured) = Output::memory();
        get(&args(&["--json"]), &out).unwrap();
        let value: Value = serde_json::from_str(&text(captured.out)).unwrap();
        assert_eq!(value["approvalTier"], "off");
        assert_eq!(value["autoUpgradeSkills"], true);
        assert_eq!(value["hiddenModels"], json!([]));
    }

    #[tokio::test]
    async fn set_json_reports_the_written_value_and_path() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let (out, captured) = Output::memory();
        set(&args(&["bellOnComplete", "false", "--json"]), &out).unwrap();
        let summary: Value = serde_json::from_str(&text(captured.out)).unwrap();
        assert_eq!(summary["key"], "bellOnComplete");
        assert_eq!(summary["value"], false);
        assert_eq!(summary["path"], home.db().display().to_string());
    }

    #[tokio::test]
    async fn unknown_keys_and_options_are_refused_on_get() {
        let _guard = crate::test_env::lock_env().await;
        let _home = Home::new();
        let (out, _captured) = Output::memory();
        assert!(get(&args(&["--nope"]), &out).is_err());
        assert!(get(&args(&["notAKey"]), &out).is_err());
        assert!(get(&args(&["approvalTier", "titleLanguage"]), &out).is_err());
    }

    #[tokio::test]
    async fn an_unsupported_language_is_refused_and_leaves_the_database_untouched() {
        let _guard = crate::test_env::lock_env().await;
        let _home = Home::new();
        let (out, _captured) = Output::memory();
        set(&args(&["titleLanguage", "fr"]), &out).unwrap_err();
        let (out, captured) = Output::memory();
        get(&args(&["titleLanguage"]), &out).unwrap();
        assert_eq!(text(captured.out), "en\n");
    }
}
