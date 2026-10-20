//! Shared host log path and diagnostics, independent of CLI discovery or GUI.
pub fn start_logging() -> Result<(), crate::Error> {
    future_agent::logfile::init(&log_file_path(), future_agent::logfile::DEFAULT_MAX_LINES)?;
    report(&format!(
        "host starting version={} pid={} mode={} log={}",
        crate::build_info::VERSION,
        std::process::id(),
        std::env::var("FUTURE_DESKTOP_AGENT_MODE").unwrap_or_else(|_| "auto".into()),
        log_file_path().display()
    ));
    Ok(())
}

/// Where this host and its embedded Agent write their log. Available before
/// [`start_logging`] runs, so an entry point can name it even when startup
/// fails early or the host is killed before any diagnostics exist.
pub fn log_file_path() -> std::path::PathBuf {
    future_rpc::home::future_home().join("app/logs/desktop.log")
}

pub fn report(message: &str) {
    eprintln!("FutureOS: {message}");
    if let Some(mirror) = future_agent::logfile::get() {
        mirror.write_stripped(&format!("FutureOS: {message}\n"));
    }
}
