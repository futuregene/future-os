//! Session registry: create, list, update, attach, close.
//!
//! The manager owns the only strong references to live sessions and keeps the
//! exited ones around for a bounded while so a reloaded view can still read the
//! final screen and the exit code.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::cwd;
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
        let resolved = cwd::resolve_for_thread(&request.thread_id)?;
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

    /// The session behind an id, for tests that must sequence an exit.
    ///
    /// `Manager` deliberately learns about exits lazily (see `note_exit`), so
    /// nothing in its public surface reports one; the transport tests need the
    /// *exited* state to exist in the registry to exercise the pump's
    /// exited-before-attach and mid-stream-exit branches, and on this host the
    /// only way to produce it is to call the same `Session::on_eof` that
    /// `close()` and the reader thread call. Test-only, and it exposes no
    /// behaviour the manager itself does not already have.
    #[cfg(test)]
    pub(crate) fn session_for_test(&self, id: &str) -> Option<Arc<super::session::Session>> {
        self.registry.lock().unwrap().sessions.get(id).cloned()
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
    use crate::terminal::test_support;

    fn manager_with_shell() -> Manager {
        Manager::new()
    }

    fn create_with_command(
        manager: &Manager,
        thread: &str,
        command: (PathBuf, Vec<String>),
    ) -> Info {
        let (program, args) = command;
        manager
            .create_with(
                thread.to_string(),
                Some("Terminal".to_string()),
                program,
                args,
                test_support::working_dir(),
                80,
                24,
            )
            .expect("create session")
    }

    /// A conversation tab running a live, silent child.
    fn create(manager: &Manager, thread: &str) -> Info {
        create_with_command(manager, thread, test_support::idle_command())
    }

    /// A conversation tab whose child exits immediately with `code`.
    fn create_exiting(manager: &Manager, thread: &str, code: i32) -> Info {
        create_with_command(manager, thread, test_support::exit_command(code))
    }

    /// Note an exit through the session's own state machine.
    ///
    /// `Manager` learns about exits lazily, from `Session::is_running()` — which
    /// is why `prune_exited`/retention are testable without the manager owning
    /// any exit callback. On this host the ConPTY master never closes, so the
    /// reader thread's EOF never arrives and a session can only be marked exited
    /// by driving `Session::on_eof` (the function the reader *and* `close()`
    /// both call). `on_eof` polls `PtySession::try_wait` itself for up to
    /// `EXIT_WAIT`, so a child that really exits is recorded with its real code.
    fn note_exit_now(manager: &Manager, info: &Info) {
        let session = manager.session(&info.id).expect("session");
        // Reap first, then drive the exit: `on_eof` records whatever `try_wait`
        // reports, so calling it before the process is gone would record `None`
        // for a session that really exited with a code.
        assert!(
            session.wait_for_child_exit(Duration::from_secs(30)),
            "the fixture child never exited; the retention assertions below would be vacuous"
        );
        session.on_eof();
    }

    #[test]
    fn lists_only_the_requested_conversation() {
        let manager = manager_with_shell();
        let a = create(&manager, "thread-a");
        let b = create(&manager, "thread-b");
        let listed = manager.list(Some("thread-a"));
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, a.id);
        assert_eq!(manager.list(None).len(), 2);
        assert_ne!(a.id, b.id);
        manager.shutdown_all(Duration::from_millis(200));
    }

    #[test]
    fn removing_a_session_kills_it_and_forgets_it() {
        let manager = manager_with_shell();
        let info = create(&manager, "thread-a");
        assert_eq!(
            manager.get(&info.id).expect("exists").status,
            super::super::session::Status::Running
        );
        manager.remove(&info.id).expect("remove");
        let error = manager.get(&info.id).expect_err("gone");
        assert_eq!(error.status(), 404);
        assert_eq!(error.code(), "TERMINAL_NOT_FOUND");
    }

    #[test]
    fn closing_a_conversation_closes_its_terminals_only() {
        let manager = manager_with_shell();
        let a = create(&manager, "thread-a");
        let b = create(&manager, "thread-b");
        manager.close_thread("thread-a");
        assert!(manager.get(&a.id).is_err());
        assert!(manager.get(&b.id).is_ok());
        manager.shutdown_all(Duration::from_millis(200));
    }

    #[test]
    fn exited_sessions_stay_listed_until_evicted() {
        let manager = manager_with_shell();
        let info = create_exiting(&manager, "thread-a", 4);
        note_exit_now(&manager, &info);
        let listed = manager.list(Some("thread-a"));
        assert_eq!(listed.len(), 1, "an exited tab is still observable");
        assert_eq!(listed[0].exit_code, Some(4));
    }

    #[test]
    fn the_exited_retention_limit_is_enforced() {
        let manager = manager_with_shell();
        let mut infos = Vec::new();
        for index in 0..(EXITED_LIMIT + 3) {
            // One conversation each: the per-conversation cap is a separate
            // limit and must not mask the retention behaviour under test.
            let info = create_exiting(&manager, &format!("thread-{index}"), 0);
            note_exit_now(&manager, &info);
            infos.push(info);
        }
        // Every session has really exited, so the manager can prune.
        assert_eq!(manager.running_count(), 0, "no session may still run");
        manager.prune_exited();
        let listed = manager.list(None);
        assert!(
            listed.len() <= EXITED_LIMIT,
            "retention must be bounded, got {}",
            listed.len()
        );
        assert_eq!(
            listed.len(),
            EXITED_LIMIT,
            "exactly the retention limit survives"
        );
        let addressable = infos
            .iter()
            .filter(|info| manager.get(&info.id).is_ok())
            .count();
        assert_eq!(addressable, EXITED_LIMIT);
    }

    /// A running session is never evicted by retention bookkeeping, even when
    /// it is the oldest entry in the exit order.
    #[test]
    fn retention_never_evicts_a_running_session() {
        let manager = manager_with_shell();
        let running = create(&manager, "thread-running");
        let mut exited = Vec::new();
        for index in 0..(EXITED_LIMIT + 2) {
            let info = create_exiting(&manager, &format!("thread-exit-{index}"), 0);
            note_exit_now(&manager, &info);
            exited.push(info);
        }
        assert_eq!(
            manager.running_count(),
            1,
            "only the live tab may still be running"
        );
        // `list` prunes; the live tab must survive it.
        let listed = manager.list(None);
        assert!(listed.iter().any(|info| info.id == running.id));
        assert!(manager.get(&running.id).is_ok());
        manager.shutdown_all(Duration::from_millis(200));
    }

    #[test]
    fn per_conversation_capacity_is_enforced() {
        let manager = manager_with_shell();
        let mut created = Vec::new();
        for _ in 0..MAX_SESSIONS_PER_THREAD {
            created.push(create(&manager, "thread-a"));
        }
        let (program, args) = test_support::idle_command();
        let error = manager
            .create_with(
                "thread-a".to_string(),
                None,
                program,
                args,
                test_support::working_dir(),
                80,
                24,
            )
            .expect_err("capacity");
        assert_eq!(error.code(), "CAPACITY_EXCEEDED");
        assert_eq!(error.status(), 429);
        assert_eq!(
            error.message(),
            format!(
                "CAPACITY_EXCEEDED: at most {MAX_SESSIONS_PER_THREAD} terminals per conversation"
            )
        );
        manager.shutdown_all(Duration::from_millis(200));
    }

    /// The global cap is a separate limit from the per-conversation one.
    #[test]
    fn global_capacity_is_enforced_across_conversations() {
        let manager = manager_with_shell();
        for index in 0..MAX_SESSIONS {
            create(
                &manager,
                &format!("thread-{}", index / MAX_SESSIONS_PER_THREAD),
            );
        }
        let (program, args) = test_support::idle_command();
        let error = manager
            .create_with(
                "thread-overflow".to_string(),
                None,
                program,
                args,
                test_support::working_dir(),
                80,
                24,
            )
            .expect_err("global capacity");
        assert_eq!(error.code(), "CAPACITY_EXCEEDED");
        assert_eq!(error.status(), 429);
        assert_eq!(
            error.message(),
            format!("CAPACITY_EXCEEDED: at most {MAX_SESSIONS} terminals can be open at once")
        );
        manager.shutdown_all(Duration::from_millis(200));
    }

    /// Create/update/get/list/attach errors all carry a stable code, a wire
    /// status and a message that names the conversation.
    #[test]
    fn update_and_get_report_a_missing_session() {
        let manager = manager_with_shell();
        let error = manager
            .update("ghost", Some("t".into()), Some((80, 24)))
            .expect_err("missing session");
        assert_eq!(error.code(), "TERMINAL_NOT_FOUND");
        assert_eq!(error.status(), 404);
        assert!(error.message().contains("ghost"));
        assert!(manager.attach("ghost", None).is_err());
        assert!(manager.list(None).is_empty());
    }

    /// `create`'s two error arms: an unresolved conversation, and a program the
    /// PTY cannot spawn.
    #[test]
    fn create_reports_store_and_spawn_failures() {
        let _home = crate::auth_store::test_support::HomeGuard::new("terminal_manager_create");
        crate::store::initialize_app_store().expect("init store");
        let manager = manager_with_shell();
        let error = manager
            .create(CreateRequest {
                thread_id: "ghost-thread".to_string(),
                title: None,
                cols: 80,
                rows: 24,
            })
            .expect_err("unknown conversation");
        assert_eq!(error.code(), "THREAD_NOT_FOUND");

        let missing = std::env::temp_dir().join("futureos-no-such-program-xyz");
        let error = manager
            .create_with(
                "ghost-thread".to_string(),
                None,
                missing,
                Vec::new(),
                test_support::working_dir(),
                80,
                24,
            )
            .expect_err("spawn must fail");
        assert_eq!(error.code(), "SPAWN_FAILED");
        assert!(!error.message().is_empty());
    }

    /// `create_with` clamps a hostile terminal size instead of handing the PTY
    /// a zero-column window.
    #[test]
    fn created_sessions_clamp_their_size() {
        let manager = manager_with_shell();
        let (program, args) = test_support::idle_command();
        let info = manager
            .create_with(
                "thread-clamp".to_string(),
                None,
                program,
                args,
                test_support::working_dir(),
                0,
                u16::MAX,
            )
            .expect("create");
        assert_eq!((info.cols, info.rows), (1, 1000));
        assert_eq!(info.title, "Terminal", "a default title is applied");
        manager.shutdown_all(Duration::from_millis(200));
    }

    /// `update` reaches a live session (retitle + resize) and reports the new
    /// state.
    #[test]
    fn update_retitles_and_resizes_a_live_session() {
        let manager = manager_with_shell();
        let info = create(&manager, "thread-update");
        let updated = manager
            .update(&info.id, Some("  Logs  ".into()), Some((132, 43)))
            .expect("update");
        assert_eq!(updated.title, "Logs");
        assert_eq!((updated.cols, updated.rows), (132, 43));
        manager.shutdown_all(Duration::from_millis(200));
    }

    #[test]
    fn attach_returns_the_replay_and_detaches_cleanly() {
        let manager = manager_with_shell();
        let info = create(&manager, "thread-attach");
        let mut attachment = manager.attach(&info.id, Some(-1)).expect("attach");
        assert!(attachment.replay.is_empty());
        attachment.detach();
        manager.shutdown_all(Duration::from_millis(200));
    }

    /// `shutdown_all` ends every session *and* empties the registry, so a later
    /// list cannot resurrect a tab.
    #[test]
    fn shutdown_all_clears_the_registry() {
        let manager = manager_with_shell();
        create(&manager, "thread-a");
        create(&manager, "thread-b");
        manager.shutdown_all(Duration::from_millis(300));
        assert!(manager.list(None).is_empty());
        assert_eq!(manager.running_count(), 0);
    }
}
