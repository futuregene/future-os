//! Session registry: create, list, update, attach, close.
//!
//! The manager owns the only strong references to live sessions and keeps the
//! exited ones around for a bounded while so a reloaded view can still read the
//! final screen and the exit code.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::cwd::{self, CwdPolicy};
use super::session::{Info, Session};
use super::shell;

/// Total live+retained sessions. A desktop app never legitimately needs more;
/// the cap turns a runaway client into a clear error instead of a fork bomb.
pub const MAX_SESSIONS: usize = 32;
/// Tabs per conversation.
pub const MAX_SESSIONS_PER_THREAD: usize = 12;
/// Exited sessions stay observable (final screen, exit code) until this many
/// later sessions have exited, or until the client removes them.
pub const EXITED_LIMIT: usize = 25;

/// Grace period for a session being closed on purpose (user closed the tab,
/// conversation deleted): short, because the user is waiting.
pub const CLOSE_GRACE: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagerError {
    NotFound(String),
    /// The client asked to write to a conversation that no longer accepts tabs.
    ThreadNotWritable(String),
    /// Working-directory resolution failed (includes the configured path).
    Cwd(cwd::CwdError),
    /// No usable shell on this machine.
    ShellUnavailable(String),
    /// The PTY refused to start.
    Spawn(String),
    /// Capacity limit reached; retry after closing a tab.
    Capacity(String),
}

impl ManagerError {
    /// Stable code for the client. The set is closed; new variants need a
    /// deliberate protocol change.
    pub fn code(&self) -> &'static str {
        match self {
            ManagerError::NotFound(_) => "TERMINAL_NOT_FOUND",
            ManagerError::ThreadNotWritable(_) => "THREAD_READONLY",
            ManagerError::Cwd(error) => error.code(),
            ManagerError::ShellUnavailable(_) => "SHELL_UNAVAILABLE",
            ManagerError::Spawn(_) => "SPAWN_FAILED",
            ManagerError::Capacity(_) => "CAPACITY_EXCEEDED",
        }
    }

    /// HTTP status for the control route that produced it.
    pub fn status(&self) -> u16 {
        match self {
            ManagerError::NotFound(_) => 404,
            ManagerError::Capacity(_) => 429,
            _ => 400,
        }
    }

    /// Only a bad working directory can be retried with `homeConfirmed`.
    pub fn allows_home_fallback(&self) -> bool {
        matches!(self, ManagerError::Cwd(error) if error.allows_home_fallback())
    }

    pub fn message(&self) -> String {
        match self {
            ManagerError::NotFound(id) => format!("TERMINAL_NOT_FOUND: no terminal {id}"),
            ManagerError::ThreadNotWritable(id) => {
                format!("THREAD_READONLY: conversation {id} does not accept terminal tabs")
            }
            ManagerError::Cwd(error) => error.to_string(),
            ManagerError::ShellUnavailable(detail) => detail.clone(),
            ManagerError::Spawn(detail) => detail.clone(),
            ManagerError::Capacity(detail) => detail.clone(),
        }
    }
}

impl From<cwd::CwdError> for ManagerError {
    fn from(error: cwd::CwdError) -> Self {
        match error {
            cwd::CwdError::ThreadNotWritable(id) => ManagerError::ThreadNotWritable(id),
            other => ManagerError::Cwd(other),
        }
    }
}

#[derive(Default)]
struct Registry {
    sessions: HashMap<String, Arc<Session>>,
    /// Ids of exited sessions, oldest first — the eviction order.
    exit_order: VecDeque<String>,
}

#[derive(Debug, Clone)]
pub struct CreateRequest {
    pub thread_id: String,
    pub title: Option<String>,
    pub cols: u16,
    pub rows: u16,
    pub policy: CwdPolicy,
}

pub struct Manager {
    registry: Mutex<Registry>,
}

impl Default for Manager {
    fn default() -> Self {
        Self::new()
    }
}

impl Manager {
    pub fn new() -> Self {
        Manager {
            registry: Mutex::new(Registry::default()),
        }
    }

    /// Create a session for a conversation: resolve directory and shell, spawn,
    /// register.
    pub fn create(&self, request: CreateRequest) -> Result<Info, ManagerError> {
        let resolved = cwd::resolve_for_thread(&request.thread_id, request.policy)?;
        let choice = shell::resolve_shell().map_err(ManagerError::ShellUnavailable)?;
        let args = choice.args();
        let program = choice.path;
        self.create_with(
            request.thread_id,
            request.title,
            program,
            args,
            resolved.path,
            request.cols,
            request.rows,
        )
    }

