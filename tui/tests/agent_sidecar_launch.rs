//! The failing half of the sidecar launch in
//! `agent_supervisor::ensure_agent_running`.
//!
//! That path is compiled out of the unit-test build — `ensure_agent_running`
//! refuses to launch anything when `cfg!(test)` — so only an integration test,
//! which links the library *without* `cfg(test)`, can drive it. Nothing real is
//! started: the program handed to the launch is a stub script that prints to
//! both of its streams and then exits, and the FutureOS home is a scratch
//! directory, so the probe can never reach a developer's own agent. Quoting
//! the stub's output in the failure report is the other half of what this file
//! exists to pin: a launch that swallows the sidecar's own words fails here.
//!
//! It is deliberately the only test in this file: it mutates the process-global
//! `FUTURE_HOME`.

/// The probe window this fixture gives the launch loop.
///
/// The loop's *default* window is a product budget (how long a user waits for a
/// cold sidecar), and this fixture must not assert the host's speed against it:
/// the stub is injected directly as the sidecar program, but it is still a
/// `/bin/sh` script, so what the loop waits for is the kernel's fork/exec for
/// that child, measured at **~375 ms** on an idle host (diagnostic: `alive at
/// attempt 0/1/2`, `exit seen at attempt 3`). Under the batch load this flake
/// was found in, that latency is the host's to decide, and a window shorter
/// than it makes the loop give up before the child's exit is *observed* — the
/// report then says the window ran out instead of naming the exit status the
/// assertion below is about. 60 s is 160x the measured latency and matches the
/// sibling fixture's window in `agent_sidecar_readiness.rs` (see
/// `docs/testing/flaky-tui.md` §7.1 and §11.3).
#[cfg(unix)]
const LAUNCH_FIXTURE_WINDOW: std::time::Duration = std::time::Duration::from_secs(60);

/// Restore an environment variable to what it was.
#[cfg(unix)]
fn restore(key: &str, previous: Option<std::ffi::OsString>) {
    match previous {
        Some(value) => std::env::set_var(key, value),
        None => std::env::remove_var(key),
    }
}

/// A sidecar that starts, writes to both of its streams and then exits is
/// reported with its exit status *and* its own words — the words being the
/// real failure reason a user needs — and the launch fails naming that single
/// attempt.
#[cfg(unix)]
#[test]
fn a_sidecar_that_exits_is_reported() {
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt as _;

    let home = tempfile::tempdir().expect("tempdir");
    let tools = tempfile::tempdir().expect("tempdir");
    let stub = tools.path().join("future");
    let mut file = std::fs::File::create(&stub).expect("create the stub");
    file.write_all(b"#!/bin/sh\necho starting\nprintf 'boom: bad config\\n' >&2\nexit 3\n")
        .expect("write the stub");
    file.flush().expect("flush the stub");
    drop(file);
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).expect("chmod");

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
        &stub,
        LAUNCH_FIXTURE_WINDOW,
    ));

    restore("FUTURE_HOME", old_home);

    let error = match outcome {
        Ok(_) => panic!("nothing is reachable and the stub cannot start an agent"),
        Err(error) => error,
    };
    assert!(
        error.contains("future exited with exit status: 3"),
        "the sidecar's exit status must be reported verbatim: {error}"
    );
    assert!(
        error.contains("boom: bad config"),
        "the sidecar's stderr must be carried into the report: {error}"
    );
    assert!(
        error.contains("starting"),
        "the sidecar's stdout must be carried into the report: {error}"
    );
    assert!(
        error.contains("the sidecar could not be started"),
        "the failure must name the attempt it gave up on: {error}"
    );
}
