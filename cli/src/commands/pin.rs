//! `future session pin|unpin|pinned` and `future workspace pin|unpin` — the
//! desktop app's pinned section, from the terminal.
//!
//! A pin is the app's **shortcut section**, not a second archive flag: pinned
//! conversations and workspaces lead their lists instead of taking part in
//! them, and the flag is an ordering decision rather than an activity one. So
//! both writes leave `updated_at` / `last_opened_at` alone — unlpin a row and it
//! returns to its recency position instead of jumping to the front.
//!
//! Two consequences are worth stating because they look like bugs otherwise:
//!
//! - **Archiving clears the pin.** A shelved row would hide behind the shelf, so
//!   `archive` drops the flag rather than carrying it away. `unpin` is therefore
//!   not the inverse of `pin` in the way `restore` is the inverse of `archive`.
//! - **A pin can be invisible.** The pinned section is a shortcut *out of* the
//!   active list, so a pin on an archived conversation, or on one whose
//!   workspace is archived, is stored but not shown. The write says which of the
//!   two it did instead of reporting a pin the reader will not find.
//!
//! The reads go through the store's own queries (`list_pinned_threads` /
//! `list_pinned_workspaces`) rather than a second copy of their SQL: the app's
//! rail, a paired phone and this command must agree about what "pinned" is.
//! Neither read may create the database — a machine that never ran the app has
//! nothing pinned, which is an empty list, not a failure.

use super::archive::{desktop_store, parse_target, pretty, resolve_workspace, with_flag};
use super::session::{now_ms, pad_end, relative_age, truncate};
use super::workspace::open_existing;
use crate::axi::Format;
use crate::output::Output;
use future_app_workspaces as workspaces;
use future_host::store;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;

type Result<T> = std::result::Result<T, String>;

/// `future session pin|unpin --help`.
pub const SESSION_HELP: &str = r#"future session pin — keep a conversation in the app's pinned section

Usage:
  future session pin <session-id> [--json]
  future session unpin <session-id> [--json]

Pinning lifts a conversation out of the recency list into the desktop app's
pinned section, where it stays above everything else. The target is the Agent
session id the CLI takes everywhere — the one `future session list` prints — and
the conversation the app imported for it is what moves. The write goes to the
store the app reads (~/.future/app/app.db), so the sidebar, the pinned section
and a paired phone see it on their next read; no running desktop app is required.

A pin is an ordering flag, not activity: `updated_at` is left alone, so unpinning
returns the conversation to the position its own activity earns rather than
sending it to the front.

Two things the app's own rules imply, and this command reports rather than
hides:

  - Archiving a conversation or its workspace clears the pin. The pinned section
    is a shortcut out of the active list, not a second shelf, so `unpin` is not
    the inverse of `archive` — `future session restore` is.
  - Pin an archived conversation, or one inside an archived workspace, and the
    flag is stored but the pinned section does not show it until the row is back
    on the active list. The command says so when that is the case.

Examples:
  future session pin 20261104-ab12cd
  future session unpin 20261104-ab12cd

`future session pinned` lists what is pinned right now. With --json, the
conversation record is printed with "pinned" saying whether it is pinned now,
"inPinnedSection" saying whether the pinned section shows it, and the database it
was written to."#;

/// `future workspace pin|unpin --help`.
pub const WORKSPACE_HELP: &str = r#"future workspace pin — keep a workspace in the app's pinned section

Usage:
  future workspace pin <path|id|name> [--json]
  future workspace unpin <path|id|name> [--json]

Pinning lifts a workspace above the unpinned groups in the desktop app, where it
stays instead of following the recency order. The target is what `future
workspace list` prints — an id, or the workspace's name when exactly one carries
it — or a directory path spelled the way `future workspace add` accepts it (~, a
symlinked path, a trailing separator). The write goes to the store the app reads
(~/.future/app/app.db), so the sidebar and a paired phone see it on their next
read; no running desktop app is required.

A pin is an ordering flag, not activity: `updated_at` and `last_opened_at` are
left alone, so unpinning returns the workspace to the position its own activity
earns rather than sending it to the front.

Archiving a workspace clears its pin — the pinned section is a shortcut out of
the active list, not a second shelf — so `unpin` is not the inverse of `archive`.
Pin an archived workspace and the flag is stored, but the section does not show
it until `future workspace restore` brings the workspace back; the command says
so when that is the case.

`future workspace list --pinned` lists what is pinned right now.

Examples:
  future workspace pin demo
  future workspace unpin ws-1a2b3c

With --json, the workspace record is printed with "pinned" saying whether it is
pinned now, "inPinnedSection" saying whether the pinned section shows it, and the
database it was written to."#;

