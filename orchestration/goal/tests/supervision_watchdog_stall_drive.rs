//! `agents/supervision.rs:315-317` — the watchdog readiness deadline in
//! `ensure_watchdog`: `child.kill()` + `child.wait()` + the bail that reports the
//! watchdog did not become ready within 3 seconds.
//!
//! The row used to be filed as unreachable-in-this-environment, on the argument
//! that the spawned watchdog either takes its lock (ready) or exits non-zero
//! (the 311-312 pair), so a pre-lock stall would need a child frozen between
//! `spawn` and `lock_file` — "and the parent holds no pid". That argument is
//! wrong twice: the parent **does** hold the `Child` (`let mut child =
//! cmd.spawn()?`, 299, which is what makes `child.kill()` at 315 possible at
//! all), and `ensure_watchdog` spawns `std::env::current_exe()`, so the stall
//! only needs the *file at that path* to be a staller when the spawn happens.
//!
//! Fixture (Unix): copy the built binary to `<tmp>/future-goal`, drive
//! `goal supervisor register`, and immediately replace the file at that path
//! with a stub (`unlink` + write). The **running** parent keeps the old inode;
//! only the *next* `exec` of that path gets the stub, so the watchdog never takes
//! its lock and the 3 s deadline is what ends the drive.
//!
//! A wrapper *script* copy does not work and is not used: `exec` makes the
//! parent's `current_exe()` the real binary, so the spawned watchdog would be the
//! real binary and the drive would register in ~1 s.
//!
//! The window is the parent's own startup (~0.7 s of CLI parsing plus store
//! open before it spawns the watchdog), so the swap delays below are all inside
//! it; the drive asserts on the deadline *message* and on the elapsed time (the
//! deadline is 3 s), and it counts how many of the delays fired rather than
//! assuming one particular delay always wins — the same "at least one hit in N
//! biased attempts, tally printed" shape this campaign uses for race fixtures.

#![cfg(unix)]

use std::io::Write;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use future_goal::state::{Goal, Todo};
use future_goal::store::{Event, Store};

const GOAL: &str = "goal_watchdog_stall_drive";

/// A live, non-terminal goal with work on it — the state `register` needs before
/// it reaches the watchdog spawn.
fn seed(root: &str, cwd: &str) {
    let mut store = Store::open(root).unwrap();
    store
        .register(&Goal::new(GOAL, "watchdog stall drive", cwd))
        .unwrap();
    store
        .append(Event::GoalStarted {
            goal_id: GOAL.into(),
            ts: 1,
        })
        .unwrap();
    store
        .append(Event::AgentRegistered {
            goal_id: GOAL.into(),
            agent_id: "w1".into(),
            workspaces: vec![],
            ts: 2,
        })
        .unwrap();
    store
        .append(Event::TodoAdded {
            goal_id: GOAL.into(),
            todo: Todo::advancement("t1", "work"),
            ts: 3,
        })
        .unwrap();
}

/// Replace the file at `path` with a stub that ignores its arguments and sleeps.
/// Returns `(old inode, new inode)` — different inodes are what prove the running
/// image survived the replacement.
fn stall_in_place(path: &Path) -> (u64, u64, bool) {
    let before = std::fs::metadata(path).map(|m| m.ino()).unwrap_or(0);
    std::fs::remove_file(path).unwrap();
    let mut stub = std::fs::File::create(path).unwrap();
    writeln!(stub, "#!/bin/sh\nexec sleep 30").unwrap();
    drop(stub);
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let after = std::fs::metadata(path).map(|m| m.ino()).unwrap_or(0);
    // The inode is not sufficient evidence: tmpfs (Linux CI's /tmp) reuses inode
    // numbers aggressively, so a completed swap can read the same number back and
    // the fixture would report "no swap landed" while the file really was
    // replaced. The content is the honest signal; the inode stays as a bonus.
    let replaced = std::fs::read_to_string(path)
        .map(|body| body.contains("exec sleep 30") && body.starts_with("#!"))
        .unwrap_or(false);
    (before, after, replaced)
}

