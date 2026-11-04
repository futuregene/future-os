//! `future workspace archive|restore` and `future session archive|restore` —
//! the desktop app's Archived view, from the terminal.
//!
//! Shelving is the same operation at two scopes. A **workspace** leaves the
//! sidebar and the workspace pickers, and the conversations filed under it go
//! with it — each keeps its own status, so restoring the workspace brings back
//! exactly what it hid. A **conversation** leaves the sidebar together with its
//! forked tree, which is the grouping the app's own Archive action uses.
//!
//! Both write the app's records (`~/.future/app/app.db`), through the same store
//! the app calls, so the archived view, the sidebar and a paired phone see the
//! change on their next read and the rules cannot drift: archiving refuses while
//! anything inside is still running (a shelved run would hide its approval
//! request behind a shelf), and it clears the pin, because the pinned section is
//! an active-list shortcut rather than a second archive flag.
//!
//! Neither needs a running desktop app. A running one is still where the visual
//! change appears, so the write is worth nothing if it lands in a database the
//! app does not read — which is why both commands report the store they wrote.

use super::workspace::{open_existing, user_workspaces};
use crate::output::Output;
use crate::rpc::{grpc_addr, RunClient};
use future_app_workspaces as workspaces;
use future_host::store;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::Path;

type Result<T> = std::result::Result<T, String>;

/// `future workspace archive|restore --help`.
pub const WORKSPACE_HELP: &str = r#"future workspace archive — shelve a workspace and its conversations

Usage:
  future workspace archive <path|id|name> [--json]
  future workspace restore <path|id|name> [--json]

Archiving moves a workspace to the desktop app's Archived view: it leaves the
sidebar and the workspace pickers, and the conversations filed under it go with
it. Every conversation keeps its own status, so restoring the workspace brings
back exactly what it hid — a conversation shelved on its own stays shelved.

The target is what `future workspace list` prints — an id, or the workspace's
name when exactly one carries it — or a directory path spelled the way `future
workspace add` accepts it (~, a symlinked path, a trailing separator). The write
goes to the store the app reads (~/.future/app/app.db), so the sidebar, the
archived view and a paired phone see it on their next read. No running desktop
app is required.

Archiving refuses while a conversation inside the workspace is still running —
a shelved run would hide its approval request — and it clears the workspace's
pin, the same rules the app itself applies. Restoring is the exact inverse and
is idempotent.

`unarchive` is accepted as another spelling of `restore`.

Examples:
  future workspace archive demo
  future workspace archive ~/projects/demo
  future workspace restore ws-1a2b3c
  future workspace list --archived

With --json, the workspace record is printed with "archived" saying whether it
is shelved now, plus the database it was written to."#;

/// `future session archive|restore --help`.
pub const SESSION_HELP: &str = r#"future session archive — shelve a conversation in the desktop app

Usage:
  future session archive <session-id> [--json]
  future session restore <session-id> [--json]

Archiving applies the desktop app's Archived view to the conversation bound to
this Agent session: it leaves the sidebar and the archived list holds it until
it is restored. A forked conversation goes with its parent — the grouping the
app's own Archive action uses when it shelves a fork tree.

The id is one `future session list` prints. A session the desktop app has never
imported has no conversation to shelve, and this says so: the CLI's session list
is the Agent's, and the shelf is the app's.

Archiving refuses while anything in the tree is still running — a shelved run
would hide its approval request — and it clears the conversation's pin.
Restoring is the exact inverse and is idempotent; it needs no running Agent.

`unarchive` is accepted as another spelling of `restore`.

Examples:
  future session archive 20261104-ab12cd
  future session restore 20261104-ab12cd

With --json, the conversation record is printed with "archived" saying whether
it is shelved now, plus the database it was written to."#;

/// A parsed `<target> [--json]` argument list, shared by both scopes.
struct Target {
    value: String,
    json: bool,
    help: bool,
}