/// `future session pinned --help`.
pub const LIST_HELP: &str = r#"future session pinned — the conversations the app's pinned section holds

Usage:
  future session pinned [--json|--format text|json|toon]

Prints the conversations pinned in the desktop app, in the order the pinned
section draws them — most recent activity first. This is the app's list, not the
Agent's: `future session list` reports every session the Agent holds, including
the ones the app has never imported, and it carries no pin (the Agent has no such
concept). The id printed is the Agent session id, which is what `future session
pin` and `future session unpin` take.

Only conversations the pinned section actually shows are listed: an archived
conversation, or one inside an archived workspace, keeps its flag but is not on
this list until it is back on the active list. A machine that never ran the
desktop app has nothing pinned, which prints as an empty list rather than an
error, and the read never creates the database.

Examples:
  future session pinned
  future session pinned --format toon

With --json, prints {"sessions": [...]} — each entry the store's conversation
record, which is the same shape `future session archive --json` prints."#;

/// `future session pin|unpin <session-id>`.
pub async fn session_command(pinned: bool, args: &[String], out: &Output) -> Result<()> {
    let target = parse_target(args, "session id")?;
    if target.help {
        out.log(SESSION_HELP);
        return Ok(());
    }
    if target.value.is_empty() {
        return Err("a session id is required".into());
    }
    let (conn, database) = desktop_store("pin or unpin")?;
    let thread = workspaces::find_thread_by_agent_session(&conn, &target.value)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| {
            format!(
                "Session {} has no conversation in the desktop app, so there is nothing to {}: \
                 the app records a conversation when it imports the session, and `future \
                 session list` is the Agent's list, which also holds sessions the app has never \
                 opened.",
                target.value,
                if pinned { "pin" } else { "unpin" }
            )
        })?;

    store::pin_thread(store::PinThreadInput {
        thread_id: thread.id.clone(),
        pinned,
    })
    .map_err(|error| error.to_string())?;

    let updated = store::get_thread(&thread.id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the conversation disappeared while it was being updated".to_string())?;
    // "Does the pinned section show it" is answered by the list itself rather
    // than by a second copy of the rule that decides — after the write, that
    // list either contains the row or it does not.
    let shown = pinned
        && pinned_sessions()?
            .rows
            .iter()
            .any(|row| row.id == thread.id);

    if target.json {
        let record = serde_json::to_value(&updated).map_err(|error| error.to_string())?;
        out.log(&pretty(reported(record, pinned, shown, &database))?);
        return Ok(());
    }
    let verb = if pinned { "Pinned" } else { "Unpinned" };
    out.log(&format!(
        "{verb} conversation \"{}\" ({})",
        updated.title, updated.id
    ));
    out.log(&format!("  session: {}", target.value));
    out.log(&format!("  database: {}", database.display()));
    // Only a pin can be hidden: an unpin is off the section by definition, and
    // warning about it would describe the thing the caller just asked for.
    if pinned && !shown {
        out.log(&hidden_conversation_note(&conn, &updated, &database)?);
    }
    Ok(())
}

