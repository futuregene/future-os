//! P1-5 contract tests: reward_memory phase 1 —
//!   ① ingestion: validator receipts (run path) and delivery-outcome
//!      resolutions (`delivery record`) land in the ledger as
//!      `RewardSignalRecorded` events; `reward-memory record` scores
//!      evidence manually.
//!   ② scoped_feedback: `reward-memory query` projects the signals back
//!      out, filtered by goal (implicit) / agent / todo / source scope,
//!      with a deterministic aggregate summary.
//!
//! These exercise the real CLI entry (`console::run`) against an isolated
//! `FUTURE_LOOP_ROOT`, plus direct ledger reads for content assertions.

use future_loop::capabilities::reward_memory as rm;
use future_loop::console;
use future_loop::store::{Event, Store};

fn with_root<F: FnOnce(&str)>(tag: &str, f: F) {
    // FUTURE_LOOP_ROOT is process-global; tests run in parallel, so
    // serialize all CLI tests behind one mutex (each still gets its own
    // isolated root dir).
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("future-loop-reward-{tag}-{}", uuid_like()));
    std::fs::create_dir_all(&dir).unwrap();
    let root = dir.join(".future/loop");
    std::fs::create_dir_all(&root).unwrap();
    std::env::set_var("FUTURE_LOOP_ROOT", root.to_str().unwrap());
    f(root.to_str().unwrap());
}

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    format!(
        "{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}

fn cli(args: &[&str]) -> Result<(), String> {
    console::run(
        "future-loop",
        args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
    )
    .map_err(|e| format!("{e:#}"))
}

/// Set up goal `g1` with one advancement todo; return its todo id.
fn setup_goal_with_todo(text: &str) -> String {
    cli(&[
        "goal",
        "init",
        "--objective",
        "reward memory test",
        "--cwd",
        "/tmp",
        "--goal-id",
        "g1",
    ])
    .unwrap();
    cli(&["todo", "add", "--goal", "g1", "--text", text]).unwrap();
    let store = Store::open(&std::env::var("FUTURE_LOOP_ROOT").unwrap()).unwrap();
    let g = store.replay("g1").unwrap().unwrap();
    g.todos
        .iter()
        .find(|t| t.text.contains(text))
        .map(|t| t.id.clone())
        .unwrap()
}

fn signals(root: &str) -> Vec<rm::RewardSignal> {
    let store = Store::open(root).unwrap();
    rm::collect_signals(&store.events("g1").unwrap(), &rm::RewardScope::default())
}

// ── ① delivery resolutions auto-ingest reward signals; the pending
//    `delivered` state ingests nothing. ────────────────────────────────────
#[test]
fn delivery_resolution_ingests_reward_signal() {
    with_root("delivery-ingest", |root| {
        let todo_id = setup_goal_with_todo("ship the widget");
        cli(&[
            "todo",
            "complete",
            "--goal",
            "g1",
            "--todo-id",
            &todo_id,
            "--no-follow-up",
        ])
        .unwrap();
        // Completing records the PENDING delivery — no reward signal yet.
        assert!(
            signals(root).is_empty(),
            "a pending delivery is not a reward signal"
        );

        // verified → score 1.0.
        cli(&[
            "delivery",
            "record",
            "--goal",
            "g1",
            "--todo-id",
            &todo_id,
            "--outcome",
            "verified",
            "--note",
            "confirmed in production",
        ])
        .unwrap();
        let got = signals(root);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].source, rm::SOURCE_DELIVERY_OUTCOME);
        assert_eq!(got[0].signal, "verified");
        assert_eq!(got[0].score, Some(1.0));
        assert_eq!(got[0].note.as_deref(), Some("confirmed in production"));
        assert_eq!(got[0].todo_id, todo_id);

        // A second todo cycles failed → re-delivered → rework; only the two
        // resolutions ingest (re-delivery is pending again).
        let todo2 = setup_goal_with_todo("second widget");
        cli(&[
            "todo",
            "complete",
            "--goal",
            "g1",
            "--todo-id",
            &todo2,
            "--no-follow-up",
        ])
        .unwrap();
        for outcome in ["failed", "delivered", "rework"] {
            cli(&[
                "delivery",
                "record",
                "--goal",
                "g1",
                "--todo-id",
                &todo2,
                "--outcome",
                outcome,
            ])
            .unwrap();
        }
        let got = signals(root);
        assert_eq!(got.len(), 3);
        assert_eq!(got[1].signal, "failed");
        assert_eq!(got[1].score, Some(0.0));
        assert_eq!(got[1].seq, 1);
        assert_eq!(got[2].signal, "rework");
        assert_eq!(got[2].score, Some(0.5));
        assert_eq!(got[2].seq, 2);

        // The ledger stays conflict-free with the new events in it.
        let report = Store::open(root).unwrap().verify("g1").unwrap();
        assert!(report.ok, "ledger conflicts: {:?}", report.conflicts);
    });
}

