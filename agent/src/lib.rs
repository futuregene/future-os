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
            restore_env("HOME", &self.previous_home);
            restore_env("USERPROFILE", &self.previous_userprofile);
        }
    }

    /// Put an env var back to its pre-test state: restore the saved value, or
    /// remove the var when it was absent before the test redirected it.
    pub(crate) fn restore_env(key: &str, previous: &Option<std::ffi::OsString>) {
        match previous {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }

    /// A provider whose stream never yields events. Shared by every test
    /// that needs a provider the run never actually queries for content —
    /// per-line coverage counts each uncalled mock's body, so there is
    /// exactly one implementation and one test that drives it.
    pub(crate) struct EmptyProvider;

    #[async_trait::async_trait]
    impl crate::types::LLMProvider for EmptyProvider {
        async fn stream_chat(
            &self,
            _model: String,
            _messages: Vec<crate::types::Message>,
            _tools: Vec<crate::types::ToolDef>,
            _system_prompt: String,
        ) -> anyhow::Result<tokio_stream::wrappers::ReceiverStream<crate::types::StreamEvent>>
        {
            let (_tx, rx) = tokio::sync::mpsc::channel(1);
            Ok(tokio_stream::wrappers::ReceiverStream::new(rx))
        }
    }

    mod provider_tests {
        #[tokio::test(flavor = "current_thread")]
        async fn empty_provider_streams_nothing() {
            use crate::types::LLMProvider;
            use tokio_stream::StreamExt;
            let provider = super::EmptyProvider;
            let mut stream = provider
                .stream_chat("mock".to_string(), vec![], vec![], String::new())
                .await
                .unwrap();
            assert!(stream.next().await.is_none());
        }
    }

    #[cfg(test)]
    mod env_restore_tests {
        /// Both restore arms, exercised directly (the Some/None mix a real
        /// TestHome sees depends on the host env, which unit tests can't
        /// control — HOME is always set under cargo test, USERPROFILE never).
        #[test]
        fn restore_env_handles_present_and_absent_values() {
            let _guard = super::home_env_lock();
            let key = "FUTURE_TEST_RESTORE_ENV";
            std::env::set_var(key, "original");
            super::restore_env(key, &Some(std::ffi::OsString::from("saved")));
            assert_eq!(std::env::var(key).as_deref(), Ok("saved"));
            super::restore_env(key, &None);
            assert!(std::env::var_os(key).is_none());
        }
    }
}
