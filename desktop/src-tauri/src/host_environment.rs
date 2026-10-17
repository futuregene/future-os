//! One-time, pre-runtime host setup. Helper dispatch never enters this path.
static INITIALIZED: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();

pub(crate) fn initialize() -> Result<(), crate::AppError> {
    INITIALIZED
        .get_or_init(|| {
            future_agent::logfile::init_embedded_tracing();
            crate::install_rustls_provider();
            future_agent::sandbox::hydrate_from_login_shell();
            crate::cli_tool::prepare_path().map_err(|e| e.to_string())
        })
        .clone()
        .map_err(Into::into)
}

/// Separate host log prevents attachment/restart from opening an external
/// Agent's log. Call after acquiring the Desktop lock; it lives until exit.
pub(crate) fn start_logging() -> Result<(), crate::AppError> {
    future_agent::logfile::init(&log_file_path(), future_agent::logfile::DEFAULT_MAX_LINES)?;
    report(&format!(
        "host starting version={} pid={} mode={} log={}",
        crate::build_info::VERSION,
        std::process::id(),
        std::env::var("FUTURE_DESKTOP_AGENT_MODE").unwrap_or_else(|_| "auto".into()),
        log_file_path().display()
    ));
    if let Err(error) = crate::cli_tool::resolve() {
        report(&error.to_string());
    }
    Ok(())
}

/// Where this host and its embedded Agent write their log. Available before
/// [`start_logging`] runs, so an entry point can name it even when startup
/// fails early or the host is killed before any diagnostics exist.
pub(crate) fn log_file_path() -> std::path::PathBuf {
    future_rpc::home::future_home().join("app/logs/desktop.log")
}

pub(crate) fn report(message: &str) {
    eprintln!("FutureOS: {message}");
    if let Some(mirror) = future_agent::logfile::get() {
        mirror.write_stripped(&format!("FutureOS: {message}\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host with no resolvable CLI still starts logging: the missing tool is
    /// *reported* through the log and the version line is written either way, so
    /// an uninstalled CLI never fails the launch. Whether `cli_tool::resolve()`
    /// succeeds is what selects the arm, so the fixture pins it instead of
    /// leaving this line at the mercy of the host's PATH.
    #[test]
    fn a_host_without_a_resolvable_cli_still_starts_logging() {
        let _home = crate::auth_store::test_support::HomeGuard::new("host-start-logging");
        let empty = tempfile::tempdir().expect("an empty directory to search");
        // A PATH to restore is a precondition of this fixture, not a branch: the
        // `None` arm would be a line no run executes.
        let saved = std::env::var_os("PATH").expect("this host has a PATH to restore");
        // The test rebuild's candidates are the executable's own directory and
        // PATH; neither holds a `future` here (the dev fallbacks are
        // `#[cfg(not(test))]`).
        std::env::set_var("PATH", empty.path());
        let unresolvable = crate::cli_tool::resolve().is_err();
        let result = start_logging();
        std::env::set_var("PATH", saved);
        assert!(
            unresolvable,
            "the fixture needs a host where `future` cannot resolve"
        );
        assert!(result.is_ok(), "{result:?}");
    }
}