// ── ① manual evidence scoring via reward-memory record. ──────────────────
#[test]
fn record_scores_evidence_into_the_ledger() {
    with_root("record", |root| {
        let todo_id = setup_goal_with_todo("evidence candidate");
        cli(&[
            "reward-memory",
            "record",
            "--goal",
            "g1",
            "--todo-id",
            &todo_id,
            "--score",
            "0.8",
            "--note",
            "thorough repro attached",
            "--agent-id",
            "agent-7",
        ])
        .unwrap();
        let got = signals(root);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].source, rm::SOURCE_EVIDENCE);
        assert_eq!(got[0].signal, "scored");
        assert_eq!(got[0].score, Some(0.8));
        assert_eq!(got[0].agent_id.as_deref(), Some("agent-7"));

        // Bad scores / unknown targets fail closed and record nothing.
        for bad in ["1.5", "-0.5", "abc"] {
            assert!(cli(&[
                "reward-memory",
                "record",
                "--goal",
                "g1",
                "--todo-id",
                &todo_id,
                "--score",
                bad,
            ])
            .is_err());
        }
        assert!(cli(&[
            "reward-memory",
            "record",
            "--goal",
            "g1",
            "--todo-id",
            "todo_nope",
            "--score",
            "0.5",
        ])
        .is_err());
        assert!(cli(&[
            "reward-memory",
            "record",
            "--goal",
            "g1",
            "--todo-id",
            &todo_id,
            "--score",
            "0.5",
            "--source",
            "bogus",
        ])
        .is_err());
        assert_eq!(signals(root).len(), 1);
    });
}

// ── ② scoped_feedback: query filters by agent / todo / source scope. ──────
#[test]
fn query_projects_scoped_feedback() {
    with_root("query", |root| {
        let todo_id = setup_goal_with_todo("scoped work");
        // Seed signals from two sources and two agents.
        cli(&[
            "reward-memory",
            "record",
            "--goal",
            "g1",
            "--todo-id",
            &todo_id,
            "--score",
            "0.9",
            "--agent-id",
            "agent-a",
        ])
        .unwrap();
        cli(&[
            "reward-memory",
            "record",
            "--goal",
            "g1",
            "--todo-id",
            &todo_id,
            "--score",
            "0.3",
            "--agent-id",
            "agent-b",
        ])
        .unwrap();
        cli(&[
            "todo",
            "complete",
            "--goal",
            "g1",
            "--todo-id",
            &todo_id,
            "--no-follow-up",
        ])
        .unwrap();
        cli(&[
            "delivery",
            "record",
            "--goal",
            "g1",
            "--todo-id",
            &todo_id,
            "--outcome",
            "verified",
        ])
        .unwrap();

        // Unscoped: everything, aggregated deterministically.
        let store = Store::open(root).unwrap();
        let events = store.events("g1").unwrap();
        let all = rm::collect_signals(&events, &rm::RewardScope::default());
        assert_eq!(all.len(), 3);
        let summary = rm::summarize(&all);
        assert_eq!(summary.total, 3);
        assert_eq!(summary.by_source.get(rm::SOURCE_EVIDENCE), Some(&2));
        assert_eq!(summary.by_source.get(rm::SOURCE_DELIVERY_OUTCOME), Some(&1));
        assert_eq!(summary.scored, 3);
        let avg = summary.avg_score.unwrap();
        assert!((avg - (0.9 + 0.3 + 1.0) / 3.0).abs() < 1e-9, "avg={avg}");

        // Agent scope.
        let scoped = rm::collect_signals(
            &events,
            &rm::RewardScope {
                agent_id: Some("agent-a"),
                ..Default::default()
            },
        );
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].score, Some(0.9));
        // Source scope.
        let scoped = rm::collect_signals(
            &events,
            &rm::RewardScope {
                source: Some(rm::SOURCE_DELIVERY_OUTCOME),
                ..Default::default()
            },
        );
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].signal, "verified");

        // The CLI surface accepts every scope flag (text + JSON).
        cli(&["reward-memory", "query", "--goal", "g1"]).unwrap();
        cli(&[
            "reward-memory",
            "query",
            "--goal",
            "g1",
            "--agent-id",
            "agent-a",
            "--format",
            "json",
        ])
        .unwrap();
        cli(&[
            "reward-memory",
            "query",
            "--goal",
            "g1",
            "--todo-id",
            &todo_id,
            "--source",
            "evidence",
        ])
        .unwrap();
        // Unknown goal / source / flag fail closed.
        assert!(cli(&["reward-memory", "query", "--goal", "nope"]).is_err());
        assert!(cli(&["reward-memory", "query", "--goal", "g1", "--source", "nope"]).is_err());
        assert!(cli(&["reward-memory", "query", "--goal", "g1", "--bogus"]).is_err());
        assert!(cli(&["reward-memory", "frobnicate", "--goal", "g1"]).is_err());
    });
}

// ── read surface: reward signals appear in the per-todo event history. ────
#[test]
fn reward_signals_appear_in_todo_event_history() {
    with_root("todo-event", |root| {
        let todo_id = setup_goal_with_todo("historied work");
        cli(&[
            "reward-memory",
            "record",
            "--goal",
            "g1",
            "--todo-id",
            &todo_id,
            "--score",
            "0.5",
        ])
        .unwrap();
        cli(&["todo-event", "--goal", "g1", "--todo-id", &todo_id]).unwrap();
        // The raw ledger line carries the flattened event for the todo.
        let store = Store::open(root).unwrap();
        let lines = store.raw_ledger_lines("g1").unwrap();
        let reward_lines: Vec<&String> = lines
            .iter()
            .filter(|l| l.contains("reward_signal_recorded"))
            .collect();
        assert_eq!(reward_lines.len(), 1);
        assert!(reward_lines[0].contains(&todo_id));
        // The event round-trips through the typed ledger reader.
        let events = store.events("g1").unwrap();
        assert!(events
            .iter()
            .any(|se| matches!(&se.event, Event::RewardSignalRecorded { todo_id: t, .. } if *t == todo_id)));
    });
}
