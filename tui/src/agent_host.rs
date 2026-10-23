//! The TUI hosts a complete Agent in its own process.
//!
//! Same policy as `future host` and the desktop: attach to an Agent that is
//! already serving; when none is and the instance lock is free, run the Agent in
//! this process. The Agent is therefore not a child of the terminal, which
//! removes the whole class of problems a child process brought: a terminal signal
//! killing the Agent, a zombie holding the endpoint, pipes that must be drained so
//! a chatty child never blocks, and output that had to be quoted back in a failure
//! report.
//!
//! One constraint decides when this is allowed at all: the Agent re-executes its
//! own executable as the Linux sandbox helper (`--linux-sandbox-helper`) and puts
//! that executable's directory on `PATH` for shell tools. Only the unified
//! `future` binary dispatches that flag (`cli/src/main.rs`), so the standalone
//! `future-tui` binary would break sandboxed shell commands by hosting the Agent.
//! It stays attach-only and says why — the same boundary the sidecar check
//! enforced, now stated in one place.

use std::path::Path;
use std::time::{Duration, Instant};

use future_host::agent_host::{self, AgentHost, Phase};

/// The name the unified binary must have to be allowed to host the Agent.
fn executable_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// Whether this process may host the Agent, or the reason it may not.
///
/// Attaching to an Agent that is already running works from any binary; only
/// *starting* one requires the unified executable.
pub fn hostable() -> Result<(), String> {
    let exe = std::env::current_exe()
        .map_err(|error| format!("cannot locate the current executable: {error}"))?;
    hostable_from(&exe)
}

/// [`hostable`] with the executable injected, so the rule is testable without
/// controlling `current_exe`.
fn hostable_from(exe: &Path) -> Result<(), String> {
    if executable_name(exe) == "future" {
        return Ok(());
    }
    Err(format!(
        "{} cannot host an Agent in-process: the Agent re-runs its own executable as the \
         Linux sandbox helper, which only the unified `future` binary dispatches. Run \
         `future tui`, or start an Agent first (`future agent`, or the desktop app).",
        exe.display()
    ))
}

/// The process-global Agent host, with this entry point's driver installed.
pub fn host() -> &'static AgentHost {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        agent_host::configure_driver(drive).expect("entry installs its Agent driver once");
    });
    agent_host::host()
}

/// Start the Agent in this process, or report why it did not start.
///
/// `AgentHost::start` waits for readiness, so it runs on a blocking thread: the
/// TUI's runtime is busy driving the UI, and a cold start can take minutes on a
/// network data root.
pub async fn start_in_process(configured: String) -> Result<(), String> {
    run_in_process(configured, false).await
}

pub async fn recover_in_process(configured: String) -> Result<(), String> {
    run_in_process(configured, true).await
}

async fn run_in_process(configured: String, repair: bool) -> Result<(), String> {
    hostable()?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("tui-agent-start".into())
        .spawn(move || {
            let result = if repair {
                host().recover(configured)
            } else {
                host().start(configured)
            };
            let _ = tx.send(result);
        })
        .map_err(|error| format!("cannot start the Agent worker thread: {error}"))?;
    match rx.await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err("the Agent worker stopped without reporting a result".into()),
    }
}

/// Whether this process currently owns (started) the Agent.
pub fn owns_agent() -> bool {
    host().owns_agent()
}

/// Stop an Agent this process owns, within a bound.
///
/// Called on the TUI's exit path: an owned Agent holds the instance lock and the
/// data-directory database, and leaving it to process teardown would make the
/// next start report a lock it cannot explain.
pub fn stop(timeout: Duration) -> Result<(), String> {
    let phase = host().phase();
    if matches!(phase, Phase::Idle | Phase::Stopped) || !host().owns_agent() {
        return Ok(());
    }
    host()
        .stop_until(Instant::now() + timeout)
        .map_err(|error| error.to_string())
}

/// The Agent driver: probe for an external service, otherwise run the service in
/// this process and serve it on this entry point's runtime.
///
/// Shaped exactly like the headless entry's driver: the probe gets its own
/// short-lived runtime and is dropped before the service is handed a runtime of
/// its own, so the transport used for discovery cannot outlive discovery.
fn drive(host: &AgentHost, configured: &str) -> Result<(), future_host::Error> {
    let probe = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let selected = probe.block_on(host.select(configured));
    drop(probe);
    host.run_selected(selected?, |prepared| {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("agent-runtime")
            .worker_threads(2)
            .max_blocking_threads(32)
            .thread_stack_size(2 * 1024 * 1024)
            .build()?;
        let result = runtime.block_on(host.serve(prepared));
        runtime.shutdown_timeout(Duration::from_secs(1));
        result
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only the unified binary may host: the Agent re-executes `current_exe` as
    /// the sandbox helper, and `future-tui` has no such dispatch.
    #[test]
    fn only_the_unified_binary_may_host() {
        assert!(hostable_from(Path::new("/usr/local/bin/future")).is_ok());
        assert!(hostable_from(Path::new("/opt/homebrew/bin/future")).is_ok());
        // The refusal must name the constraint and the way out, or the user is
        // left with "it does not work".
        for refused in ["/usr/local/bin/future-tui", "/usr/local/bin/futureos"] {
            let message =
                hostable_from(Path::new(refused)).expect_err("only the unified binary may host");
            assert!(
                message.contains("sandbox helper") && message.contains("future tui"),
                "{refused}: {message}"
            );
        }
    }
}