    /// Spawn a session against an explicit program/cwd. Split out so tests (and
    /// any future shell picker) can run a session without touching the store.
    #[allow(clippy::too_many_arguments)]
    pub fn create_with(
        &self,
        thread_id: String,
        title: Option<String>,
        program: PathBuf,
        args: Vec<String>,
        cwd: PathBuf,
        cols: u16,
        rows: u16,
    ) -> Result<Info, ManagerError> {
        self.prune_exited();
        {
            let registry = self.registry.lock().unwrap();
            if registry.sessions.len() >= MAX_SESSIONS {
                return Err(ManagerError::Capacity(format!(
                    "CAPACITY_EXCEEDED: at most {MAX_SESSIONS} terminals can be open at once"
                )));
            }
            let for_thread = registry
                .sessions
                .values()
                .filter(|session| session.thread_id() == thread_id)
                .count();
            if for_thread >= MAX_SESSIONS_PER_THREAD {
                return Err(ManagerError::Capacity(format!(
                    "CAPACITY_EXCEEDED: at most {MAX_SESSIONS_PER_THREAD} terminals per conversation"
                )));
            }
        }

        let title = title.unwrap_or_else(|| "Terminal".to_string());
        let cols = cols.clamp(1, 1000);
        let rows = rows.clamp(1, 1000);
        let session = Session::spawn(thread_id, title, program, args, cwd, cols, rows)
            .map_err(ManagerError::Spawn)?;
        let info = session.info();
        self.registry
            .lock()
            .unwrap()
            .sessions
            .insert(info.id.clone(), session);
        Ok(info)
    }

    pub fn list(&self, thread_id: Option<&str>) -> Vec<Info> {
        self.prune_exited();
        let registry = self.registry.lock().unwrap();
        let mut infos: Vec<Info> = registry
            .sessions
            .values()
            .filter(|session| thread_id.is_none_or(|id| session.thread_id() == id))
            .map(|session| session.info())
            .collect();
        infos.sort_by(|a, b| a.id.cmp(&b.id));
        infos
    }

    pub fn get(&self, id: &str) -> Result<Info, ManagerError> {
        Ok(self.session(id)?.info())
    }

    pub fn update(
        &self,
        id: &str,
        title: Option<String>,
        size: Option<(u16, u16)>,
    ) -> Result<Info, ManagerError> {
        Ok(self.session(id)?.update(title, size))
    }

    pub fn attach(
        &self,
        id: &str,
        cursor: Option<i64>,
    ) -> Result<super::session::Attachment, ManagerError> {
        Ok(self.session(id)?.attach(cursor))
    }

    /// Close one session: terminate its process tree, then drop it from the
    /// registry so its buffer is released.
    pub fn remove(&self, id: &str) -> Result<(), ManagerError> {
        let session = self.session(id)?;
        {
            let mut registry = self.registry.lock().unwrap();
            registry.sessions.remove(id);
            registry.exit_order.retain(|entry| entry != id);
        }
        session.close(CLOSE_GRACE);
        Ok(())
    }

    /// Close every session belonging to a conversation. Used when the
    /// conversation (or its workspace) is deleted: no shell may outlive the
    /// context it was opened in.
    pub fn close_thread(&self, thread_id: &str) {
        let sessions: Vec<Arc<Session>> = {
            let registry = self.registry.lock().unwrap();
            registry
                .sessions
                .values()
                .filter(|session| session.thread_id() == thread_id)
                .cloned()
                .collect()
        };
        for session in sessions {
            let _ = self.remove(session.id());
        }
    }

    /// Stop everything. Called from the app's exit path, so the grace period is
    /// deliberately short and the total wait is bounded.
    pub fn shutdown_all(&self, grace: Duration) {
        let sessions: Vec<Arc<Session>> = {
            let registry = self.registry.lock().unwrap();
            registry.sessions.values().cloned().collect()
        };
        for session in sessions {
            session.close(grace);
        }
        let mut registry = self.registry.lock().unwrap();
        registry.sessions.clear();
        registry.exit_order.clear();
    }

    #[cfg(test)]
    pub fn running_count(&self) -> usize {
        let registry = self.registry.lock().unwrap();
        registry
            .sessions
            .values()
            .filter(|session| session.is_running())
            .count()
    }

    /// Mark a session as exited so eviction can order it. The manager learns
    /// about exits lazily (on the next operation) rather than through a
    /// callback: nothing in the manager's own state depends on the timing, and
    /// a callback would need a second lock in the reader thread's hot path.
    fn note_exit(&self, id: &str) {
        let mut registry = self.registry.lock().unwrap();
        if !registry.exit_order.iter().any(|entry| entry == id) {
            registry.exit_order.push_back(id.to_string());
        }
    }

