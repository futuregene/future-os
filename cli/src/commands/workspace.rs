//! `future workspace` — manage the desktop app's workspaces.
//!
//! A workspace is the directory a workspace conversation is filed under. The
//! records live in the desktop app's database (`~/.future/app/app.db`), which is
//! the same store the app's own new-conversation dialog and a paired phone
//! write: a workspace added here shows up in the desktop app's sidebar and on
//! the phone once the app publishes its catalogue. No running desktop app is
//! needed for the write, exactly as `future desktop settings` needs none.
//!
//! The rules — `~` expansion, the directory having to exist, the name defaulting
//! to the directory's own name, and one directory meaning one workspace however
//! it is spelled — are `future-app-workspaces`, shared with the desktop backend,
//! so `future workspace add` and the app's dialog cannot drift apart.

use crate::help;
use crate::output::Output;
use future_app_workspaces as workspaces;
use rusqlite::Connection;
use serde_json::json;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, String>;

/// The desktop app opens its database with these: foreign keys enforced, a write
/// lock waited for rather than failed, WAL. A CLI write into the same file uses
/// the same settings so the two writers see identical semantics.
const DESKTOP_PRAGMAS: &str =
    "PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000; PRAGMA journal_mode = WAL;";

/// `future workspace <command> [args]`.
pub fn workspace(command: Option<&str>, rest: &[String], out: &Output) -> Result<()> {
    match command {
        None | Some("--help" | "-h") => {
            out.log(help::WORKSPACE_HELP);
            Ok(())
        }
        Some("list") => list(rest, out),
        Some("add") => add(rest, out),
        Some(other) => Err(format!(
            "Unknown argument: {other}\nUsage: future workspace [list | add <path>]\nRun `future workspace --help` for details."
        )),
    }
}

/// `future workspace list [--json]`
fn list(args: &[String], out: &Output) -> Result<()> {
    let json_flag = reject_arguments(args)?;

    // A database the desktop app has never created means the user has no
    // workspaces, not that the read failed — and listing must not create one.
    let Some(conn) = open_existing()? else {
        return report_empty(json_flag, out);
    };
    let workspaces = user_workspaces(&conn)?;
    if workspaces.is_empty() {
        return report_empty(json_flag, out);
    }

    if json_flag {
        out.log(&serde_json::to_string_pretty(&workspaces).map_err(|error| error.to_string())?);
        return Ok(());
    }
    for workspace in &workspaces {
        out.log(&format!(
            "{:<24} {:<48} {}",
            workspace.name, workspace.path, workspace.id
        ));
    }
    Ok(())
}

fn report_empty(json_flag: bool, out: &Output) -> Result<()> {
    if json_flag {
        out.log("[]");
    } else {
        out.log("No workspaces. Add one with `future workspace add <path>`.");
    }
    Ok(())
}

/// `future workspace add <path> [--name <name>] [--json]`
fn add(args: &[String], out: &Output) -> Result<()> {
    let mut name: Option<String> = None;
    let mut json_flag = false;
    let mut path: Option<String> = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => json_flag = true,
            "--name" => {
                index += 1;
                name = Some(
                    args.get(index)
                        .cloned()
                        .ok_or_else(|| "--name requires a value".to_string())?,
                );
            }
            other if other.starts_with('-') => return Err(format!("unknown option: {other}")),
            other => {
                if path.replace(other.to_string()).is_some() {
                    return Err(format!(
                        "too many arguments: expected one path, also got {other}"
                    ));
                }
            }
        }
        index += 1;
    }
    let path = path.ok_or_else(|| "a directory path is required".to_string())?;
    // Trimmed here rather than in the store, which stays faithful to whoever
    // wrote the row; the remote bridge does the same for the phone.
    let name = name
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    let (mut conn, database) = open_for_write()?;
    // Resolving first says whether this call created a row or reopened one; the
    // create below repeats the lookup, but a command that says "Added" for an
    // existing workspace would be lying.
    let directory =
        workspaces::resolve_directory(&path, false).map_err(|error| error.to_string())?;
    let existing =
        workspaces::find_user_by_path(&conn, &directory).map_err(|error| error.to_string())?;
    let workspace = workspaces::create(&mut conn, &path, name, None, false, now_millis())
        .map_err(|error| error.to_string())?;
    let created = existing.map(|row| row.id) != Some(workspace.id.clone());

    if json_flag {
        let mut record = serde_json::to_value(&workspace).map_err(|error| error.to_string())?;
        if let Some(object) = record.as_object_mut() {
            object.insert("created".to_string(), json!(created));
            object.insert(
                "database".to_string(),
                json!(database.display().to_string()),
            );
        }
        out.log(&serde_json::to_string_pretty(&record).map_err(|error| error.to_string())?);
        return Ok(());
    }
    if created {
        out.log(&format!(
            "Added workspace {} ({})",
            workspace.name, workspace.id
        ));
    } else {
        out.log(&format!(
            "Workspace {} already covers this directory ({})",
            workspace.name, workspace.id
        ));
    }
    out.log(&format!("  path: {}", workspace.path));
    out.log(&format!("  database: {}", database.display()));
    Ok(())
}

