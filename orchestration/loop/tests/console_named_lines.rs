//! `console.rs`'s remaining named lines, batch 1.
//!
//! Selected from `cargo llvm-cov report --text` (the per-line export), not from a
//! percentage. These assertions are on the operator-visible STDOUT, because for
//! projections like `status` and `frontier` the printed output IS the
//! deliverable - so the CLI runs as a subprocess and its stdout is captured.
//!
//! All state lives in a tempdir created by this file (never a user path).

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_future-loop")
}

fn tmp_root(tag: &str) -> (String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join(format!("root-{tag}"));
    std::fs::create_dir_all(&root).unwrap();
    (root.to_string_lossy().into_owned(), dir)
}

/// Run the CLI with an explicit state root; return (stdout, stderr, code).
fn run(root: &str, args: &[&str]) -> (String, String, i32) {
    let out = Command::new(bin())
        .env("FUTURE_LOOP_ROOT", root)
        // Never reach a developer's real agent from a test.
        .env("FUTURE_LOOP_AGENT_ADDR", "http://127.0.0.1:1")
        .args(args)
        .output()
        .expect("binary runs");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

fn ok(root: &str, args: &[&str]) -> String {
    let (stdout, stderr, code) = run(root, args);
    assert_eq!(code, 0, "cli {args:?} failed: {stderr}");
    stdout
}

fn init_goal(root: &str, objective: &str) -> String {
    let gid = format!("goal_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let cwd = format!("{root}/cwd");
    std::fs::create_dir_all(&cwd).unwrap();
    ok(
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
    );
    gid
}

fn add_todo(root: &str, goal: &str, text: &str) -> String {
    ok(root, &["todo", "add", "--goal", goal, "--text", text]);
    let store = future_loop::store::Store::open(root).unwrap();
    store
        .replay(goal)
        .unwrap()
        .unwrap()
        .todos
        .iter()
        .find(|t| t.text.contains(text))
        .unwrap()
        .id
        .clone()
}

/// `print_goal_status` renders the recent no-progress breaches, newest first,
/// capped at three. Without the event the whole block is skipped, so appending
/// one is what covers it.
#[test]
fn status_renders_recent_no_progress_breaches_newest_first() {
    let (root, _dir) = tmp_root("np");
    let gid = init_goal(&root, "no-progress render");
    let mut store = future_loop::store::Store::open(&root).unwrap();
    // Four breaches: only the newest three may be shown.
    for i in 1..=4u64 {
        store
            .append(future_loop::store::Event::TurnNoProgress {
                goal_id: gid.clone(),
                todo_id: format!("todo_np{i}"),
                agent_id: Some(format!("w{i}")),
                idle_secs: i * 10,
                tool_calls_total: i as u32,
                ts: i,
            })
            .unwrap();
    }
    drop(store);

    let out = ok(&root, &["status", "--goal", &gid]);
    assert!(out.contains("no-progress:"), "{out}");
    for shown in ["todo_np4", "todo_np3", "todo_np2"] {
        assert!(
            out.contains(shown),
            "the newest three must render ({shown}): {out}"
        );
    }
    assert!(
        !out.contains("todo_np1"),
        "only the newest three may be shown: {out}"
    );

    // The anonymous fallback: an event with no agent id renders `anonymous`.
    let solo = init_goal(&root, "anonymous breach");
    let mut store = future_loop::store::Store::open(&root).unwrap();
    store
        .append(future_loop::store::Event::TurnNoProgress {
            goal_id: solo.clone(),
            todo_id: "todo_solo".into(),
            agent_id: None,
            idle_secs: 99,
            tool_calls_total: 0,
            ts: 1,
        })
        .unwrap();
    drop(store);
    let out = ok(&root, &["status", "--goal", &solo]);
    assert!(
        out.contains("agent=anonymous"),
        "an event without an agent must render the fallback: {out}"
    );
}

/// `cmd_frontier` lists each todo's owner and flags an owner that is not a
/// registered agent, so a typo'd `--owner` is visible rather than silently
/// accepted (the ids are case-sensitive).
#[test]
fn frontier_renders_owners_and_flags_unregistered_ones() {
    let (root, _dir) = tmp_root("frontier");
    let gid = init_goal(&root, "frontier owners");
    ok(
        &root,
        &[
            "agent",
            "register",
            "--goal",
            &gid,
            "--agent-id",
            "known_agent",
        ],
    );
    let known = add_todo(&root, &gid, "owned by a registered agent");
    let ghost = add_todo(&root, &gid, "owned by an unregistered one");
    ok(
        &root,
        &[
            "todo",
            "update",
            "--goal",
            &gid,
            "--todo-id",
            &known,
            "--owner",
            "known_agent",
        ],
    );
    ok(
        &root,
        &[
            "todo",
            "update",
            "--goal",
            &gid,
            "--todo-id",
            &ghost,
            "--owner",
            "ghost_agent",
        ],
    );

    let out = ok(&root, &["frontier", "show", "--goal", &gid]);
    assert!(out.contains("owner=known_agent"), "{out}");
    assert!(out.contains("owner=ghost_agent"), "{out}");
    assert!(
        out.contains("not registered; IDs are case-sensitive"),
        "an unregistered owner must be flagged: {out}"
    );
    let registered_line = out
        .lines()
        .find(|l| l.contains("owner=known_agent"))
        .expect("the registered owner must be rendered");
    assert!(
        !registered_line.contains("not registered"),
        "a registered owner must not be flagged: {registered_line}"
    );
}

/// `stop_goal_workers` with `--agent-id` signals ONLY the named worker, and -
/// when the abort cannot be delivered - it must KEEP that worker's leases.
///
/// The contract here is asymmetric on purpose: a lease is the claim "I may still
/// be mid-turn". If the agent is unreachable the stop rides the ledger signal and
/// the TTL alone, because dropping the claim while a turn might still be running
/// would let a peer start the same todo concurrently.
#[test]
fn worker_stop_signals_only_the_named_worker_and_keeps_leases_when_agent_is_unreachable() {
    let (root, _dir) = tmp_root("stop-agent");
    let gid = init_goal(&root, "stop by agent");
    ok(
        &root,
        &["agent", "register", "--goal", &gid, "--agent-id", "w1"],
    );
    ok(
        &root,
        &["agent", "register", "--goal", &gid, "--agent-id", "w2"],
    );
    let todo = add_todo(&root, &gid, "lease held by w1");
    ok(
        &root,
        &[
            "lease",
            "claim",
            "--goal",
            &gid,
            "--todo-id",
            &todo,
            "--agent-id",
            "w1",
            "--lease-secs",
            "600",
        ],
    );

    // Seed a live run header per worker: that is what the session scan finds.
    let runs = std::path::Path::new(&root).join("runs");
    std::fs::create_dir_all(&runs).unwrap();
    for who in ["w1", "w2"] {
        let header = serde_json::json!({
            "type": "run_header",
            "wall_ts": 1_700_000_000u64,
            "run_id": format!("run-{who}"),
            "session_id": format!("sess-{who}"),
            "agent_id": who,
            "todo_id": todo,
            "goal_id": gid,
        });
        std::fs::write(
            runs.join(format!("run_{who}.live.jsonl")),
            format!("{header}\n"),
        )
        .unwrap();
    }

    let listed = ok(&root, &["worker", "list", "--goal", &gid]);
    assert!(listed.contains("w1") && listed.contains("w2"), "{listed}");

    // Stopping by agent id appends the signal for that worker only (the abort
    // itself cannot be delivered here - the subprocess pins an unreachable addr).
    let out = ok(
        &root,
        &["worker", "stop", "--goal", &gid, "--agent-id", "w1"],
    );
    assert!(
        out.contains("stop sent to worker w1"),
        "only the named worker may be signalled: {out}"
    );
    assert!(
        out.contains("agent unreachable"),
        "an undeliverable abort must be reported, not swallowed: {out}"
    );
    assert!(
        out.contains("leases keep their TTL"),
        "the operator must be told the claims survive: {out}"
    );

    let store = future_loop::store::Store::open(&root).unwrap();
    let after = store.replay(&gid).unwrap().unwrap();
    assert_eq!(
        after.todo(&todo).unwrap().claimed_by.as_deref(),
        Some("w1"),
        "an undeliverable stop must not drop the claim of a possibly-running turn"
    );
    let stops: Vec<Option<String>> = store
        .events(&gid)
        .unwrap()
        .into_iter()
        .filter_map(|e| match e.event {
            future_loop::store::Event::WorkerStopped { agent_id, .. } => Some(agent_id),
            _ => None,
        })
        .collect();
    assert_eq!(stops, vec![Some("w1".to_string())], "{stops:?}");
}

/// The complementary branch: when there is NO live session to abort, nothing can
/// still be mid-turn, so the claim the agent still holds only keeps peers out and
/// must be released (the TTL could be hours).
#[test]
fn worker_stop_with_no_live_session_releases_the_stale_lease() {
    let (root, _dir) = tmp_root("stop-idle");
    let gid = init_goal(&root, "stop an idle worker");
    let todo = add_todo(&root, &gid, "lease left over from a finished turn");
    // A SECOND todo held by a DIFFERENT worker: releasing must skip it, or stopping
    // one idle worker would silently free another worker's claim.
    let other = add_todo(&root, &gid, "lease held by a worker still on the job");
    // A lease with NO run header and NO ledger binding: the worker went idle.
    ok(
        &root,
        &[
            "lease",
            "claim",
            "--goal",
            &gid,
            "--todo-id",
            &todo,
            "--agent-id",
            "idle_w1",
            "--lease-secs",
            "600",
        ],
    );
    ok(
        &root,
        &[
            "lease",
            "claim",
            "--goal",
            &gid,
            "--todo-id",
            &other,
            "--agent-id",
            "busy_w2",
            "--lease-secs",
            "600",
        ],
    );
    let listed = ok(&root, &["worker", "list", "--goal", &gid]);
    assert!(listed.contains(&format!("workers for {gid}")), "{listed}");
    assert!(
        !listed.contains("idle_w1"),
        "a lease holder with no session is not a live worker: {listed}"
    );

    let out = ok(
        &root,
        &["worker", "stop", "--goal", &gid, "--agent-id", "idle_w1"],
    );
    assert!(out.contains("nothing to stop"), "{out}");
    assert!(
        out.contains("no live worker held it"),
        "the stale claim must be released and named: {out}"
    );
    assert!(
        out.contains(&todo),
        "the released todo must be named: {out}"
    );
    assert!(
        !out.contains(&other),
        "another worker's live claim must NOT be touched: {out}"
    );

    let store = future_loop::store::Store::open(&root).unwrap();
    let after = store.replay(&gid).unwrap().unwrap();
    assert!(
        after.todo(&todo).unwrap().claimed_by.is_none(),
        "a claim whose worker is idle blocks peers and must be dropped: {:?}",
        after.todo(&todo)
    );
    assert_eq!(
        after.todo(&other).unwrap().claimed_by.as_deref(),
        Some("busy_w2"),
        "an unrelated worker keeps its claim: {:?}",
        after.todo(&other)
    );

    // A ghost agent is a clean no-op (nothing to stop), not an error.
    let (_, _, code) = run(
        &root,
        &["worker", "stop", "--goal", &gid, "--agent-id", "nobody"],
    );
    assert_eq!(code, 0, "stopping a non-live agent must not fail");
}

/// Superseding a todo that a worker currently HOLDS must stop that worker: its
/// in-flight turn is wasted work, and a late writeback must not fight the
/// supersede (the ledger already refuses to resurrect it).
#[test]
fn superseding_a_claimed_todo_stops_its_holder() {
    let (root, _dir) = tmp_root("supersede-held");
    let gid = init_goal(&root, "supersede a held todo");
    ok(
        &root,
        &["agent", "register", "--goal", &gid, "--agent-id", "holder"],
    );
    let todo = add_todo(&root, &gid, "held work");
    ok(
        &root,
        &[
            "lease",
            "claim",
            "--goal",
            &gid,
            "--todo-id",
            &todo,
            "--agent-id",
            "holder",
            "--lease-secs",
            "600",
        ],
    );
    // A bound session is what makes `stop_goal_workers` find a target.
    let runs = std::path::Path::new(&root).join("runs");
    std::fs::create_dir_all(&runs).unwrap();
    let header = serde_json::json!({
        "type": "run_header",
        "wall_ts": 1_700_000_000u64,
        "run_id": "run-h",
        "session_id": "sess-h",
        "agent_id": "holder",
        "todo_id": todo,
        "goal_id": gid,
    });
    std::fs::write(runs.join("run_h.live.jsonl"), format!("{header}\n")).unwrap();

    let out = ok(
        &root,
        &[
            "todo",
            "supersede",
            "--goal",
            &gid,
            "--todo-id",
            &todo,
            "--reason",
            "obsoleted",
        ],
    );
    assert!(
        out.contains("signalled to stop"),
        "the holder must be told its work was superseded: {out}"
    );

    let store = future_loop::store::Store::open(&root).unwrap();
    let after = store.replay(&gid).unwrap().unwrap();
    assert_eq!(
        after.todo(&todo).unwrap().status,
        future_loop::state::TodoStatus::Superseded
    );
    let stops = store
        .events(&gid)
        .unwrap()
        .into_iter()
        .filter(|e| matches!(e.event, future_loop::store::Event::WorkerStopped { .. }))
        .count();
    assert_eq!(stops, 1, "exactly one stop signal");
}