    /// Drop the oldest exited sessions beyond [`EXITED_LIMIT`].
    fn prune_exited(&self) {
        let exited: Vec<(String, Arc<Session>)> = {
            let registry = self.registry.lock().unwrap();
            registry
                .sessions
                .iter()
                .filter(|(_, session)| !session.is_running())
                .map(|(id, session)| (id.clone(), Arc::clone(session)))
                .collect()
        };
        for (id, _) in &exited {
            self.note_exit(id);
        }

        let victims: Vec<Arc<Session>> = {
            let mut registry = self.registry.lock().unwrap();
            let mut victims = Vec::new();
            while registry.exit_order.len() > EXITED_LIMIT {
                let Some(id) = registry.exit_order.pop_front() else {
                    break;
                };
                // Only evict sessions that are still exited; a running session
                // must never be dropped because of retention bookkeeping.
                if let Some(session) = registry.sessions.get(&id) {
                    if session.is_running() {
                        continue;
                    }
                    victims.push(Arc::clone(session));
                    registry.sessions.remove(&id);
                }
            }
            victims
        };
        for session in victims {
            session.close(CLOSE_GRACE);
        }
    }

    fn session(&self, id: &str) -> Result<Arc<Session>, ManagerError> {
        let registry = self.registry.lock().unwrap();
        registry
            .sessions
            .get(id)
            .cloned()
            .ok_or_else(|| ManagerError::NotFound(id.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn manager_with_shell() -> Manager {
        Manager::new()
    }

    fn create(manager: &Manager, thread: &str, script: &str) -> Info {
        manager
            .create_with(
                thread.to_string(),
                Some("Terminal".to_string()),
                PathBuf::from("/bin/sh"),
                vec!["-c".to_string(), script.to_string()],
                std::env::temp_dir(),
                80,
                24,
            )
            .expect("create session")
    }

    #[cfg(unix)]
    #[test]
    fn lists_only_the_requested_conversation() {
        let manager = manager_with_shell();
        let a = create(&manager, "thread-a", "sleep 30");
        let b = create(&manager, "thread-b", "sleep 30");
        let listed = manager.list(Some("thread-a"));
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, a.id);
        assert_eq!(manager.list(None).len(), 2);
        assert_ne!(a.id, b.id);
        manager.shutdown_all(Duration::from_millis(200));
    }

    #[cfg(unix)]
    #[test]
    fn removing_a_session_kills_it_and_forgets_it() {
        let manager = manager_with_shell();
        let info = create(&manager, "thread-a", "sleep 30");
        assert!(
            manager.get(&info.id).expect("exists").status == super::super::session::Status::Running
        );
        manager.remove(&info.id).expect("remove");
        let error = manager.get(&info.id).expect_err("gone");
        assert_eq!(error.status(), 404);
        assert_eq!(error.code(), "TERMINAL_NOT_FOUND");
    }

    #[cfg(unix)]
    #[test]
    fn closing_a_conversation_closes_its_terminals_only() {
        let manager = manager_with_shell();
        let a = create(&manager, "thread-a", "sleep 30");
        let b = create(&manager, "thread-b", "sleep 30");
        manager.close_thread("thread-a");
        assert!(manager.get(&a.id).is_err());
        assert!(manager.get(&b.id).is_ok());
        manager.shutdown_all(Duration::from_millis(200));
    }

    #[cfg(unix)]
    #[test]
    fn exited_sessions_stay_listed_until_evicted() {
        let manager = manager_with_shell();
        let info = create(&manager, "thread-a", "exit 4");
        let deadline = Instant::now() + Duration::from_secs(5);
        while manager.get(&info.id).expect("exists").status
            == super::super::session::Status::Running
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        let listed = manager.list(Some("thread-a"));
        assert_eq!(listed.len(), 1, "an exited tab is still observable");
        assert_eq!(listed[0].exit_code, Some(4));
    }

    #[cfg(unix)]
    #[test]
    fn the_exited_retention_limit_is_enforced() {
        let manager = manager_with_shell();
        let mut ids = Vec::new();
        for index in 0..(EXITED_LIMIT + 3) {
            // One conversation each: the per-conversation cap is a separate
            // limit and must not mask the retention behaviour under test.
            let info = create(&manager, &format!("thread-{index}"), "exit 0");
            ids.push(info.id);
        }
        // Wait for all of them to exit, then let the manager prune.
        let deadline = Instant::now() + Duration::from_secs(10);
        while manager.running_count() > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        manager.prune_exited();
        let listed = manager.list(None);
        assert!(
            listed.len() <= EXITED_LIMIT,
            "retention must be bounded, got {}",
            listed.len()
        );
    }

    #[cfg(unix)]
    #[test]
    fn per_conversation_capacity_is_enforced() {
        let manager = manager_with_shell();
        let mut created = Vec::new();
        for _ in 0..MAX_SESSIONS_PER_THREAD {
            created.push(create(&manager, "thread-a", "sleep 30"));
        }
        let error = manager
            .create_with(
                "thread-a".to_string(),
                None,
                PathBuf::from("/bin/sh"),
                vec!["-c".to_string(), "sleep 30".to_string()],
                std::env::temp_dir(),
                80,
                24,
            )
            .expect_err("capacity");
        assert_eq!(error.code(), "CAPACITY_EXCEEDED");
        assert_eq!(error.status(), 429);
        manager.shutdown_all(Duration::from_millis(200));
    }
}
