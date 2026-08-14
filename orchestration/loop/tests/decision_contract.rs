//! Contract tests for the decision kernel — migrated from the prototype and
//! extended with the contracts learned from the live reference (completion
//! closure intent, succession replan obligation, validated terminal).
//!
//! Deterministic: state fixtures in, typed ShouldRunPacket out. No gRPC, no
//! LLM, no money.

use std::time::{Duration, SystemTime};

use future_loop::contract::TurnMode;
use future_loop::decision::{decide, MAX_REPAIR_ATTEMPTS, MONITOR_NO_CHANGE_REPLAN_THRESHOLD};
use future_loop::state::{Goal, RunRecord, TaskClass, Todo, TodoStatus};

fn now() -> SystemTime {
    SystemTime::now()
}

fn run_record(turn: u32, todo_id: &str, state: &str) -> RunRecord {
    RunRecord {
        turn,
        todo_id: todo_id.to_string(),
        run_id: format!("run-{turn}"),
        terminal_state: state.to_string(),
        error: if state == "error" {
            Some("boom".into())
        } else {
            None
        },
        tokens_in_delta: 0,
        tokens_out_delta: 0,
        cost_delta: 0.0,
        tools: vec![],
        evidence: String::new(),
        recorded_at: 0,
        spend_source: None,
        validation: None,
    }
}

// ── Contract: scoped user gate with independent fallback work ──────────────
#[test]
fn scoped_user_gate_keeps_fallback_delivery() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::user_gate(
        "G1",
        "Approve reading the private source",
        &["T2"],
    ));
    goal.add(Todo::advancement(
        "T1",
        "Public-safe fallback, independent of G1",
    ));
    goal.add(Todo::advancement("T2", "Private gap-sync, blocked by G1").blocking(&["G1"]));

    let p = decide(&goal, now());
    assert_eq!(p.interaction_contract.mode, TurnMode::AskUser);
    assert!(p
        .interaction_contract
        .user_channel
        .question
        .unwrap()
        .contains("Approve"));
    assert_eq!(
        p.interaction_contract
            .agent_channel
            .fallback_todo
            .as_deref(),
        Some("T1"),
        "independent fallback must still be deliverable (scoped gate)"
    );
}

#[test]
fn gated_todo_is_not_runnable_while_gate_open() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::user_gate("G1", "Decide X", &["T2"]));
    goal.add(Todo::advancement("T2", "Depends on gate").blocking(&["G1"]));
    let p = decide(&goal, now());
    assert_eq!(p.interaction_contract.mode, TurnMode::AskUser);
    assert_eq!(p.interaction_contract.agent_channel.selected_todo, None);
}

// ── Contract: runnable advancement ⇒ bounded delivery ──────────────────────
#[test]
fn runnable_advancement_is_bounded_delivery() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("T1", "Do the thing"));
    let p = decide(&goal, now());
    assert_eq!(p.interaction_contract.mode, TurnMode::BoundedDelivery);
    assert_eq!(
        p.interaction_contract
            .agent_channel
            .selected_todo
            .as_deref(),
        Some("T1")
    );
}

// ── Contract: work leased to other agents is quiet wait, NOT terminal ────
// An agent whose runnable frontier is empty because every open advancement
// is leased to peers must get a quiet wait — never the validated-closure
// terminal stop, which parks the goal in a skip loop until leases expire.
#[test]
fn work_leased_to_peers_is_quiet_wait_not_terminal() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.register_agent("worker-2", vec![]);
    goal.add(Todo::advancement("T1", "Open work"));
    goal.todo_mut("T1")
        .unwrap()
        .claim("worker-1", 3600, future_loop::state::now_epoch());
    let p = future_loop::decision::decide_for(&goal, now(), Some("worker-2"));
    assert_eq!(p.interaction_contract.mode, TurnMode::WaitMonitor);
    assert!(p.reason.contains("leased to other agents"));
    // And once the lease holder completes it, the same goal is terminal.
    goal.todo_mut("T1").unwrap().complete(true, vec![]);
    let p = future_loop::decision::decide_for(&goal, now(), Some("worker-2"));
    assert_eq!(p.interaction_contract.mode, TurnMode::Terminal);
}

