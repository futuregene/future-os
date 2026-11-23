//! The desktop app's workspace records.
//!
//! A *workspace* is the directory a workspace conversation is filed under (plus
//! the chat scratch directories the app manages itself). The rows live in the
//! `workspaces` table of `~/.future/app/app.db`, which the Tauri desktop app
//! owns. This crate is the single source of truth for that table's shape and for
//! the user-workspace rules — identity by canonical path, name derivation, id
//! shape — so that both writers, the desktop backend
//! (`desktop/src-tauri/src/store/workspaces.rs`) and `future workspace` in the
//! CLI, cannot drift apart.
//!
//! Nothing here is the *agent's* notion of a working directory: a session's cwd
//! is an agent-side property, and it is the desktop's import that derives a
//! workspace row from it. This crate owns the desktop's own bookkeeping only.
//!
//! Every function takes a `&rusqlite::Connection`, because the caller owns the
//! connection and therefore the transaction it may want to run inside: the
//! desktop's pooled connection and the CLI's one-shot connection are served by
//! the same code.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Errors from reading, resolving or writing a workspace row.
#[derive(Debug)]
pub enum Error {
    Database(rusqlite::Error),
    Io(std::io::Error),
    /// A path or name this crate cannot accept. The message is written for a
    /// user.
    Invalid(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Database(error) => write!(formatter, "{error}"),
            Error::Io(error) => write!(formatter, "{error}"),
            Error::Invalid(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Database(error) => Some(error),
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

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Error::Io(error)
    }
}

/// One row of `workspaces`, as the desktop serializes it (camelCase).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub id: String,
    pub name: String,
    /// `user` (a directory the user chose) or `temporary` (app-managed chat
    /// scratch).
    pub kind: String,
    pub path: String,
    pub description: Option<String>,
    pub pinned: bool,
    pub cleanup_status: String,
    pub cleanup_requested_at: Option<i64>,
    pub cleaned_at: Option<i64>,
    pub last_opened_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub archived_at: Option<i64>,
    pub deleted_at: Option<i64>,
}

/// Comma-separated `SELECT` column list, in [`from_row`]'s order.
pub const COLUMNS: &str = "id, name, kind, path, description, pinned, cleanup_status, \
                            cleanup_requested_at, cleaned_at, last_opened_at, created_at, \
                            updated_at, archived_at, deleted_at";

/// Map a row selected with [`COLUMNS`] into a [`Workspace`].
pub fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Workspace> {
    Ok(Workspace {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: row.get(2)?,
        path: row.get(3)?,
        description: row.get(4)?,
        pinned: row.get(5)?,
        cleanup_status: row.get(6)?,
        cleanup_requested_at: row.get(7)?,
        cleaned_at: row.get(8)?,
        last_opened_at: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
        archived_at: row.get(12)?,
        deleted_at: row.get(13)?,
    })
}

/// Create the `workspaces` table when it does not exist, so a first write from
/// the CLI does not require the desktop app to have run. Idempotent: the
/// desktop's own schema is created first whenever the app has started, and its
/// migration state is left untouched by this. The uniqueness rule below is
/// installed here for the same reason.
pub fn ensure_table(conn: &Connection) -> Result<(), Error> {
    future_app_settings::validate_database_header(conn)
        .map_err(|error| Error::Invalid(error.to_string()))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS workspaces (
             id TEXT PRIMARY KEY,
             name TEXT NOT NULL,
             kind TEXT NOT NULL CHECK (kind IN ('user', 'temporary')),
             path TEXT NOT NULL,
             description TEXT,
             pinned INTEGER NOT NULL DEFAULT 0,
             cleanup_status TEXT NOT NULL DEFAULT 'active',
             cleanup_requested_at INTEGER,
             cleaned_at INTEGER,
             last_opened_at INTEGER,
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             archived_at INTEGER,
             deleted_at INTEGER
         )",
    )?;
    ensure_unique_user_path_index(conn)?;
    Ok(())
}

/// The constraint behind "one directory is one workspace": at most one live
/// `user` row per path. A chat scratch workspace is a different kind of row, and
/// a deleted row is history, so neither is covered.
pub const UNIQUE_USER_PATH_INDEX: &str =
    "CREATE UNIQUE INDEX IF NOT EXISTS idx_workspaces_user_path_unique \
     ON workspaces(path) WHERE kind = 'user' AND deleted_at IS NULL";

/// Install [`UNIQUE_USER_PATH_INDEX`], and say whether it is in place.
///
/// A database written before [`find_or_create_user`] guarded its insert can
/// already hold two rows for one directory, and SQLite refuses a unique index
/// over such a table. That is not a reason to fail the open: the constraint is
/// left out, `find_or_create_user` keeps the rule on its own, and the pair stays
/// where it is — merging it means moving conversations between ids and dropping
/// a row, which is the user's decision, not a migration's.
pub fn ensure_unique_user_path_index(conn: &Connection) -> Result<bool, Error> {
    match conn.execute_batch(UNIQUE_USER_PATH_INDEX) {
        Ok(()) => Ok(true),
        Err(rusqlite::Error::SqliteFailure(error, _))
            if error.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            Ok(false)
        }
        Err(error) => Err(error.into()),
    }
}

/// Every live workspace row, most recently *active* first — the newest
/// conversation filed under a workspace counts as its activity, not only the
/// workspace's own open/edit timestamps. Both kinds are returned; a caller that
/// wants only the user's own directories filters on [`Workspace::kind`].
pub fn list(conn: &Connection) -> Result<Vec<Workspace>, Error> {
    // Which workspace did the user work in last is answered by a conversation
    // inside it as much as by opening the workspace itself. `last_opened_at`
    // moves only on the latter, so ordering by it alone ranks a workspace the
    // user has spent the day in below one they merely created: the pickers that
    // read this order (the phone's new-conversation dialog, the Desktop's own
    // workspace list) exist to offer the one used last. The threads table
    // belongs to the Desktop, so a database the CLI created before any
    // conversation existed falls back to the workspace's own timestamps.
    let order = if threads_table_exists(conn)? {
        "COALESCE((SELECT MAX(COALESCE(t.last_message_at, t.updated_at, t.created_at))
                    FROM threads t
                   WHERE t.workspace_id = workspaces.id AND t.status != 'deleted'),
                  last_opened_at, updated_at) DESC"
    } else {
        "COALESCE(last_opened_at, updated_at) DESC"
    };
    let mut statement = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM workspaces WHERE deleted_at IS NULL ORDER BY {order}"
    ))?;
    let rows = statement.query_map([], from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// One workspace by id, or `None` when it does not exist.
pub fn get(conn: &Connection, workspace_id: &str) -> Result<Option<Workspace>, Error> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM workspaces WHERE id = ?1"),
            params![workspace_id],
            from_row,
        )
        .optional()?)
}

/// Shelve a user workspace: hidden from the default lists, containers and
/// conversations untouched. Only `kind = 'user'` workspaces are archivable — a
/// chat's temporary scratch workspace lives and dies with its conversation.
pub fn archive(conn: &Connection, workspace_id: &str, now: i64) -> Result<Workspace, Error> {
    let updated = conn.execute(
        "UPDATE workspaces SET archived_at = ?1, updated_at = ?1
         WHERE id = ?2 AND kind = 'user' AND deleted_at IS NULL",
        params![now, workspace_id],
    )?;
    if updated == 0 {
        return Err(Error::Invalid(
            "Workspace is not available for archiving.".to_string(),
        ));
    }
    get(conn, workspace_id)?
        .ok_or_else(|| Error::Invalid("Workspace could not be loaded.".to_string()))
}