/// One drive: start `register`, swap the binary at the copy path after
/// `delay_ms`, and report whether the 3 s deadline fired.
fn drive_after_swap(delay_ms: u64) -> (bool, Duration, String, bool) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("loop-root");
    std::fs::create_dir_all(&root).unwrap();
    let root = root.to_string_lossy().into_owned();
    let cwd = dir.path().join("cwd");
    std::fs::create_dir_all(&cwd).unwrap();
    let cwd = cwd.to_string_lossy().into_owned();
    seed(&root, &cwd);

    let copy: PathBuf = dir.path().join("future-goal");
    std::fs::copy(env!("CARGO_BIN_EXE_future-goal"), &copy).unwrap();
    std::fs::set_permissions(&copy, std::fs::Permissions::from_mode(0o755)).unwrap();

    let child = Command::new(&copy)
        .args([
            "goal",
            "supervisor",
            "register",
            "--goal",
            GOAL,
            "--session-id",
            "sup-stall",
        ])
        .env("FUTURE_GOAL_ROOT", &root)
        // The documented opt-out must be off, or `ensure_watchdog` returns early.
        .env_remove("FUTURE_GOAL_NO_DETACH")
        // Nothing here may reach an agent on this host.
        .env("FUTURE_GOAL_AGENT_ADDR", "127.0.0.1:1")
        .env("FUTURE_AGENT_SOCKET", dir.path().join("agent.sock"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the binary copy");

    std::thread::sleep(Duration::from_millis(delay_ms));
    let (before, after, replaced) = stall_in_place(&copy);

    let started = Instant::now();
    let output = child.wait_with_output().unwrap();
    let elapsed = started.elapsed() + Duration::from_millis(delay_ms);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let fired = text.contains("did not become ready within 3 seconds");
    println!(
        "DELAY {delay_ms:4} ms -> deadline_fired={fired:<5} inode {before} -> {after} \
         elapsed={elapsed:?} exit={:?} :: {}",
        output.status.code(),
        text.replace('\n', " | ")
            .chars()
            .take(160)
            .collect::<String>()
    );
    (fired, elapsed, text, replaced || before != after)
}

/// The deadline arm is reachable: a binary whose file is replaced by a staller
/// between `spawn` and the watchdog's `exec` never takes its lock, so the parent
/// kills it and bails. All the swap delays below are inside the parent's ~0.7 s
/// startup window — measured on this host, all five fire (5/5, 3.4–3.6 s each);
/// the test stops at the first delay that does, so one drive is the normal cost.
///
/// What is pinned beyond the message: the drive **failed** (no supervisor was
/// registered), the elapsed time is the real 3 s deadline rather than an early
/// error, and the swapped inode proves the running parent survived the swap (the
/// drive was not itself replaced by the stub).
// macOS only, for a measured reason. The fixture's whole premise is a WINDOW
// between the parent's spawn and the watchdog's exec that a swap can land in: on
// macOS `Command::spawn` costs tens of milliseconds, so the five delays below all
// open it (5/5, 3.4-3.6 s each). On Linux the parent reaches the watchdog's spawn
// in about a millisecond, so NO delay - including 0 ms - lands inside the window,
// and the drive reports 0/5: not a product difference, but this route to the
// deadline does not exist there. Gated rather than widened to a number nobody
// measured; the deadline arm stays exercised on every macOS run.
#[cfg(target_os = "macos")]
#[test]
fn a_watchdog_that_stalls_before_its_lock_hits_the_readiness_deadline() {
    let mut fired = 0usize;
    let mut attempted = 0usize;
    let mut swaps = 0usize;
    let mut hits = Vec::new();
    for delay_ms in [0u64, 80, 150, 250, 400] {
        attempted += 1;
        let (did_fire, elapsed, text, swapped) = drive_after_swap(delay_ms);
        if swapped {
            swaps += 1;
        }
        if did_fire {
            fired += 1;
            hits.push((delay_ms, elapsed, text));
            break; // one hit is the claim; the tally is reported below
        }
    }
    println!(
        "SWAP-WINDOW RESULT: deadline fired on {fired} of {attempted} attempted swap delays \
         ({swaps} swaps landed)"
    );
    assert!(
        swaps >= 1,
        "the fixture never replaced the file the watchdog was spawned from"
    );
    let (delay_ms, elapsed, text) = hits.first().unwrap_or_else(|| {
        panic!("no swap delay reached the 3 s readiness deadline: {fired}/{attempted} attempts")
    });
    assert!(
        *elapsed >= Duration::from_secs(3),
        "the deadline arm must cost the real 3 s budget, took {elapsed:?}"
    );
    assert!(
        text.contains("supervisor watchdog did not become ready within 3 seconds"),
        "the bail must name the deadline (supervision.rs:317): {text}"
    );
    assert!(
        !text.contains("supervisor registered"),
        "the stalled drive must not report a registration: {text}"
    );
    println!("first hit: delay {delay_ms} ms, {elapsed:?}");
}