/// `future workspace pin|unpin <path|id|name>`.
pub async fn workspace_command(pinned: bool, args: &[String], out: &Output) -> Result<()> {
    let target = parse_target(args, "workspace path, id or name")?;
    if target.help {
        out.log(WORKSPACE_HELP);
        return Ok(());
    }
    if target.value.is_empty() {
        return Err("a workspace path, id or name is required".into());
    }
    let (conn, database) = desktop_store("pin or unpin")?;
    let workspace = resolve_workspace(&conn, target.value.trim())?;

    store::pin_workspace(store::PinWorkspaceInput {
        workspace_id: workspace.id.clone(),
        pinned,
    })
    .map_err(|error| error.to_string())?;

    let updated = workspaces::get(&conn, &workspace.id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the workspace disappeared while it was being updated".to_string())?;
    let shown = pinned
        && pinned_workspaces()?
            .iter()
            .any(|row| row.id == workspace.id);

    if target.json {
        let record = serde_json::to_value(&updated).map_err(|error| error.to_string())?;
        out.log(&pretty(reported(record, pinned, shown, &database))?);
        return Ok(());
    }
    let verb = if pinned { "Pinned" } else { "Unpinned" };
    out.log(&format!(
        "{verb} workspace {} ({})",
        updated.name, updated.id
    ));
    out.log(&format!("  path: {}", updated.path));
    out.log(&format!("  database: {}", database.display()));
    if pinned && !shown {
        out.log(&hidden_workspace_note(&updated, &database));
    }
    Ok(())
}

/// The note for a pin the section does not show, with the cause **read** from the
/// store rather than inferred from the missing row.
///
/// A conversation drops out of the pinned section for exactly two reasons, and
/// they call for different commands: its own shelf, or the shelf of the
/// workspace holding it. When neither holds — which the store's rules say cannot
/// happen, but a database another writer touched could — the note says the flag
/// is stored and names no cause instead of inventing one.
fn hidden_conversation_note(
    conn: &rusqlite::Connection,
    thread: &store::ThreadRecord,
    database: &Path,
) -> Result<String> {
    let workspace =
        workspaces::get(conn, &thread.workspace_id).map_err(|error| error.to_string())?;
    let (because, restore) = if thread.status != "active" {
        (
            "this conversation is archived",
            "future session restore <session-id>",
        )
    } else if workspace
        .as_ref()
        .is_some_and(|workspace| workspace.archived_at.is_some())
    {
        (
            "its workspace is archived",
            "future workspace restore <path|id|name>",
        )
    } else {
        return Ok(format!(
            "  note: the pin is stored in {}, but the pinned section does not list this \
             conversation — run `future session pinned` to see what it holds.",
            database.display()
        ));
    };
    Ok(format!(
        "  note: {because}, so the pinned section does not show it until it is restored \
         (`{restore}`); the pin itself is stored in {}.",
        database.display()
    ))
}

/// [`hidden_conversation_note`] for a workspace, which has one reason to be off
/// the section: it is on the shelf itself.
fn hidden_workspace_note(workspace: &workspaces::Workspace, database: &Path) -> String {
    if workspace.archived_at.is_some() {
        format!(
            "  note: this workspace is archived, so the pinned section does not show it until it \
             is restored (`future workspace restore <path|id|name>`); the pin itself is stored in \
             {}.",
            database.display()
        )
    } else {
        format!(
            "  note: the pin is stored in {}, but the pinned section does not list this \
             workspace — run `future workspace list --pinned` to see what it holds.",
            database.display()
        )
    }
}

/// `future session pinned [--json|--format text|json|toon]`.
pub fn session_pinned(args: &[String], out: &Output) -> Result<()> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        out.log(LIST_HELP);
        return Ok(());
    }
    let format = Format::resolve(
        &crate::flags::parse_flags(args, &["--format"], &[], &["--json"])
            .map_err(|error| crate::axi::usage(out, &error, LIST_USAGE))?,
        Format::Text,
    )
    .map_err(|error| crate::axi::usage(out, &error, LIST_USAGE))?;

    let pinned = pinned_sessions()?;
    if format == Format::Json {
        let mut sessions = Vec::with_capacity(pinned.rows.len());
        for row in &pinned.rows {
            let mut record = serde_json::to_value(row).map_err(|error| error.to_string())?;
            if let Some(object) = record.as_object_mut() {
                object.insert(
                    "workspaceName".to_string(),
                    json!(pinned.names.get(&row.workspace_id)),
                );
            }
            sessions.push(record);
        }
        out.log(
            &serde_json::to_string_pretty(&json!({ "sessions": sessions }))
                .map_err(|error| error.to_string())?,
        );
        return Ok(());
    }

    let now = now_ms();
    let rows: Vec<PinnedRow> = pinned
        .rows
        .iter()
        .map(|row| PinnedRow {
            session: row
                .agent_session_id
                .clone()
                .unwrap_or_else(|| row.id.clone()),
            title: row.title.clone(),
            updated: relative_age(row.last_message_at.unwrap_or(row.updated_at), now),
            workspace: pinned
                .names
                .get(&row.workspace_id)
                .cloned()
                .unwrap_or_default(),
        })
        .collect();

    if format == Format::Toon {
        let rows: Vec<Vec<String>> = rows
            .iter()
            .map(|row| {
                vec![
                    row.session.clone(),
                    row.title.clone(),
                    row.updated.clone(),
                    row.workspace.clone(),
                ]
            })
            .collect();
        out.log(&crate::axi::toon_rows("pinned", PINNED_FIELDS, &rows));
        return Ok(());
    }

    if rows.is_empty() {
        crate::axi::empty_state(out, EMPTY_SESSIONS);
        pinned_hints(out);
        return Ok(());
    }
    crate::axi::count_line(out, "", rows.len());
    // The session id is last and never cut: it is the value the reader copies
    // into `future session unpin`, and a cut id cannot be pasted anywhere.
    out.log(&format!(
        "  {} {} {} {}",
        pad_end("TITLE", 38),
        pad_end("UPDATED", 10),
        pad_end("WORKSPACE", 18),
        "SESSION ID"
    ));
    out.log(&format!(
        "  {} {} {} {}",
        "—".repeat(38),
        "—".repeat(10),
        "—".repeat(18),
        "—".repeat(36)
    ));
    for row in &rows {
        out.log(&format!(
            "  {} {} {} {}",
            pad_end(&truncate(&row.title, 42), 38),
            pad_end(&row.updated, 10),
            pad_end(&truncate(&row.workspace, 22), 18),
            row.session
        ));
    }
    pinned_hints(out);
    Ok(())
}

