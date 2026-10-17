//! The sidecar-launch half of `agent_supervisor::ensure_agent_running`.
//!
//! That path is compiled out of the unit-test build — `ensure_agent_running`
//! refuses to launch anything when `cfg!(test)` — so only an integration test,
//! which links the library *without* `cfg(test)`, can drive it. Nothing real is
//! started: the `future` found on `PATH` is a stub script that exits
//! immediately, and the FutureOS home is a scratch directory, so the probe can
//! never reach a developer's own agent.
//!
//! It is deliberately the only test in this file: it mutates the process-global
//! `PATH` and `FUTURE_HOME`.

/// The probe window this fixture gives the launch loop.
///
/// The loop's *default* window is a product budget (how long a user waits for a
/// cold sidecar), and this fixture must not assert the host's speed against it:
/// the candidate here is a `/bin/sh` script, so what the loop waits for is the
/// kernel's fork/exec for that child, measured at **~375 ms** on an idle host
/// (diagnostic: `alive at attempt 0/1/2`, `exit seen at attempt 3`). Under the
/// batch load this flake was found in, that latency is the host's to decide, and a
/// window shorter than it makes the loop kill the child **silently** — the report
/// then lists only the candidates that failed to *spawn*, omits the one that
/// spawned (which is what the assertion below is about) and fails with the very
/// message this flake recorded for 34 runs. Positive control: a 300 ms window
/// reproduces that message character for character. 60 s is 160x the measured
/// latency and matches the sibling fixture's window in
/// `agent_sidecar_readiness.rs` (see `docs/testing/flaky-tui.md` §7.1 and §11.3).
const LAUNCH_FIXTURE_WINDOW: std::time::Duration = std::time::Duration::from_secs(60);

/// Restore an environment variable to what it was.
fn restore(key: &str, previous: Option<std::ffi::OsString>) {
    match previous {
        Some(value) => std::env::set_var(key, value),
        None => std::env::remove_var(key),
    }
}

/// Every launch candidate that starts and then exits is reported with its exit
/// status, the next candidate is still tried, and the failure names them all.
#[cfg(unix)]
#[test]
fn a_sidecar_candidate_that_exits_is_reported_and_the_next_one_is_tried() {
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt as _;

    let home = tempfile::tempdir().expect("tempdir");
    let tools = tempfile::tempdir().expect("tempdir");
    let stub = tools.path().join("future");
    let mut file = std::fs::File::create(&stub).expect("create the stub");
    file.write_all(b"#!/bin/sh\nexit 3\n")
        .expect("write the stub");
    file.flush().expect("flush the stub");
    drop(file);
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    let old_path = std::env::var_os("PATH");
    // Only the stub's directory: no `future-agent` exists anywhere either, so
    // the second candidate fails to spawn and lands in the same report.
    std::env::set_var("PATH", tools.path());
    let old_home = std::env::var_os("FUTURE_HOME");
    std::env::set_var("FUTURE_HOME", home.path());

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    // The fixture's own window: see LAUNCH_FIXTURE_WINDOW — the exit status this
    // test asserts on has to be *observed* by the loop, and the loop only observes
    // it if the window outlasts the host's fork/exec for the stub.
    let outcome = runtime.block_on(future_tui::agent_supervisor::ensure_agent_running_within(
        "auto",
        LAUNCH_FIXTURE_WINDOW,
    ));

    restore("PATH", old_path);
    restore("FUTURE_HOME", old_home);

    let error = match outcome {
        Ok(_) => panic!("nothing is reachable and no candidate can start an agent"),
        Err(error) => error,
    };
    assert!(
        error.contains("future exited with exit status: 3"),
        "the candidate's exit status must be reported verbatim: {error}"
    );
    assert!(
        error.contains("future-agent"),
        "the next candidate must be tried and reported too: {error}"
    );
    assert!(
        error.contains("no sidecar could be started"),
        "the failure must name what it tried: {error}"
    );
}
