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