/// The inverse of [`archive`]: the workspace (and with it every conversation it
/// was hiding) is back in the default lists.
pub fn restore(conn: &Connection, workspace_id: &str, now: i64) -> Result<Workspace, Error> {
    let updated = conn.execute(
        "UPDATE workspaces SET archived_at = NULL, updated_at = ?1
         WHERE id = ?2 AND deleted_at IS NULL",
        params![now, workspace_id],
    )?;
    if updated == 0 {
        return Err(Error::Invalid(
            "Workspace could not be restored.".to_string(),
        ));
    }
    get(conn, workspace_id)?
        .ok_or_else(|| Error::Invalid("Workspace could not be loaded.".to_string()))
}

/// The user workspace that owns a directory, or `None` when it has none.
///
/// A client's spelling is never identity: the stored spelling is matched first
/// (one indexed lookup), and only when that misses are the other rows compared
/// by canonical path — which is how a row stored under an older aliasing
/// spelling (`/tmp/x` versus `/private/tmp/x` on macOS, a trailing separator, a
/// symlinked project directory) is still found instead of a second workspace
/// being created for the same directory.
pub fn find_user_by_path(conn: &Connection, path: &Path) -> Result<Option<Workspace>, Error> {
    let normalized = normalize_path(path);
    let stored = conn
        .query_row(
            &format!(
                "SELECT {COLUMNS} FROM workspaces
                 WHERE kind = 'user' AND path = ?1 AND deleted_at IS NULL
                 LIMIT 1"
            ),
            params![normalized.display().to_string()],
            from_row,
        )
        .optional()?;
    if stored.is_some() {
        return Ok(stored);
    }

    let mut statement = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM workspaces WHERE kind = 'user' AND deleted_at IS NULL"
    ))?;
    let rows = statement.query_map([], from_row)?;
    for row in rows {
        let row = row?;
        if normalize_path(Path::new(&row.path)) == normalized {
            return Ok(Some(row));
        }
    }
    Ok(None)
}

/// Resolve the workspace for a directory, creating it when the directory has
/// none. The caller may own a transaction (a composite write such as the
/// desktop's `create_thread` runs this inside one), but does not have to: "one
/// directory is one workspace" is enforced by the insert below, not by the
/// transaction the caller happens to hold.
pub fn find_or_create_user(
    conn: &Connection,
    name: String,
    path: &Path,
    description: Option<String>,
    now: i64,
) -> Result<Workspace, Error> {
    if let Some(workspace) = find_user_by_path(conn, path)? {
        return reopened(conn, &workspace.id, now);
    }

    // The decision and the write are one statement, so no second writer can
    // land a row for this directory between them. A lookup followed by a
    // separate `INSERT` cannot be made safe that way: whatever transaction the
    // caller holds, the window only closes for writers that *also* took the
    // write lock before reading, and the cwd filing
    // (`file_session_for_cwd`) read before it wrote. That is how the user's
    // database ended up with two workspaces for one directory, 1 ms apart
    // (`ws-20261116-141317-…-e62d87` and `…-05aa4d`, 2026-11-16 14:13:17).
    //
    // `?3` is the stored spelling, which every writer canonicalizes first, so
    // two spellings of one directory meet here as one path.
    let workspace_id = new_id("ws");
    let stored = normalize_path(path).display().to_string();
    let inserted = conn.execute(
        "INSERT INTO workspaces (
             id, name, kind, path, description, cleanup_status, last_opened_at,
             created_at, updated_at
         )
         SELECT ?1, ?2, 'user', ?3, ?4, 'active', ?5, ?5, ?5
          WHERE NOT EXISTS (
                SELECT 1 FROM workspaces
                 WHERE kind = 'user' AND path = ?3 AND deleted_at IS NULL
          )",
        params![workspace_id, name, stored, description, now],
    )?;

    if inserted == 0 {
        // A creator committed while this call was looking. Its row is the
        // workspace for the directory; this call adopts it instead of adding
        // one of its own.
        let existing = find_user_by_path(conn, path)?.ok_or_else(|| {
            Error::Invalid("Workspace could not be loaded after a concurrent write.".to_string())
        })?;
        return reopened(conn, &existing.id, now);
    }

    get(conn, &workspace_id)?
        .ok_or_else(|| Error::Invalid("Created workspace could not be loaded.".to_string()))
}

/// Hand back a workspace that already exists, waking it first: new work landing
/// in an archived workspace means the directory is in use again, so the
/// container comes back. Archiving hides a workspace until something writes
/// into it, never disables its directory.
fn reopened(conn: &Connection, workspace_id: &str, now: i64) -> Result<Workspace, Error> {
    wake_workspace(conn, workspace_id, now)?;
    get(conn, workspace_id)?
        .ok_or_else(|| Error::Invalid("Workspace could not be loaded.".to_string()))
}

/// Clear a workspace's archived marker. Idempotent; a no-op for a workspace
/// that is not archived. The one place "the container is in use again" is
/// recorded (a session filed into it, or a conversation created inside it).
pub fn wake_workspace(conn: &Connection, workspace_id: &str, now: i64) -> Result<(), Error> {
    conn.execute(
        "UPDATE workspaces SET archived_at = NULL, updated_at = ?1
         WHERE id = ?2 AND archived_at IS NOT NULL",
        params![now, workspace_id],
    )?;
    Ok(())
}

/// The full create-workspace rule both writers share: `~` is expanded, the
/// directory must already exist unless `create_directory` says to make it, the
/// name defaults to the directory's own name, and a directory that already has
/// a workspace reopens that row instead of gaining a second one.
///
/// "One directory, one workspace" is enforced by
/// [`find_or_create_user`]'s single `INSERT … WHERE NOT EXISTS` statement, which
/// holds even for a writer that read before another creator committed — the
/// case that produced two rows for one directory at 2026-11-16 14:13:17. The
/// `BEGIN IMMEDIATE` here adds the ordering, not the guarantee: it makes the
/// reopen path's wake part of the same unit as the lookup, and it keeps this
/// writer behind any other write transaction instead of failing on it.
pub fn create(
    conn: &mut Connection,
    path: &str,
    name: Option<String>,
    description: Option<String>,
    create_directory: bool,
    now: i64,
) -> Result<Workspace, Error> {
    let path = resolve_directory(path, create_directory)?;
    let name = match name {
        Some(name) => name,
        None => name_from_path(&path),
    };
    let transaction = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let workspace = find_or_create_user(&transaction, name, &path, description, now)?;
    transaction.commit()?;
    Ok(workspace)
}

/// The desktop conversation behind an Agent session: the row a cwd change is
/// filed under. Only the two columns the filing needs travel, so this stays a
/// read of the `threads` table rather than a second copy of its shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadRef {
    pub id: String,
    pub workspace_id: String,
}

/// Whether the desktop's `threads` table exists in this database.
///
/// The CLI can create `workspaces` in a database the app has never opened
/// (`future workspace add` before the app ever ran), and the app creates the
/// rest of its schema at first start — so "the table is not there" is a normal
/// state meaning "the app owns no conversations yet", not a broken store.
pub fn threads_table_exists(conn: &Connection) -> Result<bool, Error> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'threads'",
        [],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

/// The live conversation bound to an Agent session, or `None` when the desktop
/// has never imported it (or has deleted it).
pub fn find_thread_by_agent_session(
    conn: &Connection,
    session_id: &str,
) -> Result<Option<ThreadRef>, Error> {
    if !threads_table_exists(conn)? {
        return Ok(None);
    }
    Ok(conn
        .query_row(
            "SELECT id, workspace_id FROM threads
             WHERE agent_session_id = ?1 AND status != 'deleted' LIMIT 1",
            params![session_id],
            |row| {
                Ok(ThreadRef {
                    id: row.get(0)?,
                    workspace_id: row.get(1)?,
                })
            },
        )
        .optional()?)
}

