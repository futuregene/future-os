//! `agents/supervision.rs:281` — the detached watchdog of `ensure_watchdog` must
//! be spawned as `<binary> goal supervisor watch --goal G` **when this process is
//! the unified `future` binary**, because that is the only spelling the unified
//! CLI's group dispatch accepts. The `future-goal` standalone binary accepts both
//! spellings, so the branch is unreachable from ordinary test harnesses (whose
//! `current_exe` stem is neither `future` nor `future-goal`) and from the
//! `future-goal` binary (whose stem makes `stem == "future"` false).
//!
//! Fixture: a **copy** of the built `future-goal` binary renamed `future`, in a
//! temporary directory. The copy — never the real binary, and nothing on the
//! operator's `PATH` — is what gives the spawned CLI an executable whose
//! `current_exe()` stem is `future`, while keeping the drive inside this unit's
//! own state root.
//!
//! What is asserted, and why the argv read matters:
//! * the drive exits 0 and reports `supervisor registered` — `ensure_watchdog`
//!   ran to completion from a `future`-stem binary;
//! * a watcher is actually running for the goal (the `watch.lock` is held) and
//!   `ensure_watchdog` created its `watch.log` — the spawn step ran;
//! * (Unix) the child's own argv, read from `ps`, is
//!   `<copy> goal supervisor watch --goal G`: with line 281 removed the same
//!   drive would report `<copy> supervisor watch --goal G`, so this is the
//!   assertion that pins the line rather than merely executing it. Windows has
//!   no `ps` here, and the copy is the whole fixture there, so the Windows
//!   reading of this row rests on the two assertions above.
//!
//! Teardown is a `Drop` guard, not a trailing statement: the watcher is a
//! detached process, and a panic before the last statement must not leave one
//! running (FLAKES.md 4c is the same lesson for process-global state).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use future_goal::agents::supervision;
use future_goal::state::{now_epoch, Goal, Todo};
use future_goal::store::{Event, Store};

/// Cancels the goal and waits for the detached watcher to notice, so a failing
/// assertion cannot leak a process into the rest of the run.
struct WatcherTeardown {
    root: String,
    goal: String,
}

impl Drop for WatcherTeardown {
    fn drop(&mut self) {
        let Ok(mut store) = Store::open(&self.root) else {
            return;
        };
        if !supervision::running(&store, &self.goal) {
            return;
        }
        let _ = store.append(Event::GoalCancelled {
            goal_id: self.goal.clone(),
            reason: "test teardown".into(),
            ts: now_epoch(),
        });
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if !supervision::running(&store, &self.goal) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        // Report rather than panic: panicking in `drop` while unwinding aborts.
        eprintln!(
            "WARNING: the watchdog for {} did not release its lock after cancel",
            self.goal
        );
    }
}

/// A copy of the built binary named exactly `future` (`future.exe` on Windows,
/// where an extension-less name is not executable).
fn future_named_copy(dir: &Path) -> PathBuf {
    #[cfg(windows)]
    let name = "future.exe";
    #[cfg(not(windows))]
    let name = "future";
    let copy = dir.join(name);
    std::fs::copy(env!("CARGO_BIN_EXE_future-goal"), &copy).expect("copy the built binary");
    copy
}

/// The running watcher's command line. Selected by the *copy path* — a random
/// temp directory, so it names exactly one process — and not by the goal id,
/// which also occurs in whatever shell or editor happens to mention it.
#[cfg(unix)]
fn watcher_command_line(copy: &Path, goal: &str) -> String {
    let needle = copy.to_string_lossy().into_owned();
    let output = Command::new("ps")
        .args(["-ww", "-eo", "pid=,args="])
        .output()
        .expect("ps runs");
    let listing = String::from_utf8_lossy(&output.stdout).into_owned();
    let mut matches = listing
        .lines()
        .filter(|line| line.contains(&needle) && line.contains(goal));
    let line = matches
        .next()
        .unwrap_or_else(|| panic!("no process launched from {needle} for {goal}:\n{listing}"));
    assert!(
        matches.next().is_none(),
        "exactly one watcher process is expected for {goal}:\n{listing}"
    );
    line.split_once(char::is_whitespace)
        .map(|(_, args)| args.to_string())
        .unwrap_or_default()
}