fn parse_target(args: &[String], what: &str) -> Result<Target> {
    let mut target = Target {
        value: String::new(),
        json: false,
        help: false,
    };
    let mut seen = false;
    for argument in args {
        match argument.as_str() {
            "--json" => target.json = true,
            "-h" | "--help" => target.help = true,
            other if other.starts_with('-') => {
                return Err(format!("unknown option: {other}; see --help"));
            }
            other if seen => {
                return Err(format!(
                    "too many arguments: expected one {what}, also got {other}"
                ));
            }
            other => {
                target.value = other.to_string();
                seen = true;
            }
        }
    }
    Ok(target)
}

/// `future workspace archive|restore <path|id|name>`.
pub async fn workspace_command(restore: bool, args: &[String], out: &Output) -> Result<()> {
    let target = parse_target(args, "workspace path, id or name")?;
    if target.help {
        out.log(WORKSPACE_HELP);
        return Ok(());
    }
    if target.value.is_empty() {
        return Err("a workspace path, id or name is required".into());
    }
    let (conn, database) = desktop_store()?;
    let workspace = resolve_workspace(&conn, target.value.trim())?;

    if restore {
        // The inverse is one row edit and nothing inside can be running in a
        // way that matters: un-hiding is never the dangerous direction.
        workspaces::restore(&conn, &workspace.id, workspaces::now_millis())
            .map_err(|error| error.to_string())?;
    } else {
        archive_workspace(&conn, &workspace).await?;
    }

    let updated = workspaces::get(&conn, &workspace.id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the workspace disappeared while it was being updated".to_string())?;
    let verb = if restore { "Restored" } else { "Archived" };
    if target.json {
        let record = serde_json::to_value(&updated).map_err(|error| error.to_string())?;
        out.log(&pretty(with_store(
            record,
            updated.archived_at.is_some(),
            &database,
        ))?);
        return Ok(());
    }
    out.log(&format!(
        "{verb} workspace {} ({})",
        updated.name, updated.id
    ));
    out.log(&format!("  path: {}", updated.path));
    out.log(&format!("  database: {}", database.display()));
    Ok(())
}

/// `future session archive|restore <session-id>`.
pub async fn session_command(restore: bool, args: &[String], out: &Output) -> Result<()> {
    let target = parse_target(args, "session id")?;
    if target.help {
        out.log(SESSION_HELP);
        return Ok(());
    }
    if target.value.is_empty() {
        return Err("a session id is required".into());
    }
    let (conn, database) = desktop_store()?;
    let thread = workspaces::find_thread_by_agent_session(&conn, &target.value)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| {
            format!(
                "Session {} has no conversation in the desktop app, so there is nothing to {}: \
                 the app records a conversation when it imports the session, and `future \
                 session list` is the Agent's list, which also holds sessions the app has never \
                 opened.",
                target.value,
                if restore { "restore" } else { "archive" }
            )
        })?;

    if restore {
        store::restore_thread_tree(&thread.id).map_err(|error| error.to_string())?;
    } else {
        let streaming = streaming_thread_ids().await;
        store::archive_thread_tree(&thread.id, &streaming).map_err(|error| error.to_string())?;
    }

    let updated = store::get_thread(&thread.id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the conversation disappeared while it was being updated".to_string())?;
    let archived = updated.status == "archived";
    let verb = if restore { "Restored" } else { "Archived" };
    if target.json {
        let record = serde_json::to_value(&updated).map_err(|error| error.to_string())?;
        out.log(&pretty(with_store(record, archived, &database))?);
        return Ok(());
    }
    out.log(&format!(
        "{verb} conversation \"{}\" ({})",
        updated.title, updated.id
    ));
    out.log(&format!("  session: {}", target.value));
    out.log(&format!("  database: {}", database.display()));
    Ok(())
}

/// The desktop app's store, or the reason there is nothing to shelve.
fn desktop_store() -> Result<(Connection, std::path::PathBuf)> {
    let conn = open_existing()?.ok_or_else(|| {
        "No desktop app store on this machine (~/.future/app/app.db), so there is nothing to \
         archive or restore. The app creates it on its first start."
            .to_string()
    })?;
    let database = future_app_settings::app_db_path().map_err(|error| error.to_string())?;
    Ok((conn, database))
}

/// A workspace id as `future workspace list` prints it, a directory path as
/// `future workspace add` accepts it, or the workspace's name — the three
/// spellings a reader of that list has at hand.
///
/// A name resolves only when exactly one workspace carries it: guessing between
/// two identically named workspaces would shelve whichever row happened to sort
/// first.
fn resolve_workspace(conn: &Connection, target: &str) -> Result<workspaces::Workspace> {
    if let Ok(Some(workspace)) = workspaces::get(conn, target) {
        if workspace.kind == "user" && workspace.deleted_at.is_none() {
            return Ok(workspace);
        }
    }
    let directory = workspaces::expand_tilde(target).map_err(|error| error.to_string())?;
    if let Some(workspace) =
        workspaces::find_user_by_path(conn, &directory).map_err(|error| error.to_string())?
    {
        return Ok(workspace);
    }
    let mut named = user_workspaces(conn)?
        .into_iter()
        .filter(|workspace| workspace.name == target);
    match (named.next(), named.next()) {
        (Some(workspace), None) => Ok(workspace),
        (Some(first), Some(second)) => Err(format!(
            "More than one workspace is named {target:?} ({} and {}); pass the id instead.",
            first.id, second.id
        )),
        _ => Err(format!(
            "No workspace matches {target:?} as an id, a name or a directory path. Run \
             `future workspace list` to see the stored ones, or `future workspace add <path>` \
             to file conversations under this directory."
        )),
    }
}

/// Shelve one workspace, with the app's own precondition.
///
/// A store where `future workspace add` ran before the desktop app ever started
/// has no conversations — and no `threads` table for the precondition to read —
/// so the shared row rule is the whole rule there.
async fn archive_workspace(conn: &Connection, workspace: &workspaces::Workspace) -> Result<()> {
    if !workspaces::threads_table_exists(conn).map_err(|error| error.to_string())? {
        workspaces::archive(conn, &workspace.id, workspaces::now_millis())
            .map_err(|error| error.to_string())?;
        return Ok(());
    }
    let streaming = streaming_thread_ids().await;
    store::archive_workspace(&workspace.id, &streaming).map_err(|error| error.to_string())?;
    Ok(())
}

/// The Agent's resident streaming sessions, as local conversation ids.
///
/// The archive precondition needs them: a run another client started has no
/// local `runs` row, so shelving it would hide a live request the store cannot
/// see. An Agent that cannot be reached reports nothing streaming — the same
/// tolerance the app's sidebar uses, and sound here because an unreachable
/// Agent is not running anything this CLI could have started either.
async fn streaming_thread_ids() -> Vec<String> {
    let client = RunClient::new(&grpc_addr());
    let Ok(sessions) = client.list_streaming_sessions().await else {
        return Vec::new();
    };
    sessions
        .get("sessionIds")
        .and_then(Value::as_array)
        .map(|ids| ids.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|session_id| {
            store::find_thread_by_agent_session(session_id)
                .ok()
                .flatten()
        })
        .map(|thread| thread.id)
        .collect()
}

/// The record plus where it was written — the two facts a script needs after a
/// write that the running app will only show on its next read.
fn with_store(record: Value, archived: bool, database: &Path) -> Value {
    let mut object = record.as_object().cloned().unwrap_or_default();
    object.insert("archived".to_string(), json!(archived));
    object.insert(
        "database".to_string(),
        json!(database.display().to_string()),
    );
    Value::Object(object)
}

fn pretty(record: Value) -> Result<String> {
    serde_json::to_string_pretty(&record).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env::EnvGuard;
    use std::path::PathBuf;

    /// A throwaway home with the desktop app's real schema in it (`HOME`, not
    /// `FUTURE_HOME`: the app's database lives under the real home).
    struct Store {
        dir: tempfile::TempDir,
        _env: EnvGuard,
    }

    impl Store {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            let env = EnvGuard::set(&[("HOME", dir.path().as_os_str().to_owned())]);
            store::initialize_app_store().expect("the desktop app's schema");
            Store { dir, _env: env }
        }

        fn database(&self) -> PathBuf {
            self.dir.path().join(".future").join("app").join("app.db")
        }

        fn conn(&self) -> Connection {
            Connection::open(self.database()).expect("open store")
        }

        fn directory(&self, name: &str) -> PathBuf {
            let path = self.dir.path().join(name);
            std::fs::create_dir_all(&path).expect("create directory");
            path
        }
    }

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn text(captured: std::sync::Arc<std::sync::Mutex<Vec<u8>>>) -> String {
        String::from_utf8(captured.lock().unwrap().clone()).expect("utf8")
    }

    /// A workspace row created the way `future workspace add` creates one.
    fn add_workspace(store: &Store, path: &Path) -> workspaces::Workspace {
        let mut conn = store.conn();
        workspaces::create(
            &mut conn,
            path.to_str().unwrap(),
            None,
            None,
            false,
            workspaces::now_millis(),
        )
        .expect("workspace")
    }

    /// A conversation imported into that workspace, as the app records one.
    fn add_thread(store: &Store, workspace_id: &str, session_id: &str, title: &str) -> String {
        let thread_id = format!("thr-{session_id}");
        let conn = store.conn();
        conn.execute(
            "INSERT INTO threads (id, workspace_id, mode, title, status, agent_session_id,
                                  created_at, updated_at)
             VALUES (?1, ?2, 'workspace', ?3, 'active', ?4, 1, 1)",
            rusqlite::params![thread_id, workspace_id, title, session_id],
        )
        .expect("thread row");
        thread_id
    }

    fn archived_at(store: &Store, workspace_id: &str) -> Option<i64> {
        let conn = store.conn();
        conn.query_row(
            "SELECT archived_at FROM workspaces WHERE id = ?1",
            [workspace_id],
            |row| row.get(0),
        )
        .expect("read archived_at")
    }

    fn thread_status(store: &Store, thread_id: &str) -> String {
        let conn = store.conn();
        conn.query_row(
            "SELECT status FROM threads WHERE id = ?1",
            [thread_id],
            |row| row.get(0),
        )
        .expect("read status")
    }

    /// The workspace archive writes the app's own marker and reports the store
    /// it wrote, and restore takes the marker away again.
    #[tokio::test]
    async fn archiving_and_restoring_a_workspace_flips_the_marker() {
        let _guard = crate::test_env::lock_env().await;
        let store_home = Store::new();
        let project = store_home.directory("demo");
        let workspace = add_workspace(&store_home, &project);

        let (out, captured) = Output::memory();
        workspace_command(false, &args(&[project.to_str().unwrap()]), &out)
            .await
            .expect("archive");
        let printed = text(captured.out);
        assert!(printed.contains("Archived workspace demo"), "{printed}");
        assert!(
            printed.contains(&store_home.database().display().to_string()),
            "the write names the store it landed in: {printed}"
        );
        assert!(archived_at(&store_home, &workspace.id).is_some());

        let (out, captured) = Output::memory();
        workspace_command(true, &args(&[&workspace.id]), &out)
            .await
            .expect("restore");
        assert!(text(captured.out).contains("Restored workspace demo"));
        assert!(archived_at(&store_home, &workspace.id).is_none());

        // Restoring an active workspace is the same, not an error.
        let (out, _) = Output::memory();
        workspace_command(true, &args(&[&workspace.id]), &out)
            .await
            .expect("idempotent restore");
    }

    /// `--json` is the scriptable form: the row's own fields, the shelf it ended
    /// up on, and the database it was written to.
    #[tokio::test]
    async fn json_reports_the_record_and_the_shelf_it_landed_on() {
        let _guard = crate::test_env::lock_env().await;
        let store_home = Store::new();
        let project = store_home.directory("demo");
        let workspace = add_workspace(&store_home, &project);

        let (out, captured) = Output::memory();
        workspace_command(false, &args(&[project.to_str().unwrap(), "--json"]), &out)
            .await
            .expect("archive");
        let parsed: Value = serde_json::from_str(&text(captured.out)).unwrap();
        assert_eq!(parsed["id"], workspace.id);
        assert_eq!(parsed["name"], "demo");
        assert_eq!(parsed["archived"], true);
        assert_eq!(parsed["kind"], "user");
        assert_eq!(
            parsed["database"],
            store_home.database().display().to_string()
        );

        let (out, captured) = Output::memory();
        workspace_command(true, &args(&[&workspace.id, "--json"]), &out)
            .await
            .expect("restore");
        let parsed: Value = serde_json::from_str(&text(captured.out)).unwrap();
        assert_eq!(parsed["archived"], false);
    }

    /// A workspace with a conversation still running is refused with the
    /// store's own message, naming the conversation, and nothing is written.
    #[tokio::test]
    async fn a_running_conversation_keeps_the_workspace_off_the_shelf() {
        let _guard = crate::test_env::lock_env().await;
        let store_home = Store::new();
        let project = store_home.directory("demo");
        let workspace = add_workspace(&store_home, &project);
        add_thread(&store_home, &workspace.id, "sess-1", "Fix the parser");
        {
            let conn = store_home.conn();
            conn.execute(
                "INSERT INTO runs (id, thread_id, status, created_at, updated_at)
                 VALUES ('run-1', 'thr-sess-1', 'running', 1, 1)",
                [],
            )
            .expect("run row");
        }

        let (out, _) = Output::memory();
        let error = workspace_command(false, &args(&[&workspace.id]), &out)
            .await
            .unwrap_err();
        assert!(error.contains("Fix the parser"), "{error}");
        assert!(error.contains("still running"), "{error}");
        assert!(archived_at(&store_home, &workspace.id).is_none());
    }

    /// Archiving a conversation shelves its forked tree, and restoring brings
    /// the whole tree back — the grouping the app's own action uses.
    #[tokio::test]
    async fn a_conversation_archive_covers_its_forked_tree() {
        let _guard = crate::test_env::lock_env().await;
        let store_home = Store::new();
        let project = store_home.directory("demo");
        let workspace = add_workspace(&store_home, &project);
        let parent = add_thread(&store_home, &workspace.id, "sess-1", "Parent");
        // A fork records the *parent's* Agent session id, not its thread id.
        {
            let conn = store_home.conn();
            conn.execute(
                "INSERT INTO threads (id, workspace_id, mode, title, status, agent_session_id,
                                      parent_session_id, created_at, updated_at)
                 VALUES ('thr-fork', ?1, 'workspace', 'Fork', 'active', 'sess-2', 'sess-1', 2, 2)",
                [&workspace.id],
            )
            .expect("fork row");
        }

        let (out, captured) = Output::memory();
        session_command(false, &args(&["sess-1"]), &out)
            .await
            .expect("archive");
        let printed = text(captured.out);
        assert!(
            printed.contains("Archived conversation \"Parent\""),
            "{printed}"
        );
        assert!(printed.contains("session: sess-1"), "{printed}");
        assert!(
            printed.contains(&store_home.database().display().to_string()),
            "{printed}"
        );
        assert_eq!(thread_status(&store_home, &parent), "archived");
        assert_eq!(thread_status(&store_home, "thr-fork"), "archived");

        let (out, _) = Output::memory();
        session_command(true, &args(&["sess-1"]), &out)
            .await
            .expect("restore");
        assert_eq!(thread_status(&store_home, &parent), "active");
        assert_eq!(thread_status(&store_home, "thr-fork"), "active");
    }

    /// A session the desktop app never imported has no conversation to shelve,
    /// and the refusal says which list the id came from instead of pretending
    /// the archive failed for another reason.
    #[tokio::test]
    async fn a_session_the_app_never_imported_is_refused_by_name() {
        let _guard = crate::test_env::lock_env().await;
        let _store_home = Store::new();

        let (out, _) = Output::memory();
        let error = session_command(false, &args(&["sess-unknown"]), &out)
            .await
            .unwrap_err();
        assert!(
            error.contains("no conversation in the desktop app"),
            "{error}"
        );
        assert!(error.contains("sess-unknown"), "{error}");
    }

    /// Argument errors are the command's own: they are reported before the
    /// store is touched, and each scope names the argument it wanted.
    #[tokio::test]
    async fn bad_arguments_are_refused_before_any_write() {
        let _guard = crate::test_env::lock_env().await;
        let store_home = Store::new();

        let (out, _) = Output::memory();
        assert!(workspace_command(false, &[], &out)
            .await
            .unwrap_err()
            .contains("path, id or name is required"));
        assert!(workspace_command(false, &args(&["a", "b"]), &out)
            .await
            .unwrap_err()
            .contains("too many arguments"));
        assert!(workspace_command(false, &args(&["a", "--nope"]), &out)
            .await
            .unwrap_err()
            .contains("unknown option"));
        assert!(session_command(false, &[], &out)
            .await
            .unwrap_err()
            .contains("session id is required"));
        assert!(session_command(false, &args(&["--nope"]), &out)
            .await
            .unwrap_err()
            .contains("unknown option"));

        // Nothing was shelved, and no workspace was invented for the paths.
        let conn = store_home.conn();
        let archived: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM workspaces WHERE archived_at IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(archived, 0);
    }

    /// A name is the first column of `future workspace list`, so it is a target
    /// too — but only when it names one workspace: two rows with the same name
    /// is an error naming the ids, not a guess.
    #[tokio::test]
    async fn a_name_resolves_only_when_it_is_unambiguous() {
        let _guard = crate::test_env::lock_env().await;
        let store_home = Store::new();
        let first = add_workspace(&store_home, &store_home.directory("demo"));

        let (out, captured) = Output::memory();
        workspace_command(false, &args(&["demo"]), &out)
            .await
            .expect("archive by name");
        assert!(text(captured.out).contains("Archived workspace demo"));
        assert!(archived_at(&store_home, &first.id).is_some());

        // A second directory with the same leaf name makes "demo" ambiguous.
        let second = add_workspace(&store_home, &store_home.directory("nested/demo"));
        let (out, _) = Output::memory();
        let error = workspace_command(true, &args(&["demo"]), &out)
            .await
            .unwrap_err();
        assert!(
            error.contains("More than one workspace is named"),
            "{error}"
        );
        assert!(error.contains(&first.id), "{error}");
        assert!(error.contains(&second.id), "{error}");
        assert!(archived_at(&store_home, &first.id).is_some());
    }

    /// A machine that never ran the desktop app has no store, and both scopes
    /// say so rather than creating one to archive nothing in.
    #[tokio::test]
    async fn a_machine_without_the_app_store_is_told_so() {
        let _guard = crate::test_env::lock_env().await;
        let home = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set(&[("HOME", home.path().as_os_str().to_owned())]);

        let (out, _) = Output::memory();
        let error = workspace_command(false, &args(&["demo"]), &out)
            .await
            .unwrap_err();
        assert!(error.contains("No desktop app store"), "{error}");
        let error = session_command(false, &args(&["sess-1"]), &out)
            .await
            .unwrap_err();
        assert!(error.contains("No desktop app store"), "{error}");
        assert!(!home
            .path()
            .join(".future")
            .join("app")
            .join("app.db")
            .exists());
    }

    /// An unknown target stays an error rather than archiving the nearest
    /// thing: a path that is not a workspace and an id that matches no row.
    #[tokio::test]
    async fn an_unknown_workspace_names_both_ways_to_fix_it() {
        let _guard = crate::test_env::lock_env().await;
        let store_home = Store::new();
        let project = store_home.directory("demo");
        add_workspace(&store_home, &project);

        let (out, _) = Output::memory();
        let error = workspace_command(false, &args(&["ws-not-here"]), &out)
            .await
            .unwrap_err();
        assert!(error.contains("No workspace matches"), "{error}");
        assert!(error.contains("future workspace list"), "{error}");

        // A directory that exists but has no workspace is not archived either.
        let other = store_home.directory("other");
        let (out, _) = Output::memory();
        let error = workspace_command(false, &args(&[other.to_str().unwrap()]), &out)
            .await
            .unwrap_err();
        assert!(error.contains("No workspace matches"), "{error}");
    }

    /// `--help` is answered without touching the store, so it works before the
    /// app has ever run.
    #[tokio::test]
    async fn help_needs_no_store() {
        let _guard = crate::test_env::lock_env().await;
        let home = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set(&[("HOME", home.path().as_os_str().to_owned())]);

        let (out, captured) = Output::memory();
        workspace_command(false, &args(&["--help"]), &out)
            .await
            .expect("help");
        assert_eq!(text(captured.out).trim_end(), WORKSPACE_HELP);

        let (out, captured) = Output::memory();
        session_command(false, &args(&["--help"]), &out)
            .await
            .expect("help");
        assert_eq!(text(captured.out).trim_end(), SESSION_HELP);
    }
}