/// File a conversation under a workspace.
///
/// A conversation under a *user* workspace is a **workspace conversation**, so
/// `mode` moves with it. The two clients group and label by `mode`, never by
/// `workspace_id` (`desktop/src/components/layout/ActivityRail.tsx`, the
/// palette subtitle in `AppShell.tsx`, `mobile/src/screens/sessionTree.ts`),
/// and neither ever rewrites `mode` after creation. A row that kept
/// `mode = 'chat'` while its `workspace_id` named a user workspace therefore
/// stayed under "Chat" for good — the state a `future session set --cwd` used
/// to leave behind, which reads as "the workspace never changed".
///
/// The reverse is deliberately not undone here: a workspace conversation
/// pointed at a chat scratch directory keeps its mode, because demoting it
/// would need a temporary workspace that conversation never had.
pub fn move_thread_to_workspace(
    conn: &Connection,
    thread_id: &str,
    workspace_id: &str,
) -> Result<(), Error> {
    // A store old enough to predate `mode` is left with the workspace half
    // alone rather than failing: migrations refuse such a layout outright
    // (`host/src/store/migrations.rs`), so the app would not start on it
    // anyway, and a cwd change must not fail over a column that cannot be set.
    if !thread_has_mode_column(conn)? {
        conn.execute(
            "UPDATE threads SET workspace_id = ?1, updated_at = ?2
             WHERE id = ?3 AND status != 'deleted'",
            params![workspace_id, now_millis(), thread_id],
        )?;
        return Ok(());
    }
    conn.execute(
        "UPDATE threads SET workspace_id = ?1, mode = 'workspace', updated_at = ?2
         WHERE id = ?3 AND status != 'deleted'",
        params![workspace_id, now_millis(), thread_id],
    )?;
    Ok(())
}

/// Whether the `threads` table carries the `mode` column.
fn thread_has_mode_column(conn: &Connection) -> Result<bool, Error> {
    let mut statement = conn.prepare("PRAGMA table_info(threads)")?;
    let mut names = statement.query_map([], |row| row.get::<_, String>(1))?;
    Ok(names.any(|name| name.is_ok_and(|name| name == "mode")))
}

/// The app-managed chat scratch root: `~/.future/workspaces/chat`. A chat
/// conversation's own temporary directory lives under it and is named after the
/// session once the first prompt has run.
pub fn chat_workspaces_root() -> Result<PathBuf, Error> {
    let app = future_app_settings::app_dir().map_err(|error| Error::Invalid(error.to_string()))?;
    let root = app
        .parent()
        .ok_or_else(|| Error::Invalid("Desktop app directory has no data root".into()))?;
    Ok(root.join("workspaces").join("chat"))
}

/// Whether a cwd is a chat scratch directory (the agent-side spelling of the
/// app's own scratch root). A tilde spelling counts, as does a Windows one.
pub fn is_chat_directory(cwd: &str) -> bool {
    is_chat_directory_at(cwd, chat_workspaces_root())
}

/// [`is_chat_directory`] with the scratch root injected. A root that cannot be
/// resolved (no usable FutureOS home) is not a chat directory: claiming one
/// would move a project's conversation into a scratch directory nobody owns.
fn is_chat_directory_at(cwd: &str, root: Result<PathBuf, Error>) -> bool {
    if cwd.is_empty() {
        return false;
    }
    let cwd = cwd.replace('\\', "/");
    let Ok(root) = root else {
        return false;
    };
    let prefix = format!(
        "{}/",
        root.to_string_lossy()
            .replace('\\', "/")
            .trim_end_matches('/')
    );
    if cwd.starts_with(&prefix) {
        return true;
    }
    // Preserve the legacy tilde spelling only when it names this root.
    home_dir().is_ok_and(|home| root == home.join(".future/workspaces/chat"))
        && cwd.starts_with("~/.future/workspaces/chat/")
}

/// What filing a session's conversation for a cwd change did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CwdFiling {
    /// The desktop has never imported this session. There is nothing to file
    /// yet: the workspace follows the directory when the app imports it.
    NotImported,
    /// A chat scratch cwd: the conversation's temporary workspace follows the
    /// path, as the app does for its own chat conversations.
    ChatWorkspaceUpdated,
    /// Filed under a workspace created for the directory.
    WorkspaceCreated(String),
    /// Filed under the workspace that already covered the directory.
    WorkspaceReused(String),
}

/// File a conversation under the workspace for `cwd`, the rule the desktop app
/// applies when it sees a cwd change — the same code, so a cwd changed from the
/// CLI and one changed in the app land identically whether or not the app was
/// running to notice it.
///
/// A directory that is not a chat scratch directory files the conversation
/// under a user workspace, which makes it a *workspace* conversation: `mode`
/// moves with `workspace_id` (see [`move_thread_to_workspace`]), because both
/// clients group by `mode` and would otherwise keep it under "Chat" forever.
///
/// The directory must already exist: an uncreatable path is `Err`, which is how
/// the app surfaces it (a toast) and what the CLI reports as a note rather than
/// failing the cwd change itself. An empty cwd is `Ok(None)`: there is no
/// directory to file under.
///
/// The filing is one unit: the workspace it resolves or creates, the thread's
/// move into it and the wake all commit together, inside a `BEGIN IMMEDIATE`
/// transaction. It used to write outside any transaction, which left its "no
/// workspace for this directory yet" reading stale by the time it inserted —
/// the second writer in the 2026-11-16 14:13:17 duplicate.
pub fn file_session_for_cwd(
    conn: &mut Connection,
    session_id: &str,
    cwd: &str,
    now: i64,
) -> Result<Option<CwdFiling>, Error> {
    let cwd = cwd.trim().trim_end_matches(['/', '\\']);
    if cwd.is_empty() {
        return Ok(None);
    }
    // The answer that writes nothing is decided before the write lock is taken,
    // so a cwd change the desktop is not filing costs a read.
    let Some(thread) = find_thread_by_agent_session(conn, session_id)? else {
        return Ok(Some(CwdFiling::NotImported));
    };
    let transaction = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let filing = file_known_thread(&transaction, &thread, cwd, now)?;
    transaction.commit()?;
    Ok(Some(filing))
}

/// [`file_session_for_cwd`] for a session the desktop has already imported: the
/// workspace half of the rule, on the connection the caller's transaction gives
/// it.
fn file_known_thread(
    conn: &Connection,
    thread: &ThreadRef,
    cwd: &str,
    now: i64,
) -> Result<CwdFiling, Error> {
    if is_chat_directory(cwd) {
        let scratch = chat_workspaces_root()?
            .join(&thread.id)
            .display()
            .to_string();
        if scratch != cwd {
            conn.execute(
                "UPDATE workspaces SET path = ?1, updated_at = ?2
                 WHERE path = ?3 AND kind = 'temporary'",
                params![cwd, now, scratch],
            )?;
        }
        return Ok(CwdFiling::ChatWorkspaceUpdated);
    }

    // Identity is the canonical directory, not the spelling that arrived: the
    // same rule `create` applies, so both spellings find one workspace.
    let path = resolve_directory(cwd, false)?;
    let name = name_from_path(&path);
    let filing = match find_user_by_path(conn, &path)? {
        Some(workspace) => CwdFiling::WorkspaceReused(workspace.id),
        None => CwdFiling::WorkspaceCreated(find_or_create_user(conn, name, &path, None, now)?.id),
    };
    let workspace_id = match &filing {
        CwdFiling::WorkspaceCreated(id) | CwdFiling::WorkspaceReused(id) => id.clone(),
        _ => unreachable!("the chat case returned above"),
    };
    move_thread_to_workspace(conn, &thread.id, &workspace_id)?;
    // A conversation filed by cwd change is new work in that directory: an
    // archived workspace is woken so the row does not end up JOIN-hidden.
    wake_workspace(conn, &workspace_id, now)?;
    Ok(filing)
}

