//! future-agent — Rust implementation of the FutureAgent agent backend

pub mod agent;
pub mod auth;
pub mod cli;
pub mod compaction;
pub mod config;
pub mod engine;
pub mod grpc;
pub mod llm;
pub mod logfile;
pub mod models;
pub mod prompt;
pub mod rpc;
pub mod runtime;
pub mod sandbox;
pub mod session;
pub mod skills;
pub mod tools;
pub mod types;
pub mod utils;

pub use agent::Loop;
pub use auth::AuthStore;
pub use config::{load_settings, Settings};
pub use engine::{Engine, EngineConfig};
pub use llm::Client as LLMClient;
pub use models::{get_default_model, Registry as ModelRegistry};
pub use rpc::ServerSession;
pub use session::{Manager, Session, SessionEntry};
pub use skills::{
    discover_skills, discover_skills_cached, global_skill_dirs, invalidate_skills_cache, Skill,
    AGENTS_SKILLS_DIR, APP_SKILLS_DIR,
};
pub use tools::{all_tools, coding_tools};
pub use types::{AgentMessage, AgentTool, LLMProvider, Message, StreamEvent, ToolDef};
pub use utils::{default_config_dir, default_session_dir, generate_id};

/// Process-global guard serializing tests that redirect `$HOME` (TestHome in
/// rpc::commands) against tests whose assertions read `dirs::home_dir()`
/// (secret-guard tests in rpc::approval). `$HOME` is process-wide mutable
/// state; without a shared lock a redirect window flips another test's
/// home-derived expectations under parallel execution.
#[cfg(test)]
pub(crate) static HOME_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Shared test scaffolding (unit tests only).
#[cfg(test)]
pub(crate) mod test_support {
    /// Take the process-global HOME lock (poison-tolerant).
    pub(crate) fn home_env_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::HOME_ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Unique temp path for a test fixture: timestamp plus a process-wide
    /// atomic sequence. The macOS clock ticks at ~µs granularity, so bare
    /// nanosecond stamps collide when two parallel tests draw the same
    /// tick — a shared path breaks tests that assume exclusive use of the
    /// directory (one creates it while another assumes it never exists).
    pub(crate) fn unique_temp_path(tag: &str) -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir().join(format!("futureos-{tag}-{stamp}-{seq}"))
    }

    /// Redirect HOME/USERPROFILE to an isolated directory for the duration of
    /// a test. The directory is anchored under the workspace target/ dir —
    /// never the system temp dir, whose writes sandbox rules allow (a temp
    /// HOME would flip parallel sandbox tests' "outside" fixtures to Allow).
    pub(crate) struct TestHome {
        previous_home: Option<std::ffi::OsString>,
        previous_userprofile: Option<std::ffi::OsString>,
        dir: tempfile::TempDir,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl TestHome {
        pub(crate) fn new() -> Self {
            let guard = home_env_lock();
            let previous_home = std::env::var_os("HOME");
            let previous_userprofile = std::env::var_os("USERPROFILE");
            let base = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("workspace root")
                .join("target/test-homes");
            std::fs::create_dir_all(&base).expect("create test-homes dir");
            let dir = tempfile::tempdir_in(base).expect("tempdir");
            // Use the CANONICAL dir as $HOME: on macOS /var -> /private/var
            // is a symlink, and sandbox rules canonicalize their bases — a raw
            // (non-canonical) $HOME would make raw dirs::home_dir() paths never
            // match canonicalized rule bases.
            let canonical_home = crate::sandbox::paths::canonicalize_lenient(dir.path());
            std::env::set_var("HOME", &canonical_home);
            std::env::set_var("USERPROFILE", &canonical_home);
            Self {
                previous_home,
                previous_userprofile,
                dir,
                _guard: guard,
            }
        }

        pub(crate) fn path(&self) -> &std::path::Path {
            self.dir.path()
        }

        pub(crate) fn auth_path(&self) -> std::path::PathBuf {
            self.dir.path().join(".future/agent/auth.json")
        }

        pub(crate) fn models_path(&self) -> std::path::PathBuf {
            self.dir.path().join(".future/agent/models.json")
        }

        pub(crate) fn settings_path(&self) -> std::path::PathBuf {
            self.dir.path().join(".future/agent/settings.json")
        }
    }

    impl Drop for TestHome {
        fn drop(&mut self) {
            match &self.previous_home {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
            match &self.previous_userprofile {
                Some(value) => std::env::set_var("USERPROFILE", value),
                None => std::env::remove_var("USERPROFILE"),
            }
        }
    }
}
