//! A throwaway home holding the desktop app's real schema, for the command
//! tests that read or write `~/.future/app/app.db`.
//!
//! Shared by [`super::archive`] and [`super::pin`]: both drive the same store
//! through the same shapes, and two copies of the fixture would drift into two
//! different notions of "a workspace with a conversation in it".

use crate::output::Output;
use crate::test_env::EnvGuard;
use future_app_workspaces as workspaces;
use future_host::store;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// A throwaway home with the desktop app's real schema in it (`HOME`, not
/// `FUTURE_HOME`: the app's database lives under the real home).
pub(super) struct Store {
    dir: tempfile::TempDir,
    _env: EnvGuard,
}

impl Store {
    pub(super) fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let env = EnvGuard::set(&[("HOME", dir.path().as_os_str().to_owned())]);
        store::initialize_app_store().expect("the desktop app's schema");
        Store { dir, _env: env }
    }

    pub(super) fn database(&self) -> PathBuf {
        self.dir.path().join(".future").join("app").join("app.db")
    }

    pub(super) fn conn(&self) -> Connection {
        Connection::open(self.database()).expect("open store")
    }

    pub(super) fn directory(&self, name: &str) -> PathBuf {
        let path = self.dir.path().join(name);
        std::fs::create_dir_all(&path).expect("create directory");
        path
    }
}

pub(super) fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

pub(super) fn text(captured: Arc<Mutex<Vec<u8>>>) -> String {
    String::from_utf8(captured.lock().unwrap().clone()).expect("utf8")
}

/// A workspace row created the way `future workspace add` creates one.
pub(super) fn add_workspace(store: &Store, path: &Path) -> workspaces::Workspace {
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
pub(super) fn add_thread(
    store: &Store,
    workspace_id: &str,
    session_id: &str,
    title: &str,
) -> String {
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

/// The captured stdout of a command that ran into an in-memory [`Output`].
#[allow(dead_code)]
pub(super) fn memory() -> (Output, crate::output::Captured) {
    Output::memory()
}

pub(super) fn archived_at(store: &Store, workspace_id: &str) -> Option<i64> {
    let conn = store.conn();
    conn.query_row(
        "SELECT archived_at FROM workspaces WHERE id = ?1",
        [workspace_id],
        |row| row.get(0),
    )
    .expect("read archived_at")
}

pub(super) fn thread_status(store: &Store, thread_id: &str) -> String {
    let conn = store.conn();
    conn.query_row(
        "SELECT status FROM threads WHERE id = ?1",
        [thread_id],
        |row| row.get(0),
    )
    .expect("read status")
}

/// The row's `pinned` flag, at either scope — the one fact a pin command must
/// change and an archive command must clear.
pub(super) fn pinned(store: &Store, table: &str, id: &str) -> bool {
    let conn = store.conn();
    conn.query_row(
        &format!("SELECT pinned FROM {table} WHERE id = ?1"),
        [id],
        |row| row.get(0),
    )
    .expect("read pinned")
}

/// The stamps pinning must **not** move: a pin is an ordering flag, not an
/// activity event, and a moved `updated_at` would reorder the very list the pin
/// is supposed to lead.
pub(super) fn threads_stamps(store: &Store, thread_id: &str) -> (i64, Option<i64>, Option<i64>) {
    let conn = store.conn();
    conn.query_row(
        "SELECT updated_at, last_message_at, last_opened_at FROM threads WHERE id = ?1",
        [thread_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .expect("read thread stamps")
}

pub(super) fn workspace_stamps(store: &Store, workspace_id: &str) -> (i64, Option<i64>) {
    let conn = store.conn();
    conn.query_row(
        "SELECT updated_at, last_opened_at FROM workspaces WHERE id = ?1",
        [workspace_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .expect("read workspace stamps")
}

/// Give a conversation recent activity, so ordering by it is observable.
pub(super) fn set_activity(store: &Store, thread_id: &str, last_message_at: i64) {
    let conn = store.conn();
    conn.execute(
        "UPDATE threads SET last_message_at = ?1 WHERE id = ?2",
        rusqlite::params![last_message_at, thread_id],
    )
    .expect("set activity");
}