/// The `archived_at` marker of a workspace, when it is archived.
pub fn archived_at(conn: &Connection, workspace_id: &str) -> Result<Option<i64>, Error> {
    Ok(conn
        .query_row(
            "SELECT archived_at FROM workspaces WHERE id = ?1",
            params![workspace_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten())
}

/// Milliseconds since the epoch, the stamp every row write carries.
pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

/// The directory a workspace path names, refusing one that is missing or is not
/// a directory unless `create_directory` says to make it.
pub fn resolve_directory(path: &str, create_directory: bool) -> Result<PathBuf, Error> {
    let path = expand_tilde(path)?;
    if create_directory {
        std::fs::create_dir_all(&path)?;
    } else if !path.is_dir() {
        return Err(Error::Invalid(format!(
            "Workspace path does not exist or is not a directory: {}",
            path.display()
        )));
    }
    Ok(path)
}

/// `~` and `~/…` against the home directory, everything else unchanged.
pub fn expand_tilde(path: &str) -> Result<PathBuf, Error> {
    if path == "~" || path.starts_with("~/") {
        let home = home_dir()?;
        return Ok(match path.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None => home,
        });
    }
    Ok(PathBuf::from(path))
}

/// `$HOME`, else `$USERPROFILE`, and only when it is non-empty and absolute.
pub fn home_dir() -> Result<PathBuf, Error> {
    [
        std::env::var("HOME").ok(),
        std::env::var("USERPROFILE").ok(),
    ]
    .into_iter()
    .flatten()
    .find(|value| !value.is_empty() && Path::new(value).is_absolute())
    .map(PathBuf::from)
    .ok_or_else(|| Error::Invalid("HOME/USERPROFILE environment variable is not set.".to_string()))
}

/// Identity spelling of a workspace directory: the canonical path when it can
/// be resolved, and the given path otherwise — a workspace may legitimately be
/// stored before its directory exists.
pub fn normalize_path(path: &Path) -> PathBuf {
    match path.canonicalize() {
        Ok(canonical) => strip_verbatim_prefix(canonical),
        Err(_) => path.to_path_buf(),
    }
}

/// Windows `Path::canonicalize` returns the extended-length spelling
/// (`\\?\D:\...`), which no other tool prints; store the ordinary form instead
/// (`\\?\UNC\server\share` → `\\server\share`). A canonical POSIX path never
/// carries the prefix, so this is purely textual and a no-op off Windows.
pub fn strip_verbatim_prefix(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => path,
    }
}

/// The default workspace name: the directory's own name.
pub fn name_from_path(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Workspace")
        .to_string()
}

