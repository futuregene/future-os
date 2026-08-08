//! `run` identity gate contract (G-27): `run` REQUIRES `--agent-id` (so the
//! lease mechanism engages), auto-registers unregistered ids, and keeps an
//! explicit `--anonymous` escape hatch for uncoordinated one-shot runs.
//! Deterministic — no gRPC/LLM: the identity gate runs before any agent
//! connection, so the failure path and the auto-register side effect are
//! testable without a server.

use std::process::Command;

use future_loop::console::ensure_run_identity;
use future_loop::store::Store;

fn tmp_root(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "future-loop-run-identity-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}

fn store_with_goal(root: &str, goal_id: &str) -> Store {
    let mut store = Store::open(root).expect("store opens");
    let goal = future_loop::state::Goal::new(goal_id, "identity gate", "/tmp");
    store.register(&goal).expect("goal registered");
    // `replay` needs the event ledger file too (goal init appends GoalStarted).
    store
        .append(future_loop::store::Event::GoalStarted {
            goal_id: goal_id.to_string(),
            ts: future_loop::state::now_epoch(),
        })
        .expect("goal started event");
    store
}

// ── Unit: ensure_run_identity (the whole gate, no gRPC) ───────────────────

#[test]
fn run_without_agent_id_fails_closed_with_hint() {
    let root = tmp_root("gate");
    let mut store = store_with_goal(&root, "g1");
    let err = ensure_run_identity(&mut store, "g1", None, false).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("--agent-id"),
        "error must demand --agent-id: {msg}"
    );
    assert!(
        msg.contains("agent list"),
        "error must point at `agent list`: {msg}"
    );
}

#[test]
fn anonymous_escapes_the_gate() {
    let root = tmp_root("anon");
    let mut store = store_with_goal(&root, "g1");
    let id = ensure_run_identity(&mut store, "g1", None, true).expect("anonymous allowed");
    assert_eq!(id, None, "anonymous run resolves to no identity");
}

#[test]
fn unregistered_id_is_auto_registered_once() {
    let root = tmp_root("auto");
    let mut store = store_with_goal(&root, "g1");
    let id = ensure_run_identity(&mut store, "g1", Some("worker-1"), false)
        .expect("auto-register resolves");
    assert_eq!(id.as_deref(), Some("worker-1"));

    let goal = store.replay("g1").unwrap().unwrap();
    assert!(
        goal.is_registered_agent(Some("worker-1")),
        "auto-register must persist the id"
    );

    // Second call: already registered → no duplicate event appended.
    let events_before = store.events("g1").unwrap().len();
    let _ = ensure_run_identity(&mut store, "g1", Some("worker-1"), false).unwrap();
    let events_after = store.events("g1").unwrap().len();
    assert_eq!(
        events_before, events_after,
        "re-registering the same id must not append another event"
    );
}

#[test]
fn existing_registered_id_passes_through() {
    let root = tmp_root("existing");
    let mut store = store_with_goal(&root, "g1");
    // Register alice via the real event path first.
    store
        .append(future_loop::store::Event::AgentRegistered {
            goal_id: "g1".to_string(),
            agent_id: "alice".to_string(),
            ts: future_loop::state::now_epoch(),
        })
        .unwrap();
    let id = ensure_run_identity(&mut store, "g1", Some("alice"), false).unwrap();
    assert_eq!(id.as_deref(), Some("alice"));
    let events = store.events("g1").unwrap();
    assert_eq!(
        events.len(),
        2,
        "goal_started + registration only — no extra event for an already-registered id"
    );
}

// ── Binary: fail-fast before any gRPC connection ──────────────────────────

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_future-loop")
}

fn run(root: &str, args: &[&str]) -> (String, String, i32) {
    let output = Command::new(bin())
        .env("FUTURE_LOOP_ROOT", root)
        .args(args)
        .output()
        .expect("future-loop binary runs");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

#[test]
fn cli_run_without_agent_id_errors_before_connecting() {
    let root = tmp_root("cli");
    let (_out, err, code) = run(
        &root,
        &[
            "goal",
            "init",
            "--objective",
            "gate",
            "--goal-id",
            "g1",
            "--cwd",
            "/tmp",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let (out, err, code) = run(&root, &["run", "--goal", "g1", "--max-turns", "1"]);
    assert_ne!(code, 0, "run without --agent-id must fail");
    let all = format!("{out}\n{err}");
    assert!(
        all.contains("--agent-id"),
        "error must demand --agent-id: {all}"
    );
    assert!(
        all.contains("agent list"),
        "error must point at `agent list`: {all}"
    );
}