/// Usage line the format errors point at.
const LIST_USAGE: &str = "Usage: future session pinned [--json|--format text|json|toon]";

/// What a machine with nothing pinned is told — the pin command named, because
/// an empty list is the state you change rather than read.
pub(super) const EMPTY_SESSIONS: &str =
    "No pinned conversations. Pin one with `future session pin <session-id>`.";

/// What `future workspace list --pinned` says when the section is empty.
pub(super) const EMPTY_WORKSPACES: &str =
    "No pinned workspaces. Pin one with `future workspace pin <path|id|name>`.";

/// The fields `--format toon` reports: what picking a pinned conversation needs.
const PINNED_FIELDS: &[&str] = &["session", "title", "updated", "workspace"];

/// One rendered row, shared by the table and the TOON view.
struct PinnedRow {
    session: String,
    title: String,
    updated: String,
    workspace: String,
}

fn pinned_hints(out: &Output) {
    crate::axi::hints(
        out,
        &[
            "Run `future session unpin <session-id>` to take one off the pinned section.",
            "Run `future session pin <session-id>` to add one.",
        ],
    );
}

/// The pinned conversations, in the order the pinned section draws them, plus
/// the workspace names their rows do not carry.
pub(super) struct PinnedSessions {
    pub(super) rows: Vec<store::ThreadRecord>,
    pub(super) names: HashMap<String, String>,
}

/// Read the pinned conversations through the store's own definition of them.
///
/// A store the app never created, or one with no `threads` table yet (a database
/// `future workspace add` made before the app first ran), means nothing is
/// pinned — and neither case may create or migrate the file, so the existence
/// check runs first on the read-only connection.
pub(super) fn pinned_sessions() -> Result<PinnedSessions> {
    let empty = || PinnedSessions {
        rows: Vec::new(),
        names: HashMap::new(),
    };
    let Some(conn) = open_existing()? else {
        return Ok(empty());
    };
    if !workspaces::threads_table_exists(&conn).map_err(|error| error.to_string())? {
        return Ok(empty());
    }
    let names = workspaces::list(&conn)
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|workspace| (workspace.id, workspace.name))
        .collect();
    // `Recent` is the app's own answer to "every pinned conversation": the
    // shortcut section is drawn from the active list, not from one workspace.
    let rows = store::list_pinned_threads(&store::ThreadScope::Recent)
        .map_err(|error| error.to_string())?;
    Ok(PinnedSessions { rows, names })
}

/// The pinned user workspaces, whole — the same query the app's rail and a
/// paired phone read. Empty when the app has never created its store.
pub(super) fn pinned_workspaces() -> Result<Vec<store::WorkspaceRecord>> {
    if open_existing()?.is_none() {
        return Ok(Vec::new());
    }
    store::list_pinned_workspaces().map_err(|error| error.to_string())
}

/// The record plus the two facts a script needs after a write: whether the flag
/// is set now, and whether the pinned section actually shows the row.
fn reported(record: Value, pinned: bool, shown: bool, database: &Path) -> Value {
    let mut record = with_flag(record, "pinned", json!(pinned), database);
    if let Some(object) = record.as_object_mut() {
        object.insert("inPinnedSection".to_string(), json!(shown));
    }
    record
}

#[cfg(test)]
mod tests {
    use super::super::test_store::{
        add_thread, add_workspace, args, memory, pinned, set_activity, text, threads_stamps,
        workspace_stamps, Store,
    };
    use super::*;
    use crate::test_env::EnvGuard;
    use rusqlite::Connection;

    /// The pinned section is the app's shortcut row, and pinning is an ordering
    /// flag: the row moves, its activity stamps do not.
    #[tokio::test]
    async fn pinning_a_workspace_sets_the_flag_and_moves_no_stamp() {
        let _guard = crate::test_env::lock_env().await;
        let store = Store::new();
        let project = store.directory("demo");
        let workspace = add_workspace(&store, &project);
        let before = workspace_stamps(&store, &workspace.id);

        let (out, captured) = memory();
        workspace_command(true, &args(&[project.to_str().unwrap()]), &out)
            .await
            .expect("pin");
        let printed = text(captured.out);
        assert!(printed.contains("Pinned workspace demo"), "{printed}");
        assert!(
            printed.contains(&store.database().display().to_string()),
            "the write names the store it landed in: {printed}"
        );
        assert!(pinned(&store, "workspaces", &workspace.id));
        assert_eq!(
            workspace_stamps(&store, &workspace.id),
            before,
            "a pin is an ordering flag, not activity"
        );

        let (out, captured) = memory();
        workspace_command(false, &args(&[&workspace.id]), &out)
            .await
            .expect("unpin");
        assert!(text(captured.out).contains("Unpinned workspace demo"));
        assert!(!pinned(&store, "workspaces", &workspace.id));
        assert_eq!(workspace_stamps(&store, &workspace.id), before);

        // Unpinning an unpinned workspace is the same state, not an error.
        let (out, _) = memory();
        workspace_command(false, &args(&[&workspace.id]), &out)
            .await
            .expect("idempotent unpin");
    }

