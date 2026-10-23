//! Shared process preparation and diagnostics, independent of CLI discovery or GUI.

/// Hydrate the process before choosing Future home, taking its lock or starting
/// a runtime. Each entry applies its parsed relay options and CLI maintenance
/// discovery after this common preparation.
pub fn prepare_process() {
    static PREPARED: std::sync::Once = std::sync::Once::new();
    PREPARED.call_once(|| {
        future_agent::logfile::init_embedded_tracing();
        crate::install_rustls_provider();
        future_agent::sandbox::hydrate_from_login_shell();
        // A windowless GUI launch has nowhere to show a panic, and exit 101 is
        // not a crash Windows records — this hook is the only way a later
        // launch can learn what killed an earlier one.
        crate::crash_log::install();
    });
}

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