/// The user's own workspaces, most recently opened first. The chat scratch
/// directories the app manages live in the same table and are not the user's.
fn user_workspaces(conn: &Connection) -> Result<Vec<workspaces::Workspace>> {
    Ok(workspaces::list(conn)
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|workspace| workspace.kind == "user")
        .collect())
}

/// Reject anything but `--json`, so a typo is reported instead of ignored.
fn reject_arguments(args: &[String]) -> Result<bool> {
    let mut json_flag = false;
    for argument in args {
        match argument.as_str() {
            "--json" => json_flag = true,
            other => return Err(format!("unknown option: {other}")),
        }
    }
    Ok(json_flag)
}

/// `~/.future/app/app.db`, or `None` when the desktop app has never created it.
fn open_existing() -> Result<Option<Connection>> {
    let path = future_app_settings::app_db_path().map_err(|error| error.to_string())?;
    if !path.exists() {
        return Ok(None);
    }
    let conn =
        Connection::open(&path).map_err(|error| format!("open {}: {error}", path.display()))?;
    conn.execute_batch(DESKTOP_PRAGMAS)
        .map_err(|error| error.to_string())?;
    // The app's connection creates the file before any schema exists; a
    // database without the table means nothing has been written to it yet,
    // which reads as "no workspaces" rather than as a failure.
    if !table_exists(&conn)? {
        return Ok(None);
    }
    Ok(Some(conn))
}

