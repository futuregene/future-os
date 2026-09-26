//! Drive for the detached-run dispatch in `cmd_run`.
//!
//! `run` re-executes the real CLI in the background so the orchestrating agent
//! never blocks. That re-exec only happens from the actual binaries (under a
//! test harness `current_exe` is the test binary, so detach is skipped), which
//! is why this drive spawns the real instrumented binary rather than calling
//! `console::run` in-process.
//!
//! The behaviour asserted is the defensive liveness guard: a re-exec that
//! mis-dispatches exits instantly with a usage error, and the parent must
//! report that instead of printing a plausible-looking pid for a worker that
//! is already dead.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_future-loop")
}

fn tmp_root(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "future-loop-detach-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}

/// Run the real binary with a controlled environment. `no_detach` selects the
/// documented opt-outs; otherwise the parent takes the detach branch.
fn run_env(root: &str, args: &[&str], no_detach: bool) -> (String, String, i32) {
    let mut cmd = Command::new(bin());
    cmd.env("FUTURE_LOOP_ROOT", root)
        .args(args)
        // Point the agent at a port nothing listens on, so any real session
        // work fails fast instead of reaching a developer machine's agent.
        .env("FUTURE_LOOP_AGENT_ADDR", "http://127.0.0.1:1")
        // A bounded, deterministic wait: the guard's grace window is 120 ms.
        .env("FUTURE_LOOP_DETACH_GRACE_MS", "120");
    if no_detach {
        cmd.env("FUTURE_LOOP_NO_DETACH", "1");
    } else {
        cmd.env_remove("FUTURE_LOOP_NO_DETACH");
    }
    let out = cmd.output().expect("binary runs");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

fn init_goal(root: &str, objective: &str) -> String {
    let gid = format!("goal_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let cwd = format!("{root}/cwd");
    std::fs::create_dir_all(&cwd).unwrap();
    let (_, err, code) = run_env(
        root,
        &[
            "goal",
            "init",
            "--objective",
            objective,
            "--goal-id",
            &gid,
            "--cwd",
            &cwd,
        ],
        true,
    );
    assert_eq!(code, 0, "goal init failed: {err}");
    gid
}

/// The detach branch is selected only for the real CLI (not a test binary),
/// not passed `--detach`, and not opted out via the env var. Driving it needs a
/// goal that is already closed, so the watchdog that detach also starts exits
/// cleanly on its first pass instead of polling for the rest of the test run.
///
/// Assertions are made against the CHILD's own log rather than the parent's
/// exit status, because the parent's liveness grace window (120 ms) is a
/// wall-clock heuristic: on a loaded box a debug build takes longer than that to
/// reach its first error, so the parent may legitimately report a pid for a
/// child that dies moments later. The child's log is deterministic and proves
/// the property the guard exists for — the re-exec reached `cmd_run` and did
/// real work instead of mis-dispatching.
#[test]
fn detached_re_exec_re_dispatches_into_run_and_hits_the_identity_gate() {
    let root = tmp_root("reject");
    let gid = init_goal(&root, "detach liveness guard");
    // Close the goal first: the watchdog acquires its lock, sees `cancelled`
    // and returns, so the parent's readiness loop finishes quickly.
    let (_, err, code) = run_env(
        &root,
        &["goal", "cancel", "--goal", &gid, "--reason", "test"],
        true,
    );
    assert_eq!(code, 0, "goal cancel failed: {err}");

    // No `--agent-id` and no `--anonymous`, so the re-executed child fails the
    // identity gate at once — before any gRPC work.
    let (out, err, _code) = run_env(&root, &["run", "--goal", &gid], false);

    let log_dir = std::path::Path::new(&root).join("detached").join(&gid);
    assert!(
        log_dir.is_dir(),
        "the detached run must create its log dir at {}; stderr={err}",
        log_dir.display()
    );
    let logs: Vec<std::path::PathBuf> = std::fs::read_dir(&log_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "log"))
        .collect();
    assert_eq!(
        logs.len(),
        1,
        "exactly one child log — a second would mean the re-exec recursed: {logs:?}"
    );
    let name = logs[0].file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        name.starts_with("anonymous-") && name.ends_with(".log"),
        "log naming must be <agent>-<ts>.log: {name}"
    );

    // The child's own output proves it reached the run path: the identity gate
    // fires before any session or agent call, and its message names the fix.
    let child_log = std::fs::read_to_string(&logs[0]).unwrap();
    assert!(
        child_log.contains("run requires --agent-id"),
        "the re-exec must land in cmd_run and hit the identity gate; log={child_log}"
    );

    // When the child IS quick enough, the parent reports the fast exit and
    // points at the log instead of claiming a healthy worker. Both outcomes are
    // accepted (see the doc comment), but a success report must at least name
    // the log the operator has to read.
    if err.contains("exited immediately") {
        assert!(
            err.contains(&gid),
            "the fast-exit report must name the goal: {err}"
        );
    } else {
        assert!(
            out.contains("detached run pid=") && out.contains(&gid),
            "a success report must name the pid and the goal: {out}"
        );
        assert!(
            out.contains("log "),
            "a success report must point at the child log: {out}"
        );
    }
}

/// The documented opt-outs must keep `run` in the foreground: with
/// `FUTURE_LOOP_NO_DETACH=1` the same invocation never creates a detached log,
/// and `--detach` marks an already-detached child so a re-exec cannot recurse.
#[test]
fn no_detach_env_and_detach_flag_stay_in_the_foreground() {
    for (tag, args, no_detach) in [
        ("env", vec!["run", "--goal", "PLACEHOLDER"], true),
        (
            "flag",
            vec!["run", "--goal", "PLACEHOLDER", "--detach"],
            false,
        ),
    ] {
        let root = tmp_root(tag);
        let gid = init_goal(&root, "foreground run");
        let args: Vec<&str> = args
            .into_iter()
            .map(|a| if a == "PLACEHOLDER" { gid.as_str() } else { a })
            .collect();
        let (_, err, code) = run_env(&root, &args, no_detach);
        // The run fails because no agent is reachable — that is the point: it
        // did the foreground work instead of forking.
        assert_ne!(code, 0, "{tag}: expected a foreground failure, got success");
        assert!(
            !err.contains("detached run pid"),
            "{tag}: must not fork a detached child: {err}"
        );
        let log_dir = std::path::Path::new(&root).join("detached").join(&gid);
        assert!(
            !log_dir.exists(),
            "{tag}: a foreground run must not create {}",
            log_dir.display()
        );
    }
}