    /// The same at conversation scope, resolved through the Agent session id the
    /// CLI takes everywhere.
    #[tokio::test]
    async fn pinning_a_conversation_sets_the_flag_and_moves_no_stamp() {
        let _guard = crate::test_env::lock_env().await;
        let store = Store::new();
        let project = store.directory("demo");
        let workspace = add_workspace(&store, &project);
        let thread = add_thread(&store, &workspace.id, "sess-1", "Fix the parser");
        let before = threads_stamps(&store, &thread);

        let (out, captured) = memory();
        session_command(true, &args(&["sess-1"]), &out)
            .await
            .expect("pin");
        let printed = text(captured.out);
        assert!(
            printed.contains("Pinned conversation \"Fix the parser\""),
            "{printed}"
        );
        assert!(printed.contains("session: sess-1"), "{printed}");
        assert!(pinned(&store, "threads", &thread));
        assert_eq!(threads_stamps(&store, &thread), before);

        let (out, _) = memory();
        session_command(false, &args(&["sess-1"]), &out)
            .await
            .expect("unpin");
        assert!(!pinned(&store, "threads", &thread));
        assert_eq!(threads_stamps(&store, &thread), before);
    }

    /// `--json` carries the flag, whether the pinned section shows the row, and
    /// the database — the three facts a script needs after a write the running
    /// app only shows on its next read.
    #[tokio::test]
    async fn json_reports_the_pin_and_the_store() {
        let _guard = crate::test_env::lock_env().await;
        let store = Store::new();
        let project = store.directory("demo");
        let workspace = add_workspace(&store, &project);
        add_thread(&store, &workspace.id, "sess-1", "Fix the parser");

        let (out, captured) = memory();
        workspace_command(true, &args(&[&workspace.id, "--json"]), &out)
            .await
            .expect("pin");
        let parsed: Value = serde_json::from_str(&text(captured.out)).unwrap();
        assert_eq!(parsed["id"], workspace.id);
        assert_eq!(parsed["pinned"], true);
        assert_eq!(parsed["inPinnedSection"], true);
        assert_eq!(parsed["database"], store.database().display().to_string());

        let (out, captured) = memory();
        session_command(true, &args(&["sess-1", "--json"]), &out)
            .await
            .expect("pin");
        let parsed: Value = serde_json::from_str(&text(captured.out)).unwrap();
        assert_eq!(parsed["pinned"], true);
        assert_eq!(parsed["inPinnedSection"], true);

        let (out, captured) = memory();
        session_command(false, &args(&["sess-1", "--json"]), &out)
            .await
            .expect("unpin");
        let parsed: Value = serde_json::from_str(&text(captured.out)).unwrap();
        assert_eq!(parsed["pinned"], false);
        assert_eq!(parsed["inPinnedSection"], false);
    }