/// A fresh row id (`ws-20260924-141714-3fa9c1b7d204e86f`). The shape matches the
/// desktop's other ids: a prefix, local time, and a random suffix.
///
/// The suffix is the id's *whole* collision budget for anything minted inside
/// the same second — the timestamp above it is identical for the whole burst —
/// so its width is a correctness property and not a style choice. Three random
/// bytes made a burst of a few hundred rows collide about once in a thousand
/// bursts: measured 2026-11-23 as `UNIQUE constraint failed: workspaces.id` in
/// `the_handshake_confirmation_does_not_grow_with_the_catalogue`, whose 210
/// conversations are created in ~80 ms, and reproduced directly at 2 collisions
/// in 2 000 bursts of 210 ids. Those same ids are primary keys on every row kind
/// this crate and the Host store create (`ws`, `thread`, `event`, `artifact`,
/// `approval`, …), so the burst width is what a bulk import hits, not just a
/// test. Eight bytes of a v4 UUID (60 bits after its version nibble) put a
/// burst of n ids in one second at n²/2^61 of a repeat.
pub fn new_id(prefix: &str) -> String {
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let random = uuid::Uuid::new_v4();
    let suffix: String = random.as_bytes()[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{prefix}-{timestamp}-{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory database");
        ensure_table(&conn).expect("create the workspaces table");
        conn
    }

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "futureos-app-workspaces-{}-{}",
            std::process::id(),
            label
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create test directory");
        dir
    }

    /// A file-backed database two connections can open at once — the shape the
    /// desktop app and a CLI process have. An in-memory one cannot be shared, so
    /// the write races below need a real file.
    fn racing_conn(database: &Path) -> Connection {
        let conn = Connection::open(database).expect("open the shared database");
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000; PRAGMA foreign_keys = ON;",
        )
        .expect("pragmas");
        conn
    }

    /// `SELECT count(*) FROM workspaces WHERE path = ?` — what "one directory is
    /// one workspace" means on disk.
    fn rows_for_path(conn: &Connection, path: &Path) -> i64 {
        conn.query_row(
            "SELECT count(*) FROM workspaces WHERE kind = 'user' AND path = ?1 AND deleted_at IS NULL",
            params![normalize_path(path).display().to_string()],
            |row| row.get(0),
        )
        .expect("count rows")
    }

    /// Establish the write lock the racing write has to wait behind, with a row
    /// for `path` written but not yet committed: the state the winning creator
    /// is in between its own write and its commit.
    fn uncommitted_winner(conn: &Connection, path: &Path) {
        conn.execute_batch("BEGIN IMMEDIATE").expect("begin");
        conn.execute(
            "INSERT INTO workspaces (id, name, kind, path, cleanup_status, last_opened_at, created_at, updated_at)
             VALUES ('ws-winner', 'winner', 'user', ?1, 'active', 1, 1, 1)",
            params![normalize_path(path).display().to_string()],
        )
        .expect("insert the winning row");
    }

    /// Run `work` on another connection while this one holds an uncommitted row
    /// for the same directory, then commit — the interleaving that produced
    /// `ws-20261116-141317-e62d87` and `ws-20261116-141317-05aa4d` for one
    /// directory one millisecond apart. The racing write cannot finish first:
    /// its insert waits for this connection's lock, so committing afterwards
    /// lands exactly in its window, not somewhere near it.
    fn race_against_uncommitted_row<T: Send + 'static>(
        database: &Path,
        path: &Path,
        work: impl FnOnce(Connection) -> T + Send + 'static,
    ) -> T {
        let winner = racing_conn(database);
        uncommitted_winner(&winner, path);
        let racer_database = database.to_path_buf();
        let racer = std::thread::spawn(move || work(racing_conn(&racer_database)));
        // Give the racing write time to reach its insert and block there. It
        // cannot complete while the lock is held, so this is an ordering
        // guarantee, not a race: if it finished, it would be because the write
        // returned early, which no writer should do.
        std::thread::sleep(std::time::Duration::from_millis(150));
        assert!(
            !racer.is_finished(),
            "the racing write finished while the winning row was still uncommitted; \
             it did not wait for the write lock"
        );
        winner.execute_batch("COMMIT").expect("commit the winner");
        racer.join().expect("racing thread")
    }

    /// One directory, one workspace row — including when the row that answers
    /// the lookup is committed *after* the other writer looked. That is the
    /// interleaving that put two workspaces in the user's database at
    /// 2026-11-16 14:13:17 (…504 and …505), and the guard has to be the write
    /// itself: a lookup followed by an insert in a second statement leaves the
    /// window open no matter how the caller spells it.
    #[test]
    fn a_concurrent_creator_of_one_directory_leaves_one_row() {
        let base = temp_dir("race-create");
        let database = base.join("app.db");
        let dir = base.join("project");
        std::fs::create_dir_all(&dir).expect("create the directory");
        ensure_table(&racing_conn(&database)).expect("schema");

        let racing_dir = dir.clone();
        let created = race_against_uncommitted_row(&database, &dir, move |mut conn| {
            create(
                &mut conn,
                &racing_dir.display().to_string(),
                None,
                None,
                false,
                2,
            )
            .expect("create under the race")
        });

        let reader = racing_conn(&database);
        assert_eq!(
            rows_for_path(&reader, &dir),
            1,
            "one directory must stay one workspace row, got {:?}",
            list(&reader)
                .expect("list")
                .into_iter()
                .map(|workspace| workspace.id)
                .collect::<Vec<_>>()
        );
        // The loser reports the row that won instead of a row of its own.
        assert_eq!(created.id, "ws-winner");
        assert_eq!(created.path, normalize_path(&dir).display().to_string());

        let _ = std::fs::remove_dir_all(&base);
    }

    /// The same rule for the third writer: filing a conversation under a
    /// directory (`future session set --cwd`, and the desktop noticing a cwd
    /// change) creates the workspace when it is missing. It took its "no
    /// workspace yet" reading outside any transaction, so a creator committing
    /// in that window left the directory with two rows — one of them holding
    /// the conversation that caused it.
    #[test]
    fn a_cwd_filing_racing_a_creator_leaves_one_row() {
        let base = temp_dir("race-filing");
        let database = base.join("app.db");
        let dir = base.join("project");
        std::fs::create_dir_all(&dir).expect("create the directory");
        let seeded = racing_conn(&database);
        ensure_table(&seeded).expect("schema");
        seeded
            .execute_batch(
                "CREATE TABLE threads (
                     id TEXT PRIMARY KEY,
                     workspace_id TEXT NOT NULL DEFAULT '',
                     mode TEXT NOT NULL DEFAULT 'chat',
                     agent_session_id TEXT,
                     status TEXT NOT NULL DEFAULT 'active',
                     last_message_at INTEGER,
                     created_at INTEGER NOT NULL DEFAULT 0,
                     updated_at INTEGER NOT NULL DEFAULT 0
                 );
                 INSERT INTO threads (id, workspace_id, agent_session_id, status)
                      VALUES ('thread-1', '', 'sess-1', 'active');",
            )
            .expect("threads table");

        let racing_dir = dir.clone();
        let filing = race_against_uncommitted_row(&database, &dir, move |mut conn| {
            file_session_for_cwd(&mut conn, "sess-1", &racing_dir.display().to_string(), 2)
                .expect("file under the race")
        });

        let reader = racing_conn(&database);
        assert_eq!(
            rows_for_path(&reader, &dir),
            1,
            "the filing must join the existing workspace, not add a second one"
        );
        let filed_under: String = reader
            .query_row(
                "SELECT workspace_id FROM threads WHERE id = 'thread-1'",
                [],
                |row| row.get(0),
            )
            .expect("thread workspace");
        assert_eq!(filed_under, "ws-winner", "got filing {filing:?}");
        assert_eq!(
            get(&reader, "ws-winner")
                .expect("get")
                .expect("exists")
                .name,
            "winner",
            "the winner's row keeps its own name"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// The index, where the data allows it: a second row for one directory is
    /// refused by the schema itself, but a database that already holds such a
    /// pair still opens — the constraint is skipped rather than the rows being
    /// deleted, because which of the two to keep is the user's decision.
    #[test]
    fn the_unique_index_is_installed_but_never_costs_existing_duplicates() {
        let conn = test_conn();
        assert!(ensure_unique_user_path_index(&conn).expect("first attempt"));
        assert!(
            ensure_unique_user_path_index(&conn).expect("idempotent"),
            "a second attempt is a no-op, not a failure"
        );
        conn.execute(
            "INSERT INTO workspaces (id, name, kind, path, created_at, updated_at)
             VALUES ('ws-a', 'A', 'user', '/tmp/dup', 1, 1)",
            [],
        )
        .expect("seed");
        let refused = conn.execute(
            "INSERT INTO workspaces (id, name, kind, path, created_at, updated_at)
             VALUES ('ws-b', 'B', 'user', '/tmp/dup', 1, 1)",
            [],
        );
        assert!(refused.is_err(), "the index must refuse a second row");
        // A deleted row is history, not a live workspace: it may share a path.
        conn.execute(
            "INSERT INTO workspaces (id, name, kind, path, created_at, updated_at, deleted_at)
             VALUES ('ws-gone', 'Gone', 'user', '/tmp/dup', 1, 1, 1)",
            [],
        )
        .expect("a deleted row may reuse the path");
        // And a chat scratch workspace is a different kind of row.
        conn.execute(
            "INSERT INTO workspaces (id, name, kind, path, created_at, updated_at)
             VALUES ('ws-chat', 'Chat', 'temporary', '/tmp/dup', 1, 1)",
            [],
        )
        .expect("a temporary workspace may reuse the path");

        // The legacy layout (no index, two rows for one directory) still opens.
        let legacy = Connection::open_in_memory().expect("open");
        legacy
            .execute_batch(
                "CREATE TABLE workspaces (
                     id TEXT PRIMARY KEY,
                     name TEXT NOT NULL,
                     kind TEXT NOT NULL CHECK (kind IN ('user', 'temporary')),
                     path TEXT NOT NULL,
                     description TEXT,
                     pinned INTEGER NOT NULL DEFAULT 0,
                     cleanup_status TEXT NOT NULL DEFAULT 'active',
                     cleanup_requested_at INTEGER,
                     cleaned_at INTEGER,
                     last_opened_at INTEGER,
                     created_at INTEGER NOT NULL,
                     updated_at INTEGER NOT NULL,
                     archived_at INTEGER,
                     deleted_at INTEGER
                 );
                 INSERT INTO workspaces (id, name, kind, path, created_at, updated_at)
                      VALUES ('ws-1', 'W', 'user', '/tmp/dup', 1, 1),
                             ('ws-2', 'W', 'user', '/tmp/dup', 2, 2);",
            )
            .expect("legacy database");
        assert!(
            !ensure_unique_user_path_index(&legacy).expect("tolerated"),
            "an existing duplicate leaves the index out instead of failing the open"
        );
        assert_eq!(
            legacy
                .query_row("SELECT count(*) FROM workspaces", [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("count"),
            2,
            "the duplicate rows are the user's to merge, so they stay"
        );
        // ensure_table refuses to fail on it too.
        ensure_table(&legacy).expect("ensure_table tolerates the duplicate");
    }

    /// Two creators that both read "no workspace yet" outside a transaction is
    /// the shape this crate must not break on: whichever write lands second
    /// adopts the row the first one made.
    #[test]
    fn a_losing_create_adopts_the_row_that_won() {
        let conn = test_conn();
        let dir = temp_dir("adopt");
        let mut first = find_or_create_user(&conn, "Mine".into(), &dir, None, 1).expect("first");
        assert!(first.id.starts_with("ws-"));
        // The second call looks at a database that already has the row, and the
        // guarded insert reports it inserted nothing.
        let second = find_or_create_user(&conn, "Theirs".into(), &dir, None, 2).expect("second");
        assert_eq!(second.id, first.id);
        assert_eq!(second.name, "Mine", "the stored name wins");
        assert_eq!(rows_for_path(&conn, &dir), 1);
        // A wake on the adopted row still clears an archived marker.
        conn.execute(
            "UPDATE workspaces SET archived_at = 5 WHERE id = ?1",
            params![first.id],
        )
        .expect("archive");
        first = find_or_create_user(&conn, "Mine".into(), &dir, None, 3).expect("third");
        assert_eq!(first.archived_at, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A workspace this crate creates has no archived marker, and archiving
    /// hides it without touching its conversations' rows.
    #[test]
    fn archive_and_restore_round_trip() {
        let conn = test_conn();
        conn.execute_batch(
            "INSERT INTO workspaces (id, name, kind, path, created_at, updated_at)
                 VALUES ('ws', 'W', 'user', '/tmp/ws', 1, 1),
                        ('chat', 'Chat', 'temporary', '/tmp/chat', 1, 1);",
        )
        .expect("seed");

        let archived = archive(&conn, "ws", 5).expect("archive");
        assert_eq!(archived.archived_at, Some(5));

        let refused = archive(&conn, "chat", 6).expect_err("a chat workspace is not archivable");
        assert!(refused.to_string().contains("not available"), "{refused}");

        let restored = restore(&conn, "ws", 7).expect("restore");
        assert_eq!(restored.archived_at, None);
        assert!(restore(&conn, "ghost", 8).is_err());
    }

    /// The table this crate creates is the table the column list describes, so
    /// a CLI-created database and the desktop's migrations cannot disagree on
    /// the row shape.
    #[test]
    fn ensure_table_matches_the_column_list() {
        let conn = test_conn();
        let mut statement = conn
            .prepare("PRAGMA table_info(workspaces)")
            .expect("read table info");
        let declared: Vec<String> = statement
            .query_map([], |row| row.get::<_, String>(1))
            .expect("query table info")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("collect column names");
        let expected: Vec<String> = COLUMNS
            .split(',')
            .map(|column| column.trim().to_string())
            .collect();
        assert_eq!(declared, expected);
        // Creating it twice is a no-op, so a second CLI run cannot fail here.
        ensure_table(&conn).expect("idempotent");
    }

    /// A directory becomes one workspace however it is spelled, and creating it
    /// again reopens the row rather than duplicating the group.
    #[test]
    fn a_directory_is_one_workspace_and_reopens_by_path() {
        let mut conn = test_conn();
        let dir = temp_dir("one");
        let canonical = strip_verbatim_prefix(dir.canonicalize().expect("canonicalize"));

        let created = create(
            &mut conn,
            &dir.display().to_string(),
            None,
            Some("d".to_string()),
            false,
            0,
        )
        .expect("create");
        assert_eq!(created.kind, "user");
        assert_eq!(
            created.name,
            canonical.file_name().unwrap().to_string_lossy()
        );
        assert_eq!(created.path, canonical.display().to_string());
        assert_eq!(created.description.as_deref(), Some("d"));
        assert_eq!(created.cleanup_status, "active");
        assert!(!created.pinned);
        assert!(created.id.starts_with("ws-"), "got {}", created.id);

        // The same directory through a trailing separator: the stored spelling
        // is matched as given, and the canonical comparison resolves the rest.
        let alias = format!("{}{}", canonical.display(), std::path::MAIN_SEPARATOR);
        let again = create(
            &mut conn,
            &alias,
            Some("Ignored".to_string()),
            None,
            false,
            0,
        )
        .expect("reopen");
        assert_eq!(again.id, created.id);
        assert_eq!(
            again.name, created.name,
            "reopening keeps the stored name, not the one passed in"
        );
        assert_eq!(list(&conn).expect("list").len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// One directory must resolve to one workspace even when it is reached
    /// through a symlink (the macOS `/tmp` → `/private/tmp` shape).
    #[cfg(unix)]
    #[test]
    fn a_symlinked_spelling_resolves_to_the_stored_workspace() {
        let mut conn = test_conn();
        let base = temp_dir("symlink");
        let real = base.join("real");
        let alias = base.join("alias");
        std::fs::create_dir_all(&real).expect("create real directory");
        std::os::unix::fs::symlink(&real, &alias).expect("symlink");

        let created = create(&mut conn, &real.display().to_string(), None, None, false, 0)
            .expect("create through the real path");
        let aliased = create(
            &mut conn,
            &alias.display().to_string(),
            None,
            None,
            false,
            0,
        )
        .expect("create through the alias");
        assert_eq!(aliased.id, created.id);

        // A legacy row kept under the aliasing spelling is still found: the
        // lookup compares canonical paths, not the text stored.
        conn.execute(
            "UPDATE workspaces SET path = ?1 WHERE id = ?2",
            params![alias.display().to_string(), created.id],
        )
        .expect("re-spell the stored path");
        assert_eq!(
            find_user_by_path(&conn, &real)
                .expect("find")
                .map(|workspace| workspace.id),
            Some(created.id)
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// A missing directory is refused before any row is written, and
    /// `create_directory` is the one way to accept one.
    #[test]
    fn a_missing_directory_is_refused_unless_it_may_be_created() {
        let mut conn = test_conn();
        let path = temp_dir("missing").join("nested/proj");

        let refused = create(&mut conn, &path.display().to_string(), None, None, false, 0);
        let message = refused
            .expect_err("a missing directory is refused")
            .to_string();
        assert!(message.contains("does not exist"), "got: {message}");
        assert!(list(&conn).expect("list").is_empty());

        let created = create(&mut conn, &path.display().to_string(), None, None, true, 0)
            .expect("create the directory and the workspace");
        assert!(path.is_dir());
        assert_eq!(created.name, "proj");

        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn tilde_expands_against_home_and_other_paths_pass_through() {
        let home = home_dir().expect("this host has a home directory");
        assert_eq!(expand_tilde("~").expect("bare tilde"), home);
        assert_eq!(
            expand_tilde("~/docs").expect("tilde with suffix"),
            home.join("docs")
        );
        assert_eq!(
            expand_tilde("/abs/path").expect("absolute path"),
            PathBuf::from("/abs/path")
        );
        assert_eq!(
            expand_tilde("relative/path").expect("relative path"),
            PathBuf::from("relative/path")
        );
    }

    /// The chat scratch directories share the table, so a caller that wants the
    /// user's own workspaces must filter; deleting a row hides it from `list`
    /// while leaving it readable by id.
    #[test]
    fn list_hides_deleted_rows_and_orders_by_recency() {
        let conn = test_conn();
        conn.execute_batch(
            "INSERT INTO workspaces (id, name, kind, path, last_opened_at, created_at, updated_at)
                 VALUES ('ws_old', 'Old', 'user', '/tmp/old', 100, 1, 1),
                        ('ws_new', 'New', 'user', '/tmp/new', 200, 1, 1),
                        ('ws_chat', 'Chat', 'temporary', '/tmp/chat', NULL, 1, 250),
                        ('ws_gone', 'Gone', 'user', '/tmp/gone', 300, 1, 1);
             UPDATE workspaces SET deleted_at = 1 WHERE id = 'ws_gone';",
        )
        .expect("seed");

        let listed = list(&conn).expect("list");
        let ids: Vec<&str> = listed
            .iter()
            .map(|workspace| workspace.id.as_str())
            .collect();
        assert_eq!(ids, vec!["ws_chat", "ws_new", "ws_old"]);
        assert!(get(&conn, "ws_gone").expect("get").is_some());
        assert!(get(&conn, "ws_ghost").expect("get").is_none());
    }

    /// A workspace is ranked by its most recent *activity*, and a conversation
    /// filed under it is that activity: the picker's question is which one was
    /// used last, and a workspace the user has been working in all day holds
    /// conversations far newer than the row's own `last_opened_at` (which only
    /// moves when the workspace itself is opened, or when it is created).
    #[test]
    fn list_ranks_a_workspace_by_its_newest_conversation() {
        let conn = test_conn();
        add_threads_table(&conn);
        conn.execute_batch(
            "INSERT INTO workspaces (id, name, kind, path, last_opened_at, created_at, updated_at)
                 VALUES ('ws_worked_in', 'Worked in', 'user', '/tmp/worked', 100, 1, 100),
                        ('ws_just_created', 'Just created', 'user', '/tmp/created', 200, 1, 200),
                        ('ws_untouched', 'Untouched', 'user', '/tmp/untouched', 150, 1, 150);
             INSERT INTO threads (id, workspace_id, status, last_message_at, created_at, updated_at)
                 VALUES ('t_live', 'ws_worked_in', 'active', 300, 1, 100),
                        ('t_gone', 'ws_untouched', 'deleted', 900, 1, 900);",
        )
        .expect("seed");

        let ids: Vec<String> = list(&conn)
            .expect("list")
            .into_iter()
            .map(|workspace| workspace.id)
            .collect();
        // The conversation outranks the newer row; a deleted one counts for
        // nothing, so its workspace keeps the rank its own timestamps give it.
        assert_eq!(ids, vec!["ws_worked_in", "ws_just_created", "ws_untouched"]);
    }

    /// A workspace path is stored canonicalized, and a `~` path is expanded
    /// with it, so every client's spelling maps to the one row.
    #[test]
    fn created_paths_are_stored_canonically() {
        let mut conn = test_conn();
        let dir = temp_dir("canonical");
        let created =
            create(&mut conn, &dir.display().to_string(), None, None, false, 0).expect("create");
        assert_eq!(
            created.path,
            strip_verbatim_prefix(dir.canonicalize().expect("canonicalize"))
                .display()
                .to_string()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A threads table shaped like the desktop's, for the filing tests. Only
    /// the columns the filing and the activity ordering read are declared.
    fn add_threads_table(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE threads (
                 id TEXT PRIMARY KEY,
                 workspace_id TEXT NOT NULL DEFAULT '',
                 mode TEXT NOT NULL DEFAULT 'chat',
                 agent_session_id TEXT,
                 status TEXT NOT NULL DEFAULT 'active',
                 last_message_at INTEGER,
                 created_at INTEGER NOT NULL DEFAULT 0,
                 updated_at INTEGER NOT NULL DEFAULT 0
             );",
        )
        .expect("create threads");
    }

    fn bind_thread(conn: &Connection, thread: &str, session: &str) {
        conn.execute(
            "INSERT INTO threads (id, workspace_id, agent_session_id, status, updated_at)
             VALUES (?1, '', ?2, 'active', 0)",
            params![thread, session],
        )
        .expect("bind thread");
    }

    fn thread_workspace(conn: &Connection, thread: &str) -> String {
        conn.query_row(
            "SELECT workspace_id FROM threads WHERE id = ?1",
            params![thread],
            |row| row.get(0),
        )
        .expect("thread workspace")
    }

    fn thread_mode(conn: &Connection, thread: &str) -> String {
        conn.query_row(
            "SELECT mode FROM threads WHERE id = ?1",
            params![thread],
            |row| row.get(0),
        )
        .expect("thread mode")
    }

    /// A cwd change files the conversation under the workspace for the
    /// directory: created the first time, reused afterwards — the same rule the
    /// desktop app applies when it observes the change itself, so a cwd changed
    /// while the app was closed lands identically.
    #[test]
    fn a_cwd_change_files_the_conversation_under_one_workspace() {
        let mut conn = test_conn();
        add_threads_table(&conn);
        bind_thread(&conn, "thread-1", "sess-1");
        bind_thread(&conn, "thread-2", "sess-2");
        let dir = temp_dir("filing");
        let canonical = strip_verbatim_prefix(dir.canonicalize().expect("canonicalize"));

        let filing =
            file_session_for_cwd(&mut conn, "sess-1", &dir.display().to_string(), 5).expect("file");
        let Some(CwdFiling::WorkspaceCreated(id)) = filing else {
            panic!("expected a created workspace, got {filing:?}");
        };
        assert_eq!(thread_workspace(&conn, "thread-1"), id);
        let workspace = get(&conn, &id).expect("get").expect("exists");
        assert_eq!(workspace.path, canonical.display().to_string());
        assert_eq!(workspace.name, name_from_path(&canonical));
        assert_eq!(workspace.kind, "user");

        // A trailing separator is the same directory: reused, not a second row.
        let alias = format!("{}{}", canonical.display(), std::path::MAIN_SEPARATOR);
        assert_eq!(
            file_session_for_cwd(&mut conn, "sess-2", &alias, 6).expect("file"),
            Some(CwdFiling::WorkspaceReused(id.clone()))
        );
        assert_eq!(thread_workspace(&conn, "thread-2"), id);
        assert_eq!(list(&conn).expect("list").len(), 1);

        // Empty cwd and unimported sessions are answered, not failed: there is
        // nothing to file, and the app is not left thinking it wrote something.
        assert_eq!(
            file_session_for_cwd(&mut conn, "sess-1", "   ", 7).expect("empty"),
            None
        );
        assert_eq!(
            file_session_for_cwd(&mut conn, "sess-ghost", "/tmp", 8).expect("unimported"),
            Some(CwdFiling::NotImported)
        );
        assert_eq!(thread_workspace(&conn, "thread-1"), id);

        // A database the CLI created before the app ever ran has no `threads`
        // table at all. That is "no conversations yet", not a broken store: the
        // filing must answer, not fail — otherwise a cwd change on a machine
        // where the app never imported anything reports a confusing error.
        let mut cli_only = test_conn();
        assert!(!threads_table_exists(&cli_only).expect("probe"));
        assert_eq!(
            file_session_for_cwd(&mut cli_only, "sess-1", "/tmp", 10).expect("no app schema"),
            Some(CwdFiling::NotImported)
        );

        // A directory that is not there cannot be filed under.
        let missing = temp_dir("filing-missing").join("not-there");
        let error = file_session_for_cwd(&mut conn, "sess-1", &missing.display().to_string(), 9)
            .expect_err("missing directory");
        assert!(error.to_string().contains("does not exist"), "{error}");
        assert_eq!(thread_workspace(&conn, "thread-1"), id, "unchanged");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A chat conversation whose cwd moves into a real directory stops being a
    /// chat: both clients group by `mode`, so filing the workspace alone used to
    /// leave it under "Chat" forever — the row said `chat` while its workspace
    /// said `future-os`, and nothing rewrites `mode` after creation.
    #[test]
    fn a_chat_conversation_filed_under_a_project_becomes_a_workspace_conversation() {
        let mut conn = test_conn();
        add_threads_table(&conn);
        bind_thread(&conn, "thread-chat", "sess-chat");
        assert_eq!(thread_mode(&conn, "thread-chat"), "chat");
        let dir = temp_dir("promote");

        let filing = file_session_for_cwd(&mut conn, "sess-chat", &dir.display().to_string(), 3)
            .expect("file");
        let Some(CwdFiling::WorkspaceCreated(id)) = filing else {
            panic!("expected a created workspace, got {filing:?}");
        };
        assert_eq!(thread_workspace(&conn, "thread-chat"), id);
        assert_eq!(
            thread_mode(&conn, "thread-chat"),
            "workspace",
            "a conversation under a user workspace is a workspace conversation"
        );

        // Filing again (the app noticing a cwd it already filed, or the CLI and
        // the app both handling one change) keeps it a workspace conversation.
        assert_eq!(
            file_session_for_cwd(&mut conn, "sess-chat", &dir.display().to_string(), 4)
                .expect("refile"),
            Some(CwdFiling::WorkspaceReused(id.clone()))
        );
        assert_eq!(thread_mode(&conn, "thread-chat"), "workspace");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A `threads` table that predates `mode` still gets its workspace filed:
    /// the column cannot be set, and a cwd change must not fail because of it.
    /// (Migrations refuse such a layout outright, so no running app meets it.)
    #[test]
    fn a_thread_table_without_mode_is_still_filed() {
        let mut conn = test_conn();
        conn.execute_batch(
            "CREATE TABLE threads (
                 id TEXT PRIMARY KEY,
                 workspace_id TEXT NOT NULL DEFAULT '',
                 agent_session_id TEXT,
                 status TEXT NOT NULL DEFAULT 'active',
                 updated_at INTEGER NOT NULL DEFAULT 0
             );",
        )
        .expect("create old threads");
        conn.execute(
            "INSERT INTO threads (id, workspace_id, agent_session_id, status)
             VALUES ('thread-old', '', 'sess-old', 'active')",
            [],
        )
        .expect("seed old thread");
        let dir = temp_dir("no-mode");

        let filing = file_session_for_cwd(&mut conn, "sess-old", &dir.display().to_string(), 2)
            .expect("filed");
        let Some(CwdFiling::WorkspaceCreated(id)) = filing else {
            panic!("expected a created workspace, got {filing:?}");
        };
        assert_eq!(thread_workspace(&conn, "thread-old"), id);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A chat scratch cwd is not a project: the conversation's own temporary
    /// workspace follows the path (that is what the app does for its chats)
    /// rather than a new user workspace appearing for the scratch directory.
    #[test]
    fn a_chat_directory_moves_the_temporary_workspace_instead() {
        let mut conn = test_conn();
        add_threads_table(&conn);
        let root = chat_workspaces_root().expect("home");
        conn.execute(
            "INSERT INTO workspaces (id, name, kind, path, cleanup_status, created_at, updated_at)
             VALUES ('ws_chat', 'Chat Workspace', 'temporary', ?1, 'active', 1, 1)",
            params![root.join("thread-chat").display().to_string()],
        )
        .expect("seed chat workspace");
        conn.execute(
            "INSERT INTO threads (id, workspace_id, agent_session_id, status, updated_at)
             VALUES ('thread-chat', 'ws_chat', 'sess-chat', 'active', 1)",
            [],
        )
        .expect("seed chat thread");

        let moved = root.join("sess-chat");
        assert_eq!(
            file_session_for_cwd(&mut conn, "sess-chat", &moved.display().to_string(), 4)
                .expect("file"),
            Some(CwdFiling::ChatWorkspaceUpdated)
        );
        let workspace = get(&conn, "ws_chat").expect("get").expect("exists");
        assert_eq!(workspace.path, moved.display().to_string());
        assert_eq!(workspace.kind, "temporary");
        assert_eq!(
            list(&conn).expect("list").len(),
            1,
            "no user workspace for a scratch directory"
        );
        // A scratch cwd is a chat's own, so it never promotes; and a workspace
        // conversation sent to one is not demoted either (demoting would need a
        // temporary workspace it never had). Both keep the mode they had.
        assert_eq!(thread_mode(&conn, "thread-chat"), "chat");
        conn.execute(
            "INSERT INTO threads (id, workspace_id, mode, agent_session_id, status, updated_at)
             VALUES ('thread-proj', 'ws_chat', 'workspace', 'sess-proj', 'active', 1)",
            [],
        )
        .expect("seed workspace thread");
        assert_eq!(
            file_session_for_cwd(
                &mut conn,
                "sess-proj",
                &root.join("sess-proj").display().to_string(),
                5
            )
            .expect("file"),
            Some(CwdFiling::ChatWorkspaceUpdated)
        );
        assert_eq!(thread_mode(&conn, "thread-proj"), "workspace");

        // A tilde spelling of the same sentinel is recognised too.
        assert!(is_chat_directory("~/.future/workspaces/chat/sess-chat"));
        assert!(!is_chat_directory("/tmp/.future/workspaces/chat/sess-chat"));
        assert!(!is_chat_directory(""));
    }

    /// A cwd that cannot be classified against a root must never be classified
    /// as a scratch directory: "the home is unknown" is not "it is a chat".
    #[test]
    fn a_cwd_is_not_a_chat_directory_when_the_home_cannot_be_resolved() {
        assert!(!is_chat_directory_at(
            "/home/user/.future/workspaces/chat/sess-1",
            Err(Error::Invalid(
                "HOME/USERPROFILE environment variable is not set.".into()
            ))
        ));
        // The empty cwd is refused before the root is even considered.
        assert!(!is_chat_directory_at(
            "",
            Ok(chat_workspaces_root().expect("the test host has a home"))
        ));
        // And a real root still classifies its own children.
        let root = chat_workspaces_root().expect("the test host has a home");
        assert!(is_chat_directory_at(
            &root.join("sess-1").display().to_string(),
            Ok(root.clone())
        ));
        assert!(!is_chat_directory_at(
            &std::env::temp_dir().join("project").display().to_string(),
            Ok(root)
        ));
    }

    /// Workspace identity is the canonical directory when it exists, and the
    /// given spelling when it does not — a workspace may be stored before its
    /// directory is created, and it must still be one row.
    #[test]
    fn identity_falls_back_to_the_given_spelling_for_a_missing_directory() {
        let dir = temp_dir("normalize");
        let canonical = strip_verbatim_prefix(dir.canonicalize().expect("canonicalize"));
        assert_eq!(normalize_path(&dir), canonical);
        // The same directory, spelled one level deeper and through `.`/`..`.
        let spelled = dir.join(".").join("..").join(dir.file_name().unwrap());
        assert_eq!(
            normalize_path(&spelled),
            canonical,
            "an existing directory resolves to one identity however it is spelled"
        );

        let missing = dir.join("not-there");
        assert_eq!(
            normalize_path(&missing),
            missing,
            "a directory that does not exist keeps the spelling it was given"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Windows canonicalization returns the extended-length spelling; the crate
    /// stores the ordinary one. The rule is purely textual, so it is the same
    /// rule on every platform.
    #[test]
    fn the_verbatim_prefix_is_stripped_from_both_drive_and_unc_spellings() {
        assert_eq!(
            strip_verbatim_prefix(PathBuf::from(r"\\?\UNC\server\share\dir")),
            PathBuf::from(r"\\server\share\dir")
        );
        assert_eq!(
            strip_verbatim_prefix(PathBuf::from(r"\\?\D:\projects\app")),
            PathBuf::from(r"D:\projects\app")
        );
        assert_eq!(
            strip_verbatim_prefix(PathBuf::from("/tmp/project")),
            PathBuf::from("/tmp/project"),
            "a POSIX path is a no-op"
        );
    }

    /// A store that cannot be read must say why and keep the cause reachable:
    /// the CLI prints these messages and the desktop renders them.
    #[test]
    fn every_error_variant_renders_its_cause_and_keeps_it_as_its_source() {
        let sqlite_message = Error::Database(rusqlite::Error::InvalidQuery).to_string();
        let sqlite = Error::from(rusqlite::Error::InvalidQuery);
        assert_eq!(sqlite.to_string(), sqlite_message);
        assert!(std::error::Error::source(&sqlite).is_some());

        let io = Error::from(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "denied",
        ));
        assert!(io.to_string().contains("denied"), "{io}");
        assert!(std::error::Error::source(&io).is_some());

        let invalid = Error::Invalid("no home".into());
        assert_eq!(invalid.to_string(), "no home");
        assert!(std::error::Error::source(&invalid).is_none());
    }

    /// Ids are opaque, but they keep the desktop's shape: prefix, local time,
    /// and a random suffix.
    #[test]
    fn new_id_keeps_the_shared_shape_and_does_not_repeat() {
        let id = new_id("ws");
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts.len(), 4, "got {id}");
        assert_eq!(parts[0], "ws");
        assert_eq!(parts[1].len(), 8, "date: {id}");
        assert_eq!(parts[2].len(), 6, "time: {id}");
        assert_eq!(parts[3].len(), 16, "random suffix: {id}");
        assert!(parts[3].chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(new_id("ws"), new_id("ws"));
    }

    /// A burst of ids minted inside one second never repeats.
    ///
    /// Every id in such a burst carries the same timestamp, so the suffix alone
    /// separates them — the failure this test exists for was
    /// `UNIQUE constraint failed: workspaces.id` from 210 conversations created
    /// in 80 ms, i.e. a burst whose ids shared one second. Twenty thousand ids
    /// is far past any burst a caller mints in a second and still cheap; at the
    /// old three-byte suffix it repeats about twelve times per run, and at the
    /// current eight it needs roughly 2^61 draws to repeat at all.
    #[test]
    fn a_burst_minted_in_one_second_never_repeats() {
        const BURST: usize = 20_000;
        let ids: std::collections::HashSet<String> = (0..BURST).map(|_| new_id("ws")).collect();
        assert_eq!(
            ids.len(),
            BURST,
            "a one-second burst of {BURST} ids must not repeat an id"
        );
    }
}
