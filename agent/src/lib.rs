//! future-agent — Rust implementation of the FutureAgent agent backend

pub mod agent;
pub mod approval_review;
pub mod auth;
pub mod cli;
pub mod compaction;
pub mod config;
pub mod console;
pub mod engine;
pub mod grpc;
pub mod http;
pub mod llm;
pub mod logfile;
pub mod models;
pub mod prompt;
pub mod rpc;
pub mod runtime;
pub mod sandbox;
pub mod service;
pub mod session;
pub mod skill_reco;
pub mod skills;
mod system_one;
pub mod tools;
pub mod types;
pub mod utils;

pub use agent::{Loop, RunEvent};
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
pub use types::{AgentMessage, AgentTool, LLMProvider, Message, ToolDef};
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
    /// Native subprocess probe for a PowerShell parent's console. Reuses the
    /// already-built test executable instead of compiling C# through Add-Type.
    #[cfg(windows)]
    pub(crate) fn console_probe_command() -> String {
        let executable = std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .replace('\'', "''");
        format!("$env:FUTURE_AGENT_CONSOLE_PROBE='1'; & '{executable}' --exact test_support::console_probe_child --nocapture")
    }

    #[cfg(windows)]
    #[test]
    fn console_probe_child() {
        if std::env::var_os("FUTURE_AGENT_CONSOLE_PROBE").is_none() {
            return; // Entry point used only by the two shell subprocess fixtures.
        }
        use windows_sys::Win32::Foundation::ERROR_INVALID_HANDLE;
        use windows_sys::Win32::System::Console::{
            AttachConsole, FreeConsole, GetConsoleWindow, ATTACH_PARENT_PROCESS,
        };
        // Inspect the shell's console, regardless of how it launched this child.
        // FreeConsole affects only this isolated subprocess, never libtest's parent.
        unsafe { FreeConsole() };
        let attached = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
        let window = if attached != 0 {
            (unsafe { GetConsoleWindow() }) as usize
        } else {
            let error = std::io::Error::last_os_error();
            assert_eq!(
                error.raw_os_error(),
                Some(ERROR_INVALID_HANDLE as i32),
                "{error}"
            );
            0
        };
        println!("FUTUREOS_PARENT_CONSOLE={window}");
    }

    /// Take the process-global HOME lock (poison-tolerant).
    pub(crate) fn home_env_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::HOME_ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Isolate tests that replace process-wide shell state and cached probes.
    #[cfg(not(target_os = "windows"))]
    pub(crate) fn reexec_env_test() -> bool {
        let thread = std::thread::current();
        let name = thread.name().expect("named libtest thread");
        const CHILD: &str = "FUTURE_AGENT_ISOLATED_ENV_TEST";
        if std::env::var(CHILD).as_deref() == Ok(name) {
            return false;
        }
        let home = tempfile::tempdir().unwrap();
        let child = {
            let _lock = home_env_lock();
            std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", name, "--nocapture"])
                .env(CHILD, name)
                .env("HOME", home.path())
                .env("USERPROFILE", home.path())
                .env_remove("FUTURE_HOME")
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("spawn isolated shell test")
        };
        let output = child.wait_with_output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("1 passed; 0 failed"),
            "isolated {name}: {stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        true
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

    /// An absolute path in the *host's* spelling from a POSIX-spelled
    /// argument. The Linux-helper protocol is POSIX, but the code under test
    /// validates host paths (`Path::is_absolute`), so a fixture that is
    /// absolute on unix has to gain a drive prefix on Windows to reach the
    /// same accept branch. On unix the string is returned unchanged.
    pub(crate) fn host_absolute_path(path: &str) -> std::path::PathBuf {
        if cfg!(windows) {
            std::path::PathBuf::from(format!(
                "C:\\{}",
                path.trim_start_matches('/').replace('/', "\\")
            ))
        } else {
            std::path::PathBuf::from(path)
        }
    }

    /// Redirect HOME/USERPROFILE to an isolated directory for the duration of
    /// a test. The directory is anchored under the workspace target/ dir —
    /// never the system temp dir, whose writes sandbox rules allow (a temp
    /// HOME would flip parallel sandbox tests' "outside" fixtures to Allow).
    pub(crate) struct TestHome {
        previous_home: Option<std::ffi::OsString>,
        previous_userprofile: Option<std::ffi::OsString>,
        previous_future_home: Option<std::ffi::OsString>,
        dir: tempfile::TempDir,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl TestHome {
        pub(crate) fn new() -> Self {
            Self::at(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .expect("workspace root")
                    .join("target/test-homes"),
            )
        }

        /// A home whose absolute path is short enough for a unix `sun_path`.
        ///
        /// [`TestHome::new`] anchors the home under `<workspace>/target/test-homes`
        /// so sandbox rules keep matching (see its comment). That anchor is also
        /// as long as the checkout is deep: in a git worktree the derived
        /// `<home>/.future/run/agent.sock` measured 106 bytes against the 104-byte
        /// `sun_path` limit, and binding then fails with `EPERM` — a test that
        /// binds a local endpoint fails for a reason that has nothing to do with
        /// the code under test.
        ///
        /// The platform temp root is short on every supported system, and a test
        /// that starts a real listener does not depend on sandbox rule matching,
        /// so it uses this instead. Everything else keeps `new()`.
        #[cfg_attr(not(unix), allow(dead_code))] // the limit it avoids is unix-only
        pub(crate) fn short() -> Self {
            Self::at(std::env::temp_dir())
        }

        fn at(base: std::path::PathBuf) -> Self {
            let guard = home_env_lock();
            let previous_home = std::env::var_os("HOME");
            let previous_userprofile = std::env::var_os("USERPROFILE");
            let previous_future_home = std::env::var_os("FUTURE_HOME");
            std::fs::create_dir_all(&base).expect("create test-homes dir");
            let dir = tempfile::tempdir_in(base).expect("tempdir");
            // Use the CANONICAL dir as $HOME: on macOS /var -> /private/var
            // is a symlink, and sandbox rules canonicalize their bases — a raw
            // (non-canonical) $HOME would make raw dirs::home_dir() paths never
            // match canonicalized rule bases.
            let canonical_home = crate::sandbox::paths::canonicalize_lenient(dir.path());
            std::env::remove_var("FUTURE_HOME");
            std::env::set_var("HOME", &canonical_home);
            std::env::set_var("USERPROFILE", &canonical_home);
            Self {
                previous_home,
                previous_userprofile,
                previous_future_home,
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
            restore_env("FUTURE_HOME", &self.previous_future_home);
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
        async fn stream_model(
            &self,
            _request: crate::llm::schema::ModelRequest,
        ) -> anyhow::Result<
            tokio_stream::wrappers::ReceiverStream<crate::llm::schema::ModelStreamEvent>,
        > {
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
                .stream_model(crate::llm::schema::ModelRequest {
                    model: "mock".into(),
                    system_prompt: String::new(),
                    messages: vec![],
                    tools: vec![],
                })
                .await
                .unwrap();
            assert!(stream.next().await.is_none());
        }
    }

    #[cfg(test)]
    mod env_restore_tests {
        #[test]
        fn test_home_isolates_and_restores_an_inherited_future_home() {
            const CHILD: &str = "FUTURE_AGENT_HOME_GUARD_PROBE";
            if std::env::var_os(CHILD).is_none() {
                let inherited = tempfile::tempdir().unwrap();
                let output = std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "test_support::env_restore_tests::test_home_isolates_and_restores_an_inherited_future_home", "--nocapture"])
                    .env(CHILD, "1").env("FUTURE_HOME", inherited.path())
                    .output().unwrap();
                let stdout = String::from_utf8_lossy(&output.stdout);
                assert!(
                    output.status.success() && stdout.contains("1 passed; 0 failed"),
                    "{stdout}\n{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                return;
            }
            let inherited = std::env::var_os("FUTURE_HOME").unwrap();
            let home = super::TestHome::new();
            assert!(std::env::var_os("FUTURE_HOME").is_none());
            assert_eq!(
                crate::utils::future_home(),
                crate::sandbox::paths::canonicalize_lenient(home.path()).join(".future")
            );
            drop(home);
            assert_eq!(std::env::var_os("FUTURE_HOME"), Some(inherited));
        }

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