    /// A pin on a row the pinned section does not draw is stored and *said* to be
    /// invisible: archiving clears the flag, so a pin taken afterwards is a
    /// different fact from a row appearing in the section.
    #[tokio::test]
    async fn a_pin_the_section_does_not_show_says_so() {
        let _guard = crate::test_env::lock_env().await;
        let store = Store::new();
        let project = store.directory("demo");
        let workspace = add_workspace(&store, &project);
        let thread = add_thread(&store, &workspace.id, "sess-1", "Fix the parser");
        {
            let conn = store.conn();
            conn.execute(
                "UPDATE threads SET status = 'archived', archived_at = 2 WHERE id = ?1",
                [&thread],
            )
            .unwrap();
        }

        let (out, captured) = memory();
        session_command(true, &args(&["sess-1"]), &out)
            .await
            .expect("pin an archived conversation");
        let printed = text(captured.out);
        assert!(printed.contains("Pinned conversation"), "{printed}");
        assert!(printed.contains("does not show it"), "{printed}");
        assert!(
            printed.contains("future session restore"),
            "the note names the command that brings it back: {printed}"
        );
        assert!(pinned(&store, "threads", &thread), "the flag is stored");
        assert!(pinned_sessions().unwrap().rows.is_empty(), "but not drawn");

        // A *live* conversation whose workspace is archived is the other way to
        // be hidden, and the note has to name the other command. The cause is
        // read from the store, so the two are never confused.
        {
            let conn = store.conn();
            conn.execute(
                "UPDATE threads SET status = 'active', archived_at = NULL WHERE id = ?1",
                [&thread],
            )
            .unwrap();
            conn.execute(
                "UPDATE workspaces SET archived_at = 3 WHERE id = ?1",
                [&workspace.id],
            )
            .unwrap();
        }
        let (out, captured) = memory();
        session_command(true, &args(&["sess-1"]), &out)
            .await
            .expect("pin under a shelved workspace");
        let printed = text(captured.out);
        assert!(printed.contains("its workspace is archived"), "{printed}");
        assert!(printed.contains("future workspace restore"), "{printed}");
        assert!(!printed.contains("future session restore"), "{printed}");

        // Archiving clears the pin, so the two directions are not inverses.
        {
            let conn = store.conn();
            conn.execute(
                "UPDATE workspaces SET archived_at = NULL WHERE id = ?1",
                [&workspace.id],
            )
            .unwrap();
        }
        let (out, _) = memory();
        crate::commands::archive::session_command(false, &args(&["sess-1"]), &out)
            .await
            .expect("archive");
        assert!(
            !pinned(&store, "threads", &thread),
            "archive clears the pin rather than carrying it onto the shelf"
        );
    }

    /// An unpin is off the section by definition, so it must not warn about a
    /// pin being hidden — that note describes the thing the caller just asked
    /// for. Measured against the real store: unpinning a live workspace printed
    /// "its workspace is archived" because the cause was inferred from the
    /// missing row instead of read.
    #[tokio::test]
    async fn an_unpin_never_warns_about_a_hidden_pin() {
        let _guard = crate::test_env::lock_env().await;
        let store = Store::new();
        let project = store.directory("demo");
        let workspace = add_workspace(&store, &project);
        let thread = add_thread(&store, &workspace.id, "sess-1", "Fix the parser");

        let (out, _) = memory();
        workspace_command(true, &args(&[&workspace.id]), &out)
            .await
            .expect("pin");
        let (out, _) = memory();
        session_command(true, &args(&["sess-1"]), &out)
            .await
            .expect("pin");

        // A live workspace, unpinned: no note.
        let (out, captured) = memory();
        workspace_command(false, &args(&[&workspace.id]), &out)
            .await
            .expect("unpin");
        let printed = text(captured.out);
        assert!(!printed.contains("note:"), "{printed}");
        assert!(printed.contains("Unpinned workspace demo"), "{printed}");

        // A live conversation, unpinned: no note either.
        let (out, captured) = memory();
        session_command(false, &args(&["sess-1"]), &out)
            .await
            .expect("unpin");
        let printed = text(captured.out);
        assert!(!printed.contains("note:"), "{printed}");
        assert!(printed.contains("Unpinned conversation"), "{printed}");
        assert!(!pinned(&store, "threads", &thread));

        // And a *pin* on a live row shows no note, because the section does
        // show it — the note is not a blanket message on every write.
        let (out, captured) = memory();
        session_command(true, &args(&["sess-1"]), &out)
            .await
            .expect("pin");
        assert!(!text(captured.out).contains("note:"));
    }