// ── Contract: terminal closure is NOT "open_count == 0" ────────────────────
#[test]
fn open_todos_alone_is_not_terminal() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("T1", "Still open"));
    assert_ne!(
        decide(&goal, now()).interaction_contract.mode,
        TurnMode::Terminal
    );
}

#[test]
fn closed_todos_with_acceptance_gap_is_replan_not_terminal() {
    let mut goal = Goal::new("g", "objective", "/tmp")
        .with_acceptance(vec![("A1", "result matches tolerance")]);
    goal.add(Todo::advancement("T1", "Run experiment"));
    goal.todo_mut("T1").unwrap().complete(true, vec![]);
    let p = decide(&goal, now());
    assert_eq!(p.interaction_contract.mode, TurnMode::Replan);
    assert!(p.reason.contains("acceptance gap"));
}

#[test]
fn closed_todos_open_monitor_is_not_terminal() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::monitor("M1", "Watch CI", Duration::from_secs(60)));
    goal.add(Todo::advancement("T1", "Done work"));
    goal.todo_mut("T1").unwrap().complete(true, vec![]);
    let p = decide(&goal, now());
    assert_eq!(p.interaction_contract.mode, TurnMode::WaitMonitor);
}

// ── Contract: completion closure intent (learned from live LoopX) ─────────
// A completed advancement todo must declare successor or no-follow-up;
// silent completion raises a succession replan obligation and NEVER yields
// terminal (LoopX: completed_advancement_without_successor).
#[test]
fn silent_completion_is_not_terminal() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("T1", "Work"));
    // Silent complete: no successor, no no-follow-up.
    goal.todo_mut("T1").unwrap().status = TodoStatus::Done;
    let p = decide(&goal, now());
    assert_eq!(
        p.interaction_contract.mode,
        TurnMode::Replan,
        "silent completion must raise a succession replan obligation"
    );
    assert!(p.reason.contains("closure intent"));
}

#[test]
fn completion_with_no_followup_is_terminal() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("T1", "Work"));
    goal.todo_mut("T1").unwrap().complete(true, vec![]);
    let p = decide(&goal, now());
    assert_eq!(p.interaction_contract.mode, TurnMode::Terminal);
    let tc = p.terminal_closure.expect("terminal must derive closure");
    assert_eq!(tc.kind, "no_followup");
    assert!(tc.derived);
    assert_eq!(tc.source, "validated_goal_closure");
}

#[test]
fn completion_with_successor_is_terminal_when_successor_closed() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("T1", "Slice 1"));
    goal.add(Todo::advancement("T2", "Slice 2"));
    goal.todo_mut("T1")
        .unwrap()
        .complete(false, vec!["T2".into()]);
    goal.todo_mut("T2").unwrap().complete(true, vec![]);
    let p = decide(&goal, now());
    assert_eq!(p.interaction_contract.mode, TurnMode::Terminal);
}

// ── Contract: monitor cadence / backoff ────────────────────────────────────
#[test]
fn not_due_monitor_waits_with_backoff() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::monitor(
        "M1",
        "Poll PR checks",
        Duration::from_secs(3600),
    ));
    let p = decide(&goal, now());
    assert_eq!(p.interaction_contract.mode, TurnMode::WaitMonitor);
    assert!(p.scheduler_hint.next_due_ms.is_some());
}

#[test]
fn due_monitor_allows_one_poll() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::monitor(
        "M1",
        "Poll PR checks",
        Duration::from_millis(10),
    ));
    std::thread::sleep(Duration::from_millis(50));
    let p = decide(&goal, now());
    assert_eq!(p.interaction_contract.mode, TurnMode::MonitorPoll);
}

#[test]
fn monitor_no_change_never_spends() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::monitor("M1", "Watch A", Duration::from_millis(10)));
    std::thread::sleep(Duration::from_millis(50));
    let record = run_record(1, "M1", "completed");
    future_loop::executor::writeback(&mut goal, &record, Some(false), None);
    assert_eq!(goal.todo("M1").unwrap().consecutive_no_change, 1);
    assert_eq!(
        goal.history.len(),
        0,
        "no-change monitor polls never enter the ledger"
    );
}

