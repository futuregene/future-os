//! Retained Desktop launch/retirement wiring. Actual headless process checks live in CLI.
use std::path::Path;
use std::process::{Command, Stdio};

fn desktop(home: &Path) -> Command {
    isolated_command(env!("CARGO_BIN_EXE_futureos"), home)
}

fn isolated_command(binary: &str, home: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("FUTURE_HOME", home.join(".future"))
        .env_remove("FUTURE_AGENT_SOCKET")
        .env("FUTURE_AGENT_GRPC_ADDR", "http://[::1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

#[cfg(feature = "gui")]
#[test]
fn help_and_invalid_arguments_have_no_startup_side_effects() {
    let home = tempfile::tempdir().unwrap();
    let help = desktop(home.path()).arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("Usage: futureos [--relay"));
    assert!(!home.path().join(".future").exists());
    for args in [
        vec!["--headless"],
        vec!["--headless", "--relay", "https://relay.example"],
        vec!["--help", "--headless"],
    ] {
        let invalid = desktop(home.path()).args(args).output().unwrap();
        assert_eq!(invalid.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&invalid.stderr).contains("future host"));
        assert!(invalid.stdout.is_empty());
        assert!(!home.path().join(".future").exists());
    }
    // The pairing flags this entry point used to take are refused by name, and a
    // refused launch still touches no state.
    for flag in ["--re-pair", "--no-qr"] {
        let invalid = desktop(home.path()).arg(flag).output().unwrap();
        assert_eq!(invalid.status.code(), Some(1), "{flag}");
        let stderr = String::from_utf8_lossy(&invalid.stderr);
        assert!(stderr.contains(flag), "{flag}: {stderr}");
        assert!(invalid.stdout.is_empty(), "{flag}");
        assert!(!home.path().join(".future").exists(), "{flag}");
    }
}

/// A launch with no display connection must refuse, not wait: the event loop
/// never reports `Ready` in a background session, so a launch that enters it
/// hangs with a blank window and a log that only says "stalled at step 'event
/// loop'". The override is what makes this reproducible on a machine that
/// *does* have a desktop (CI, a developer's Mac), so the refusal is asserted
/// from the process the user actually runs.
#[cfg(feature = "gui")]
#[test]
fn a_launch_without_a_graphical_session_refuses_instead_of_waiting_for_ready() {
    let home = tempfile::tempdir().unwrap();
    let mut child = desktop(home.path())
        .env("FUTURE_DESKTOP_SESSION", "background")
        .spawn()
        .unwrap();
    let mut stderr_pipe = child.stderr.take().expect("stderr is piped");
    // The regression this test exists for is a launch that *waits*: a process
    // that enters the event loop never exits, so the deadline is the assertion,
    // not a safety net around one.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the launch waited for Ready instead of refusing");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let mut stderr = String::new();
    std::io::Read::read_to_string(&mut stderr_pipe, &mut stderr).unwrap();
    // A script, a supervisor or a user in a shell has to see the failure.
    assert_eq!(status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("needs a graphical session"), "{stderr}");
    // The remedy, not just the symptom: somewhere else to open the window, and
    // an entry point that needs none.
    assert!(stderr.contains("screen"), "{stderr}");
    assert!(stderr.contains("future host"), "{stderr}");
    // The launch stopped at the session check, which is the assertion that it
    // never entered the wait above.
    let log = std::fs::read_to_string(home.path().join(".future/app/logs/desktop.log"))
        .expect("the host log written before the refusal");
    assert!(log.contains("'graphical session'"), "{log}");
    // The refusal also came before the window build, which is where the toolkit is
    // initialized. A build reached first is not a cosmetic difference: on Linux it
    // is what turned this launch into a GTK abort ("Failed to initialize gtk
    // backend!", exit 101) instead of the exit code and message asserted above.
    assert!(
        !log.contains("startup step 'window build' reached"),
        "{log}"
    );
    // The step itself never ran: the refusal quotes "step 'event loop'" in its
    // own wording, so the check has to name the discarded report instead.
    assert!(!log.contains("startup step 'event loop' reached"), "{log}");
    assert!(!log.contains("Agent identity"), "{log}");
}

#[cfg(not(feature = "gui"))]
#[test]
fn gui_free_build_points_to_standalone_entrypoint() {
    let home = tempfile::tempdir().unwrap();
    let result = desktop(home.path()).output().unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("future host"));
    assert!(!home.path().join(".future").exists());
}