    /// The listing is the app's own query: conversation rows in the order the
    /// pinned section draws them, with the workspace each one sits in.
    #[tokio::test]
    async fn the_pinned_list_reports_the_section_in_its_own_order() {
        let _guard = crate::test_env::lock_env().await;
        let store = Store::new();
        let project = store.directory("demo");
        let workspace = add_workspace(&store, &project);
        let older = add_thread(&store, &workspace.id, "sess-old", "Older");
        let newer = add_thread(&store, &workspace.id, "sess-new", "Newer");
        set_activity(&store, &older, 100);
        set_activity(&store, &newer, 900);
        for session in ["sess-old", "sess-new"] {
            let (out, _) = memory();
            session_command(true, &args(&[session]), &out)
                .await
                .expect("pin");
        }
        // An unpinned conversation must not appear.
        add_thread(&store, &workspace.id, "sess-idle", "Idle");

        let (out, captured) = memory();
        session_pinned(&[], &out).expect("pinned");
        let printed = text(captured.out);
        assert!(printed.contains("count: 2"), "{printed}");
        let newer_at = printed.find("Newer").expect("newer row");
        let older_at = printed.find("Older").expect("older row");
        assert!(newer_at < older_at, "most recent activity leads: {printed}");
        assert!(printed.contains("sess-new"), "{printed}");
        assert!(!printed.contains("Idle"), "{printed}");
        assert!(
            printed.contains("demo"),
            "the workspace is named: {printed}"
        );

        let (out, captured) = memory();
        session_pinned(&args(&["--json"]), &out).expect("pinned --json");
        let parsed: Value = serde_json::from_str(&text(captured.out)).unwrap();
        let sessions = parsed["sessions"].as_array().unwrap();
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0]["agentSessionId"], "sess-new");
        assert_eq!(sessions[0]["workspaceName"], "demo");
    }

    /// `future workspace list --pinned` is the app's workspace query, whose order
    /// is the workspace's own recency rather than its conversations'.
    #[tokio::test]
    async fn the_pinned_workspace_shelf_lists_what_is_pinned() {
        let _guard = crate::test_env::lock_env().await;
        let store = Store::new();
        let demo = add_workspace(&store, &store.directory("demo"));
        let service = add_workspace(&store, &store.directory("service"));

        let (out, captured) = memory();
        super::super::workspace::workspace(Some("list"), &args(&["--pinned"]), &out)
            .await
            .expect("empty pinned shelf");
        let printed = text(captured.out);
        assert!(printed.contains("No pinned workspaces"), "{printed}");

        for id in [&demo.id, &service.id] {
            let (out, _) = memory();
            workspace_command(true, &args(&[id]), &out)
                .await
                .expect("pin");
        }
        // The conversation activity that orders the plain list must not decide
        // this one: `list_pinned_workspaces` orders by the workspace's own
        // recency, and a pin never writes that.
        set_activity(
            &store,
            &add_thread(&store, &service.id, "sess-1", "Work"),
            9999,
        );

        let (out, captured) = memory();
        super::super::workspace::workspace(Some("list"), &args(&["--pinned"]), &out)
            .await
            .expect("pinned shelf");
        let printed = text(captured.out);
        assert!(printed.contains("count: 2"), "{printed}");
        assert!(printed.contains("demo"), "{printed}");
        assert!(printed.contains("service"), "{printed}");

        let (out, captured) = memory();
        super::super::workspace::workspace(Some("list"), &args(&["--pinned", "--json"]), &out)
            .await
            .expect("pinned shelf --json");
        let parsed: Value = serde_json::from_str(&text(captured.out)).unwrap();
        assert_eq!(parsed.as_array().unwrap().len(), 2);
        assert!(parsed[0]["pinned"].as_bool().unwrap());

        // Unpinning takes it off the shelf.
        let (out, _) = memory();
        workspace_command(false, &args(&[&demo.id]), &out)
            .await
            .expect("unpin");
        let (out, captured) = memory();
        session_pinned(&args(&["--json"]), &out).expect("pinned conversations");
        super::super::workspace::workspace(Some("list"), &args(&["--pinned"]), &out)
            .await
            .expect("pinned shelf");
        let printed = text(captured.out);
        assert!(printed.contains(&service.id), "{printed}");
    }

    /// A name is ambiguous when two workspaces carry it, and guessing would pin
    /// whichever row sorted first.
    #[tokio::test]
    async fn an_ambiguous_workspace_name_is_refused() {
        let _guard = crate::test_env::lock_env().await;
        let store = Store::new();
        let first = add_workspace(&store, &store.directory("demo"));
        let second = add_workspace(&store, &store.directory("nested/demo"));

        let (out, _) = memory();
        let error = workspace_command(true, &args(&["demo"]), &out)
            .await
            .unwrap_err();
        assert!(
            error.contains("More than one workspace is named"),
            "{error}"
        );
        assert!(error.contains(&first.id), "{error}");
        assert!(error.contains(&second.id), "{error}");
        assert!(!pinned(&store, "workspaces", &first.id));
        assert!(!pinned(&store, "workspaces", &second.id));

        // A unique name still works, which is the point of accepting one.
        let (out, _) = memory();
        workspace_command(true, &args(&["service"]), &out)
            .await
            .unwrap_err();
        let service = add_workspace(&store, &store.directory("service"));
        let (out, _) = memory();
        workspace_command(true, &args(&["service"]), &out)
            .await
            .expect("pin by unique name");
        assert!(pinned(&store, "workspaces", &service.id));
    }

    /// A session the app never imported has no conversation to pin, and the
    /// refusal says which list the id came from.
    #[tokio::test]
    async fn a_session_the_app_never_imported_is_refused_by_name() {
        let _guard = crate::test_env::lock_env().await;
        let _store = Store::new();

        let (out, _) = memory();
        let error = session_command(true, &args(&["sess-unknown"]), &out)
            .await
            .unwrap_err();
        assert!(
            error.contains("no conversation in the desktop app"),
            "{error}"
        );
        assert!(error.contains("sess-unknown"), "{error}");
    }

    /// Reads and writes both refuse on a machine that never ran the app, and
    /// neither creates the database to have nothing to report in it.
    #[tokio::test]
    async fn a_machine_without_the_app_store_gets_an_empty_list_and_a_refusal() {
        let _guard = crate::test_env::lock_env().await;
        let home = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set(&[("HOME", home.path().as_os_str().to_owned())]);

        let (out, _) = memory();
        let error = session_command(true, &args(&["sess-1"]), &out)
            .await
            .unwrap_err();
        assert!(error.contains("No desktop app store"), "{error}");
        let (out, _) = memory();
        let error = workspace_command(true, &args(&["demo"]), &out)
            .await
            .unwrap_err();
        assert!(error.contains("No desktop app store"), "{error}");

        // The reads are empty states, not failures.
        let (out, captured) = memory();
        session_pinned(&[], &out).expect("pinned");
        let printed = text(captured.out);
        assert!(printed.contains("count: 0"), "{printed}");
        assert!(printed.contains("No pinned conversations"), "{printed}");
        let (out, captured) = memory();
        session_pinned(&args(&["--json"]), &out).expect("pinned --json");
        assert_eq!(text(captured.out).trim(), "{\n  \"sessions\": []\n}");
        let (out, captured) = memory();
        super::super::workspace::workspace(Some("list"), &args(&["--pinned", "--json"]), &out)
            .await
            .expect("pinned shelf --json");
        assert_eq!(text(captured.out).trim(), "[]");

        assert!(
            !home
                .path()
                .join(".future")
                .join("app")
                .join("app.db")
                .exists(),
            "a read must never create the app's database"
        );
    }

    /// A store `future workspace add` created before the desktop app first ran
    /// has no `threads` table: the conversation list is empty rather than an
    /// error about a missing table.
    #[tokio::test]
    async fn a_store_without_the_threads_table_lists_nothing() {
        let _guard = crate::test_env::lock_env().await;
        let home = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set(&[("HOME", home.path().as_os_str().to_owned())]);
        // Only the workspaces table, which is what a bare `workspace add` writes.
        let path = home.path().join(".future").join("app");
        std::fs::create_dir_all(&path).unwrap();
        let conn = Connection::open(path.join("app.db")).unwrap();
        workspaces::ensure_table(&conn).unwrap();
        drop(conn);

        let (out, captured) = memory();
        session_pinned(&[], &out).expect("pinned");
        assert!(text(captured.out).contains("No pinned conversations"));
        let (out, captured) = memory();
        super::super::workspace::workspace(Some("list"), &args(&["--pinned"]), &out)
            .await
            .expect("pinned shelf");
        assert!(text(captured.out).contains("No pinned workspaces"));
    }

    /// Argument errors are the command's own, refused before any write.
    #[tokio::test]
    async fn bad_arguments_are_refused_before_any_write() {
        let _guard = crate::test_env::lock_env().await;
        let store = Store::new();
        let project = store.directory("demo");
        let workspace = add_workspace(&store, &project);

        let (out, _) = memory();
        assert!(workspace_command(true, &[], &out)
            .await
            .unwrap_err()
            .contains("path, id or name is required"));
        let (out, _) = memory();
        assert!(workspace_command(true, &args(&["a", "b"]), &out)
            .await
            .unwrap_err()
            .contains("too many arguments"));
        let (out, _) = memory();
        assert!(workspace_command(true, &args(&["a", "--nope"]), &out)
            .await
            .unwrap_err()
            .contains("unknown option"));
        let (out, _) = memory();
        assert!(session_command(true, &[], &out)
            .await
            .unwrap_err()
            .contains("session id is required"));
        let (out, _) = memory();
        assert!(session_pinned(&args(&["--nope"]), &out).is_err());

        assert!(!pinned(&store, "workspaces", &workspace.id));
    }

    /// `--help` is a read: it prints the page and touches no store, so it works
    /// on a machine with no desktop app at all.
    #[tokio::test]
    async fn help_needs_no_store() {
        let _guard = crate::test_env::lock_env().await;
        let home = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set(&[("HOME", home.path().as_os_str().to_owned())]);

        let (out, captured) = memory();
        session_command(true, &args(&["--help"]), &out)
            .await
            .expect("help");
        assert_eq!(text(captured.out), format!("{SESSION_HELP}\n"));
        let (out, captured) = memory();
        workspace_command(false, &args(&["-h"]), &out)
            .await
            .expect("help");
        assert_eq!(text(captured.out), format!("{WORKSPACE_HELP}\n"));
        let (out, captured) = memory();
        session_pinned(&args(&["--help"]), &out).expect("help");
        assert_eq!(text(captured.out), format!("{LIST_HELP}\n"));
    }
}
