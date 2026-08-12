//! P1-4 contract tests: decision_context deep implementation —
//!   ① assembler + providers: `decision-context assemble` composes the
//!      packet from the builtin providers (run history / outcome streak /
//!      quota status) over current goal state.
//!   ② outcome feedback: `decision-context feedback` settles an audited
//!      receipt against a persisted decision summary (fail closed without
//!      an anchor) and writes decisive outcomes back into reward memory
//!      under the `decision_outcome` source; `decision-context outcomes`
//!      is the read model.
//!   ③ replay integration: `replay record` captures the assembled context
//!      in the case (the record→run mismatch fix).
//!
//! These exercise the real CLI entry (`console::run`) against an isolated
//! `FUTURE_LOOP_ROOT`, plus direct ledger reads for content assertions.

use future_loop::capabilities::decision_context::assembler::assemble_decision_context;
use future_loop::capabilities::decision_context::outcome_feedback as feedback;
use future_loop::capabilities::decision_context::packets::DECISION_CONTEXT_PACKET_SCHEMA_VERSION;
use future_loop::capabilities::reward_memory as rm;
use future_loop::console;
use future_loop::store::{Event, Store};

fn with_root<F: FnOnce(&str)>(tag: &str, f: F) {
    // FUTURE_LOOP_ROOT is process-global; tests run in parallel, so
    // serialize all CLI tests behind one mutex (each still gets its own
    // isolated root dir).
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!(
        "future-loop-decision-context-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let root = dir.join(".future/loop");
    std::fs::create_dir_all(&root).unwrap();
    std::env::set_var("FUTURE_LOOP_ROOT", root.to_str().unwrap());
    f(root.to_str().unwrap());
}

fn cli(args: &[&str]) -> Result<(), String> {
    console::run(
        "future-loop",
        args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
    )
    .map_err(|e| format!("{e:#}"))
}

/// Set up goal `g1` with one advancement todo `T1`.
fn setup_goal() {
    cli(&[
        "goal",
        "init",
        "--objective",
        "decision context test",
        "--cwd",
        "/tmp",
        "--goal-id",
        "g1",
    ])
    .unwrap();
    cli(&["todo", "add", "--goal", "g1", "--text", "do work"]).unwrap();
}

/// Persist a decision summary anchoring `turn` (the run path's P1-1
/// writeback, staged directly: a full agent run is out of scope here).
fn anchor_decision(root: &str, turn: u32) {
    let store = Store::open(root).unwrap();
    let goal = store.replay("g1").unwrap().unwrap();
    let packet = future_loop::decision::decide(&goal, std::time::SystemTime::now());
    let summary =
        future_loop::quota::decision_summary::DecisionSummary::from_packet(&packet, None, turn);
    let mut store = Store::open(root).unwrap();
    store
        .append(Event::DecisionSummaryRecorded {
            goal_id: "g1".to_string(),
            summary,
            ts: 1_000,
        })
        .unwrap();
}

// ── ① assembler read model ───────────────────────────────────────────────

#[test]
fn assemble_reflects_goal_state_and_runs_via_cli() {
    with_root("assemble", |root| {
        setup_goal();
        // One material run (tools + evidence) lands in the history.
        let store = Store::open(root).unwrap();
        let goal = store.replay("g1").unwrap().unwrap();
        let todo_id = goal.todos[0].id.clone();
        drop(store);
        let store = Store::open(root).unwrap();
        store
            .append_run(
                "g1",
                &future_loop::state::RunRecord {
                    turn: 1,
                    todo_id,
                    run_id: "r1".to_string(),
                    validation: None,
                    terminal_state: "completed".to_string(),
                    error: None,
                    tokens_in_delta: 0,
                    tokens_out_delta: 0,
                    cost_delta: 0.0,
                    tools: vec!["shell".to_string()],
                    evidence: "artifact".to_string(),
                    recorded_at: 900,
                    spend_source: None,
                },
            )
            .unwrap();
        // Surface streak just under the floor: not breached.
        let goal = store.replay("g1").unwrap().unwrap();
        let packet = assemble_decision_context(&goal);
        assert_eq!(
            packet.schema_version,
            DECISION_CONTEXT_PACKET_SCHEMA_VERSION
        );
        assert_eq!(
            packet.providers,
            vec![
                "run_history".to_string(),
                "outcome_streak".to_string(),
                "quota_status".to_string()
            ]
        );
        assert_eq!(packet.run_history.run_count, 1);
        assert_eq!(packet.run_history.material_runs, 1);
        assert_eq!(packet.run_history.recent_terminal_states, vec!["completed"]);
        assert_eq!(packet.quota.spent_slots, 1);
        // CLI surfaces assemble/outcomes read-only without error.
        cli(&["decision-context", "assemble", "--goal", "g1"]).unwrap();
        cli(&[
            "decision-context",
            "assemble",
            "--goal",
            "g1",
            "--format",
            "json",
        ])
        .unwrap();
        cli(&["decision-context", "outcomes", "--goal", "g1"]).unwrap();
    });
}

// ── ② outcome feedback writeback ─────────────────────────────────────────

#[test]
fn feedback_fails_closed_without_a_decision_anchor() {
    with_root("no-anchor", |_root| {
        setup_goal();
        let err = cli(&[
            "decision-context",
            "feedback",
            "--goal",
            "g1",
            "--turn",
            "7",
            "--status",
            "verified",
        ])
        .unwrap_err();
        assert!(
            err.contains("no persisted decision summary"),
            "unexpected error: {err}"
        );
    });
}

#[test]
fn feedback_settles_receipt_and_ingests_reward_signal() {
    with_root("settle", |root| {
        setup_goal();
        anchor_decision(root, 3);
        cli(&[
            "decision-context",
            "feedback",
            "--goal",
            "g1",
            "--turn",
            "3",
            "--status",
            "verified",
            "--note",
            "held up",
        ])
        .unwrap();
        let store = Store::open(root).unwrap();
        let events = store.events("g1").unwrap();
        // Receipt settled + anchored to turn-3 with the decision payload.
        let receipt = feedback::outcome_for(&events, "turn-3").expect("receipt recorded");
        assert_eq!(receipt.verification_status, "verified");
        assert_eq!(receipt.accepted_decision, "normal_run");
        assert_eq!(receipt.reason_code, "runnable_todo");
        assert_eq!(receipt.seq, 1);
        assert!(receipt.settled_at.is_some());
        assert_eq!(receipt.note.as_deref(), Some("held up"));
        // Reward-memory writeback: verified = 1.0 under decision_outcome.
        let signals = rm::collect_signals(&events, &rm::RewardScope::default());
        let signal = signals
            .iter()
            .find(|s| s.source == rm::SOURCE_DECISION_OUTCOME)
            .expect("decision_outcome signal ingested");
        assert_eq!(signal.signal, "verified");
        assert_eq!(signal.score, Some(1.0));
        assert!(
            signal.note.as_deref().unwrap_or("").contains("turn-3"),
            "signal note links the receipt: {:?}",
            signal.note
        );
        // The outcomes read model lists it (CLI + json).
        cli(&["decision-context", "outcomes", "--goal", "g1"]).unwrap();
        cli(&[
            "decision-context",
            "outcomes",
            "--goal",
            "g1",
            "--format",
            "json",
        ])
        .unwrap();
    });
}

#[test]
fn feedback_refuted_and_inconclusive_signals() {
    with_root("signals", |root| {
        setup_goal();
        anchor_decision(root, 1);
        anchor_decision(root, 2);
        cli(&[
            "decision-context",
            "feedback",
            "--goal",
            "g1",
            "--turn",
            "1",
            "--status",
            "refuted",
        ])
        .unwrap();
        cli(&[
            "decision-context",
            "feedback",
            "--goal",
            "g1",
            "--turn",
            "2",
            "--status",
            "inconclusive",
        ])
        .unwrap();
        let store = Store::open(root).unwrap();
        let events = store.events("g1").unwrap();
        let receipts = feedback::decision_outcomes(&events);
        assert_eq!(receipts.len(), 2);
        assert_eq!(receipts[0].verification_status, "refuted");
        assert_eq!(receipts[1].verification_status, "inconclusive");
        let signals = rm::collect_signals(&events, &rm::RewardScope::default());
        let by_signal: std::collections::BTreeMap<_, _> = signals
            .iter()
            .filter(|s| s.source == rm::SOURCE_DECISION_OUTCOME)
            .map(|s| (s.signal.as_str(), s.score))
            .collect();
        assert_eq!(by_signal.get("refuted"), Some(&Some(0.0)));
        assert_eq!(by_signal.get("inconclusive"), Some(&None));
        // Scoped query surfaces the source.
        cli(&["decision-context", "outcomes", "--goal", "g1"]).unwrap();
        cli(&[
            "reward-memory",
            "query",
            "--goal",
            "g1",
            "--source",
            "decision_outcome",
        ])
        .unwrap();
    });
}

#[test]
fn feedback_rejects_unknown_status_and_repeat_settles_get_fresh_seq() {
    with_root("seq", |root| {
        setup_goal();
        anchor_decision(root, 5);
        let err = cli(&[
            "decision-context",
            "feedback",
            "--goal",
            "g1",
            "--turn",
            "5",
            "--status",
            "bogus",
        ])
        .unwrap_err();
        assert!(err.contains("--status"), "unexpected error: {err}");
        // Two settles against the same decision both land (per-decision seq
        // is the G-3 content-id dedupe anchor).
        cli(&[
            "decision-context",
            "feedback",
            "--goal",
            "g1",
            "--turn",
            "5",
            "--status",
            "verified",
        ])
        .unwrap();
        cli(&[
            "decision-context",
            "feedback",
            "--goal",
            "g1",
            "--turn",
            "5",
            "--status",
            "refuted",
        ])
        .unwrap();
        let store = Store::open(root).unwrap();
        let events = store.events("g1").unwrap();
        let receipts = feedback::decision_outcomes(&events);
        assert_eq!(receipts.len(), 2, "both settles must land");
        assert_eq!(receipts[0].seq, 1);
        assert_eq!(receipts[1].seq, 2);
        assert_ne!(receipts[0].receipt_id, receipts[1].receipt_id);
        assert_eq!(feedback::next_seq(&events, "turn-5"), 3);
    });
}

// ── ③ replay integration: the case carries the assembled context ─────────

#[test]
fn replay_record_captures_decision_context() {
    with_root("replay", |root| {
        setup_goal();
        let out = std::path::Path::new(root).join("replay.json");
        cli(&[
            "replay",
            "record",
            "--goal",
            "g1",
            "--case-id",
            "case-1",
            "--out",
            out.to_str().unwrap(),
        ])
        .unwrap();
        let replay = future_loop::replay::decision_replay::DecisionReplay::load(&out).unwrap();
        assert_eq!(replay.cases.len(), 1);
        let case = &replay.cases[0];
        let context = case
            .decision_context
            .as_ref()
            .expect("recorded case must carry the assembled context");
        assert_eq!(
            context.schema_version,
            DECISION_CONTEXT_PACKET_SCHEMA_VERSION
        );
        assert_eq!(context.goal_id, "g1");
        // The recorded case replays exactly (context applied on rebuild).
        let comparison =
            future_loop::replay::decision_replay::replay_public_safe_decision_case(case).unwrap();
        assert!(comparison.matched, "{:?}", comparison.mismatches);
        // And the whole file passes the CLI replay gate.
        cli(&["replay", "run", "--case", out.to_str().unwrap()]).unwrap();
    });
}