/// The same database, opened for writing: the directory, the file and the
/// `workspaces` table are created when they do not exist yet, so a workspace can
/// be added before the desktop app has ever run.
fn open_for_write() -> Result<(Connection, PathBuf)> {
    let path = future_app_settings::app_db_path().map_err(|error| error.to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let conn =
        Connection::open(&path).map_err(|error| format!("open {}: {error}", path.display()))?;
    conn.execute_batch(DESKTOP_PRAGMAS)
        .map_err(|error| error.to_string())?;
    workspaces::ensure_table(&conn).map_err(|error| error.to_string())?;
    Ok((conn, path))
}

fn table_exists(conn: &Connection) -> Result<bool> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'workspaces'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    Ok(count > 0)
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env::EnvGuard;
    use future_app_workspaces::Workspace;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    /// A throwaway home with `HOME` pointed at it. The desktop app's database
    /// lives under the real home, not `FUTURE_HOME`, matching the app.
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
        fn database(&self) -> PathBuf {
            self.dir.path().join(".future").join("app").join("app.db")
        }
        fn dir(&self, name: &str) -> PathBuf {
            let path = self.dir.path().join(name);
            std::fs::create_dir_all(&path).expect("create directory");
            path
        }
    }

    fn text(captured: std::sync::Arc<std::sync::Mutex<Vec<u8>>>) -> String {
        String::from_utf8(captured.lock().unwrap().clone()).expect("utf8")
    }

    /// `add` writes the desktop app's own table, in the desktop app's database,
    /// with the directory's name unless one is given — and `list` reads it back.
    #[tokio::test]
    async fn add_writes_the_desktop_store_and_list_reads_it() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let project = home.dir("projects/demo");

        let (out, captured) = Output::memory();
        add(&args(&[project.to_str().unwrap()]), &out).unwrap();
        let printed = text(captured.out);
        assert!(printed.contains("Added workspace demo"), "{printed}");
        assert!(
            printed.contains(&home.database().display().to_string()),
            "the write names the store it landed in: {printed}"
        );

        let (out, captured) = Output::memory();
        list(&[], &out).unwrap();
        let printed = text(captured.out);
        assert!(printed.contains("demo"), "{printed}");
        assert!(
            printed.contains(&project.canonicalize().unwrap().display().to_string()),
            "the stored path is canonical: {printed}"
        );

        // The row is the desktop's, in the desktop's table.
        let conn = Connection::open(home.database()).unwrap();
        let stored: Workspace = conn
            .query_row(
                &format!("SELECT {0} FROM workspaces", workspaces::COLUMNS),
                [],
                workspaces::from_row,
            )
            .expect("the desktop schema decodes the CLI's row");
        assert_eq!(stored.kind, "user");
        assert_eq!(stored.name, "demo");
        assert_eq!(stored.cleanup_status, "active");
    }

    /// A path that already has a workspace reopens it: the second call must not
    /// add a row, and it must say so instead of claiming it added something.
    #[tokio::test]
    async fn adding_the_same_directory_twice_reopens_it() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let project = home.dir("demo");

        let (out, _) = Output::memory();
        add(&args(&[project.to_str().unwrap()]), &out).unwrap();

        let (out, captured) = Output::memory();
        add(
            &args(&[project.to_str().unwrap(), "--name", "Renamed"]),
            &out,
        )
        .unwrap();
        let printed = text(captured.out);
        assert!(printed.contains("already covers"), "{printed}");
        assert!(
            printed.contains("demo"),
            "the stored name wins over the one passed in: {printed}"
        );

        let conn = Connection::open(home.database()).unwrap();
        assert_eq!(user_workspaces(&conn).unwrap().len(), 1);
    }

    /// A directory that does not exist is refused with the store's own reason,
    /// and nothing is written.
    #[tokio::test]
    async fn a_missing_directory_is_refused_and_writes_nothing() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let missing = home.dir("here").join("not-there");

        let (out, _) = Output::memory();
        let error = add(&args(&[missing.to_str().unwrap()]), &out).unwrap_err();
        assert!(error.contains("does not exist"), "got: {error}");

        let conn = Connection::open(home.database()).unwrap();
        assert!(user_workspaces(&conn).unwrap().is_empty());
    }

    /// `~` means the same directory here as it does in the desktop app.
    #[tokio::test]
    async fn a_tilde_path_is_expanded() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let project = home.dir("tilde-project");

        let (out, _) = Output::memory();
        add(&args(&["~/tilde-project"]), &out).unwrap();

        let (out, captured) = Output::memory();
        list(&args(&["--json"]), &out).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text(captured.out)).unwrap();
        assert_eq!(parsed[0]["name"], "tilde-project");
        assert_eq!(
            parsed[0]["path"],
            project.canonicalize().unwrap().display().to_string()
        );
    }

    /// Reading before the app has ever run reports an empty list rather than
    /// failing, and does not create the database.
    #[tokio::test]
    async fn listing_without_a_database_reports_no_workspaces() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();

        let (out, captured) = Output::memory();
        list(&[], &out).unwrap();
        assert!(text(captured.out).contains("No workspaces"));
        assert!(
            !home.database().exists(),
            "a read must not create the desktop app's database"
        );

        let (out, captured) = Output::memory();
        list(&args(&["--json"]), &out).unwrap();
        assert_eq!(text(captured.out).trim(), "[]");
    }

    /// The chat scratch directories share the table; `list` shows only the
    /// user's own workspaces, while a workspace added next to one still lands.
    #[tokio::test]
    async fn list_hides_the_apps_chat_scratch_directories() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let project = home.dir("demo");
        let (out, _) = Output::memory();
        add(&args(&[project.to_str().unwrap()]), &out).unwrap();

        let conn = Connection::open(home.database()).unwrap();
        conn.execute(
            "INSERT INTO workspaces (id, name, kind, path, cleanup_status, created_at, updated_at)
             VALUES ('ws_chat', 'New Chat Workspace', 'temporary', '/tmp/chat', 'active', 9, 9)",
            [],
        )
        .unwrap();
        drop(conn);

        let conn = Connection::open(home.database()).unwrap();
        let listed = user_workspaces(&conn).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "demo");
    }

    /// `--json` is the scriptable form: the row's own fields plus which call
    /// this was and where it was written.
    #[tokio::test]
    async fn add_json_reports_the_record_and_whether_it_was_created() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let project = home.dir("demo");

        let (out, captured) = Output::memory();
        add(&args(&[project.to_str().unwrap(), "--json"]), &out).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text(captured.out)).unwrap();
        assert_eq!(parsed["name"], "demo");
        assert_eq!(parsed["kind"], "user");
        assert_eq!(parsed["created"], true);
        assert_eq!(parsed["database"], home.database().display().to_string());
        assert!(parsed["id"].as_str().unwrap().starts_with("ws-"));

        let (out, captured) = Output::memory();
        add(&args(&[project.to_str().unwrap(), "--json"]), &out).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text(captured.out)).unwrap();
        assert_eq!(parsed["created"], false);
    }

    /// Argument errors are the command's own, and are reported before the
    /// database is touched.
    #[tokio::test]
    async fn bad_arguments_are_refused_before_any_write() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let (out, _) = Output::memory();

        assert!(add(&args(&[]), &out)
            .unwrap_err()
            .contains("path is required"));
        assert!(add(&args(&["/tmp", "--name"]), &out)
            .unwrap_err()
            .contains("--name requires a value"));
        assert!(add(&args(&["/tmp", "/var"]), &out)
            .unwrap_err()
            .contains("too many arguments"));
        assert!(add(&args(&["/tmp", "--nope"]), &out)
            .unwrap_err()
            .contains("unknown option"));
        assert!(list(&args(&["--all"]), &out)
            .unwrap_err()
            .contains("unknown option"));
        assert!(!home.database().exists());
    }

    /// Group dispatch: a bare group prints its help, an unknown subcommand is an
    /// error naming the usage.
    #[tokio::test]
    async fn group_dispatch_prints_help_and_refuses_unknown_subcommands() {
        let (out, captured) = Output::memory();
        workspace(None, &[], &out).unwrap();
        assert_eq!(text(captured.out).trim_end(), help::WORKSPACE_HELP);

        let (out, _) = Output::memory();
        let error = workspace(Some("remove"), &[], &out).unwrap_err();
        assert!(error.contains("Unknown argument: remove"), "{error}");
        assert!(
            error.contains("future workspace [list | add <path>]"),
            "{error}"
        );
    }
}
