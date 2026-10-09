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
    pub deleted_at: Option<i64>,
}

/// Comma-separated `SELECT` column list, in [`from_row`]'s order.
pub const COLUMNS: &str = "id, name, kind, path, description, pinned, cleanup_status, \
                            cleanup_requested_at, cleaned_at, last_opened_at, created_at, \
                            updated_at, deleted_at";

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
        deleted_at: row.get(12)?,
    })
}

/// Create the `workspaces` table when it does not exist, so a first write from
/// the CLI does not require the desktop app to have run. Idempotent: the
/// desktop's own schema is created first whenever the app has started, and its
/// migration state is left untouched by this.
pub fn ensure_table(conn: &Connection) -> Result<(), Error> {
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
             deleted_at INTEGER
         )",
    )?;
    Ok(())
}

/// Every live workspace row, most recently opened first. Both kinds are
/// returned; a caller that wants only the user's own directories filters on
/// [`Workspace::kind`].
pub fn list(conn: &Connection) -> Result<Vec<Workspace>, Error> {
    let mut statement = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM workspaces WHERE deleted_at IS NULL
         ORDER BY COALESCE(last_opened_at, updated_at) DESC"
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
/// none. The caller owns the transaction: a composite write (the desktop's
/// `create_thread`) runs this inside its own.
pub fn find_or_create_user(
    conn: &Connection,
    name: String,
    path: &Path,
    description: Option<String>,
    now: i64,
) -> Result<Workspace, Error> {
    if let Some(workspace) = find_user_by_path(conn, path)? {
        return Ok(workspace);
    }

    let workspace_id = new_id("ws");
    conn.execute(
        "INSERT INTO workspaces (
             id, name, kind, path, description, cleanup_status, last_opened_at,
             created_at, updated_at
         ) VALUES (?1, ?2, 'user', ?3, ?4, 'active', ?5, ?5, ?5)",
        params![
            workspace_id,
            name,
            normalize_path(path).display().to_string(),
            description,
            now
        ],
    )?;

    get(conn, &workspace_id)?
        .ok_or_else(|| Error::Invalid("Created workspace could not be loaded.".to_string()))
}

/// The full create-workspace rule both writers share: `~` is expanded, the
/// directory must already exist unless `create_directory` says to make it, the
/// name defaults to the directory's own name, and a directory that already has
/// a workspace reopens that row instead of gaining a second one.
///
/// The `SELECT`-then-`INSERT` runs in one `BEGIN IMMEDIATE` transaction, so the
/// CLI and the desktop app creating the same directory at the same moment
/// cannot produce two workspaces for it.
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

/// A fresh row id (`ws-20260924-141714-3fa9c1`). The shape matches the
/// desktop's other ids: a prefix, local time, and three random bytes.
pub fn new_id(prefix: &str) -> String {
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let random = uuid::Uuid::new_v4();
    let suffix: String = random.as_bytes()[..3]
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
        let canonical = dir.canonicalize().expect("canonicalize");

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
            dir.canonicalize()
                .expect("canonicalize")
                .display()
                .to_string()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Ids are opaque, but they keep the desktop's shape: prefix, local time,
    /// three random bytes — and two ids never collide.
    #[test]
    fn new_id_keeps_the_shared_shape_and_does_not_repeat() {
        let id = new_id("ws");
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts.len(), 4, "got {id}");
        assert_eq!(parts[0], "ws");
        assert_eq!(parts[1].len(), 8, "date: {id}");
        assert_eq!(parts[2].len(), 6, "time: {id}");
        assert_eq!(parts[3].len(), 6, "random suffix: {id}");
        assert!(parts[3].chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(new_id("ws"), new_id("ws"));
    }
}