#[test]
fn stalled_monitor_triggers_replan() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::monitor("M1", "Watch A", Duration::from_millis(10)));
    std::thread::sleep(Duration::from_millis(50));
    let record = run_record(1, "M1", "completed");
    for _ in 0..MONITOR_NO_CHANGE_REPLAN_THRESHOLD {
        future_loop::executor::writeback(&mut goal, &record, Some(false), None);
    }
    let p = decide(&goal, now());
    assert_eq!(p.interaction_contract.mode, TurnMode::Replan);
    assert!(p.reason.contains("stalled"));
}

// ── Contract: repair budget ────────────────────────────────────────────────
#[test]
fn failed_todo_retries_then_stops() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("T1", "Flaky work"));
    let d1 = decide(&goal, now());
    assert_eq!(d1.interaction_contract.mode, TurnMode::BoundedDelivery);

    let fail = run_record(1, "T1", "error");
    future_loop::executor::writeback(&mut goal, &fail, None, None);
    let d2 = decide(&goal, now());
    assert_eq!(d2.interaction_contract.mode, TurnMode::BoundedDelivery);
    assert!(d2.reason.contains("repair attempt"));

    future_loop::executor::writeback(&mut goal, &fail, None, None);
    assert!(goal.todo("T1").unwrap().failed_attempts > MAX_REPAIR_ATTEMPTS);
    let d3 = decide(&goal, now());
    assert_eq!(d3.interaction_contract.mode, TurnMode::Replan);
}

// ── Contract: gate resolution unlocks the dependent todo ───────────────────
#[test]
fn resolving_gate_unblocks_dependent_todo() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::user_gate("G1", "Approve X", &["T2"]));
    goal.add(Todo::advancement("T2", "Depends on gate").blocking(&["G1"]));
    goal.todo_mut("G1").unwrap().status = TodoStatus::Done;
    goal.todo_mut("G1").unwrap().decision = Some("approved".into());
    let p = decide(&goal, now());
    assert_eq!(p.interaction_contract.mode, TurnMode::BoundedDelivery);
    assert_eq!(
        p.interaction_contract
            .agent_channel
            .selected_todo
            .as_deref(),
        Some("T2")
    );
}

// ── Contract: task classes are orthogonal to status ────────────────────────
#[test]
fn task_classes_are_orthogonal_to_status() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::monitor("M1", "watch", Duration::from_secs(60)));
    assert_eq!(goal.todo("M1").unwrap().class, TaskClass::Monitor);
    assert_eq!(goal.todo("M1").unwrap().status, TodoStatus::Open);
}

// ── Contract: full packet shape ────────────────────────────────────────────
#[test]
fn packet_has_three_channels_and_auxiliary_contracts() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("T1", "Work"));
    let p = decide(&goal, now());
    assert_eq!(
        p.interaction_contract.schema_version,
        "future_loop_interaction_contract_v0"
    );
    assert!(p.interaction_contract.agent_channel.must_attempt);
    assert!(!p.interaction_contract.agent_channel.quiet_noop_allowed);
    assert!(p.interaction_contract.cli_channel.spend_after_validation);
    assert!(!p.interaction_contract.cli_channel.spend_allowed_now);
    assert_eq!(
        p.work_lane_contract.obligation,
        "advance_one_bounded_segment"
    );
    assert_eq!(p.work_lane_contract.reason_codes, vec!["open_agent_todo"]);
    assert!(p.execution_obligation.must_attempt_work);
    assert!(p.automation_liveness.keep_active);
    assert!(!p.automation_liveness.pause_allowed);
    assert_eq!(p.scheduler_hint.schema_version, "scheduler_hint_v0");
    assert!(p.quota.allowed_slots > 0);
    // Packet must serialize to JSON (the transport contract).
    let json = serde_json::to_string(&p).expect("packet serializes");
    assert!(json.contains("interaction_contract"));
    assert!(json.contains("agent_channel"));
}