/// `journald`-style watch log created by `ensure_watchdog` inside the state root.
fn watch_log(root: &str) -> Option<PathBuf> {
    std::fs::read_dir(Path::new(root).join("supervision"))
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.to_string_lossy().ends_with(".watch.log"))
}

#[test]
fn a_future_named_binary_spawns_its_watchdog_with_the_goal_group_verb() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("loop-root");
    std::fs::create_dir_all(&root).unwrap();
    let root = root.to_string_lossy().into_owned();
    let cwd = dir.path().join("cwd");
    std::fs::create_dir_all(&cwd).unwrap();

    // A live, non-terminal goal: the watcher it starts stays up until cancelled,
    // which is the window the argv read needs.
    let gid = "goal_future_stem_probe";
    {
        let mut store = Store::open(&root).unwrap();
        store
            .register(&Goal::new(
                gid,
                "watchdog spawn probe",
                &cwd.to_string_lossy(),
            ))
            .unwrap();
        store
            .append(Event::GoalStarted {
                goal_id: gid.into(),
                ts: 1,
            })
            .unwrap();
        store
            .append(Event::AgentRegistered {
                goal_id: gid.into(),
                agent_id: "w1".into(),
                workspaces: vec![],
                ts: 2,
            })
            .unwrap();
        store
            .append(Event::TodoAdded {
                goal_id: gid.into(),
                todo: Todo::advancement("t1", "work"),
                ts: 3,
            })
            .unwrap();
    }
    // Declared before the copy so it drops first; it owns the process cleanup.
    let _teardown = WatcherTeardown {
        root: root.clone(),
        goal: gid.to_string(),
    };
    let copy = future_named_copy(dir.path());

    let output = Command::new(&copy)
        .args([
            "goal",
            "supervisor",
            "register",
            "--goal",
            gid,
            "--session-id",
            "sup-future-stem",
        ])
        .env("FUTURE_GOAL_ROOT", &root)
        // The documented opt-out must be off, or `ensure_watchdog` returns early
        // and nothing below runs.
        .env_remove("FUTURE_GOAL_NO_DETACH")
        // Nothing here may reach a developer's real agent.
        .env("FUTURE_GOAL_AGENT_ADDR", "127.0.0.1:1")
        .env("FUTURE_AGENT_SOCKET", dir.path().join("agent.sock"))
        .output()
        .expect("the future-named copy runs");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "a future-stem binary must register its supervisor (and its watchdog): \
         {stdout}\n{stderr}"
    );
    assert!(
        stdout.contains("supervisor registered"),
        "the register command must report its own success: {stdout}"
    );

    let store = Store::open(&root).unwrap();
    assert!(
        supervision::running(&store, gid),
        "ensure_watchdog must leave a running watcher; stdout was {stdout}\n{stderr}"
    );
    assert!(
        watch_log(&root).is_some(),
        "ensure_watchdog creates the watcher's log before spawning it"
    );

    #[cfg(unix)]
    {
        let command_line = watcher_command_line(&copy, gid);
        assert!(
            command_line.contains(&format!(" goal supervisor watch --goal {gid}")),
            "the detached watcher must be invoked with the `goal` group verb — \
             that is the whole point of supervision.rs:281: {command_line}"
        );
        assert!(
            command_line.starts_with(copy.to_string_lossy().as_ref()),
            "the watcher must be this future-named copy, not another binary: {command_line}"
        );
    }

    // Cleanup is the teardown guard's job; assert here that it works, so a
    // leaked watcher cannot pass silently.
    drop(_teardown);
    let stopped = Instant::now() + Duration::from_secs(15);
    while Instant::now() < stopped {
        if !supervision::running(&store, gid) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("the detached watcher must exit once the goal is cancelled");
}
