//! Contract tests for the G-2/G-11 scheduler arbitration layer — the 9
//! dispositions derived from the final interaction contract, the
//! CONSISTENCY_REPAIR fail-closed path, and the observe-only → enforcement
//! rollout switch. Mirrors LoopX
//! `tests/control_plane/test_scheduler_interaction_arbitration.py`.
//!
//! Deterministic: typed InteractionContract / decided packets in,
//! SchedulerArbitration out. No gRPC, no LLM.

use std::time::{Duration, SystemTime};

use future_loop::contract::{AgentChannel, CliChannel, InteractionContract, TurnMode, UserChannel};
use future_loop::decision::arbitration::{
    apply_arbitration, build_scheduler_arbitration, classify_disposition, SchedulerDisposition,
    SCHEDULER_ARBITRATION_SCHEMA_VERSION,
};
use future_loop::decision::decide;
use future_loop::state::{Goal, Todo};

fn now() -> SystemTime {
    SystemTime::now()
}

fn contract(
    mode: TurnMode,
    user_required: bool,
    must_attempt: bool,
    delivery_allowed: bool,
    quiet_noop_allowed: bool,
) -> InteractionContract {
    InteractionContract {
        schema_version: "loopx_interaction_contract_v0".to_string(),
        mode,
        user_channel: UserChannel {
            action_required: user_required,
            notify: "DONT_NOTIFY".to_string(),
            question: None,
            todo_ids: vec![],
        },
        agent_channel: AgentChannel {
            must_attempt,
            delivery_allowed,
            quiet_noop_allowed,
            primary_action: None,
            selected_todo: None,
            fallback_todo: None,
        },
        cli_channel: CliChannel {
            next_cli_actions: vec![],
            spend_allowed_now: false,
            spend_after_validation: true,
            spend_policy: "spend once after validated writeback".to_string(),
        },
    }
}

// ── The 9 dispositions, value-aligned with LoopX ───────────────────────────
#[test]
fn disposition_enum_values_align_with_loopx() {
    let pairs = [
        (SchedulerDisposition::TerminalStop, "terminal_stop"),
        (
            SchedulerDisposition::AgentMonitorOnlyWait,
            "agent_monitor_only_wait",
        ),
        (SchedulerDisposition::ActiveWork, "active_work"),
        (SchedulerDisposition::AgentScopeWait, "agent_scope_wait"),
        (
            SchedulerDisposition::ConsistencyRepair,
            "consistency_repair",
        ),
        (SchedulerDisposition::HumanGate, "human_gate"),
        (SchedulerDisposition::MonitorWait, "monitor_wait"),
        (SchedulerDisposition::QuietWait, "quiet_wait"),
        (SchedulerDisposition::UnchangedWait, "unchanged_wait"),
    ];
    for (disposition, expected) in pairs {
        assert_eq!(disposition.as_str(), expected);
        assert_eq!(
            serde_json::to_value(disposition).unwrap(),
            serde_json::json!(expected),
            "serialized disposition must match LoopX string"
        );
    }
}

#[test]
fn schema_version_is_scheduler_arbitration_v0() {
    assert_eq!(
        SCHEDULER_ARBITRATION_SCHEMA_VERSION,
        "scheduler_arbitration_v0"
    );
    let arbitration = build_scheduler_arbitration(
        &contract(TurnMode::BoundedDelivery, false, true, true, false),
        &[],
    );
    assert!(arbitration.ok());
    // The record itself is not schema-versioned in LoopX; the repair payload is.
    assert!(arbitration.consistency_error().is_none());
}

// ── Classifier matrix: all 9 dispositions reachable (LoopX classify) ───────
#[test]
fn classifier_covers_all_nine_dispositions() {
    // terminal_no_followup → TERMINAL_STOP (reason_code = mode)
    let (d, r) = classify_disposition("terminal_no_followup", false, false, true, &[]);
    assert_eq!(d, SchedulerDisposition::TerminalStop);
    assert_eq!(r, "terminal_no_followup");

    // agent_monitor_only → AGENT_MONITOR_ONLY_WAIT (reason_code = mode)
    let (d, r) = classify_disposition("agent_monitor_only", false, false, true, &[]);
    assert_eq!(d, SchedulerDisposition::AgentMonitorOnlyWait);
    assert_eq!(r, "agent_monitor_only");

    // user_required && !must_attempt → HUMAN_GATE
    let (d, r) = classify_disposition("user_gate", true, false, false, &[]);
    assert_eq!(d, SchedulerDisposition::HumanGate);
    assert_eq!(r, "interaction_blocking_user_gate");

    // monitor_quiet_skip → MONITOR_WAIT
    let (d, r) = classify_disposition("monitor_quiet_skip", false, false, true, &[]);
    assert_eq!(d, SchedulerDisposition::MonitorWait);
    assert_eq!(r, "interaction_monitor_quiet_wait");

    // successor_replan_required && must_attempt → ACTIVE_WORK
    let (d, r) = classify_disposition("successor_replan_required", false, true, false, &[]);
    assert_eq!(d, SchedulerDisposition::ActiveWork);
    assert_eq!(r, "interaction_successor_replan_required");

    // mode in agent_scope_frontier_actions → AGENT_SCOPE_WAIT
    let (d, r) = classify_disposition(
        "bounded_delivery",
        false,
        true,
        false,
        &["bounded_delivery"],
    );
    assert_eq!(d, SchedulerDisposition::AgentScopeWait);
    assert_eq!(r, "interaction_agent_scope_wait");

    // mapped_noop_if_unchanged → UNCHANGED_WAIT
    let (d, r) = classify_disposition("mapped_noop_if_unchanged", false, false, true, &[]);
    assert_eq!(d, SchedulerDisposition::UnchangedWait);
    assert_eq!(r, "interaction_unchanged_wait");

    // must_attempt → ACTIVE_WORK
    let (d, r) = classify_disposition("bounded_delivery", false, true, false, &[]);
    assert_eq!(d, SchedulerDisposition::ActiveWork);
    assert_eq!(r, "interaction_agent_attempt_required");

    // quiet_noop_allowed → QUIET_WAIT
    let (d, r) = classify_disposition("blocked_wait", false, false, true, &[]);
    assert_eq!(d, SchedulerDisposition::QuietWait);
    assert_eq!(r, "interaction_quiet_noop_allowed");

    // fallback → QUIET_WAIT (interaction_delivery_not_allowed)
    let (d, r) = classify_disposition("skip", false, false, false, &[]);
    assert_eq!(d, SchedulerDisposition::QuietWait);
    assert_eq!(r, "interaction_delivery_not_allowed");
}

// ── build_scheduler_arbitration: disposition + reason_code + mode ──────────
#[test]
fn blocking_gate_is_human_gate() {
    let arbitration =
        build_scheduler_arbitration(&contract(TurnMode::AskUser, true, false, false, false), &[]);
    assert!(arbitration.ok());
    assert_eq!(arbitration.disposition, SchedulerDisposition::HumanGate);
    assert_eq!(arbitration.reason_code, "interaction_blocking_user_gate");
    assert_eq!(arbitration.mode, "user_gate");
}

#[test]
fn nonblocking_notice_with_work_is_active_work() {
    let arbitration = build_scheduler_arbitration(
        &contract(TurnMode::BoundedDelivery, true, true, true, false),
        &[],
    );
    assert!(arbitration.ok());
    assert_eq!(arbitration.disposition, SchedulerDisposition::ActiveWork);
    assert_eq!(
        arbitration.reason_code,
        "interaction_agent_attempt_required"
    );
    assert_eq!(arbitration.mode, "bounded_delivery");
}

#[test]
fn successor_replan_is_active_work() {
    let arbitration =
        build_scheduler_arbitration(&contract(TurnMode::Replan, false, true, false, false), &[]);
    assert!(arbitration.ok());
    assert_eq!(arbitration.disposition, SchedulerDisposition::ActiveWork);
    assert_eq!(
        arbitration.reason_code,
        "interaction_successor_replan_required"
    );
    assert_eq!(arbitration.mode, "successor_replan_required");
}

#[test]
fn terminal_no_followup_is_terminal_stop() {
    let arbitration = build_scheduler_arbitration(
        &contract(TurnMode::Terminal, false, false, false, true),
        &[],
    );
    assert!(arbitration.ok());
    assert_eq!(arbitration.disposition, SchedulerDisposition::TerminalStop);
    assert_eq!(arbitration.reason_code, "terminal_no_followup");
    assert_eq!(arbitration.mode, "terminal_no_followup");
}

#[test]
fn monitor_quiet_skip_is_monitor_wait() {
    let arbitration = build_scheduler_arbitration(
        &contract(TurnMode::WaitMonitor, false, false, false, true),
        &[],
    );
    assert!(arbitration.ok());
    assert_eq!(arbitration.disposition, SchedulerDisposition::MonitorWait);
    assert_eq!(arbitration.reason_code, "interaction_monitor_quiet_wait");
    assert_eq!(arbitration.mode, "monitor_quiet_skip");
}

// ── Structural contradictions fail closed to CONSISTENCY_REPAIR ────────────
#[test]
fn terminal_contract_with_open_action_fails_closed() {
    let arbitration = build_scheduler_arbitration(
        &contract(TurnMode::Terminal, true, false, false, false),
        &[],
    );
    assert!(!arbitration.ok());
    assert_eq!(
        arbitration.disposition,
        SchedulerDisposition::ConsistencyRepair
    );
    assert_eq!(
        arbitration.reason_code,
        "scheduler_interaction_contract_inconsistent"
    );
    assert!(arbitration
        .errors
        .contains(&"interaction_contract.terminal_conflicts_with_open_action".to_string()));
}

#[test]
fn delivery_without_attempt_fails_closed() {
    let arbitration = build_scheduler_arbitration(
        &contract(TurnMode::BoundedDelivery, false, false, true, false),
        &[],
    );
    assert!(!arbitration.ok());
    assert_eq!(
        arbitration.disposition,
        SchedulerDisposition::ConsistencyRepair
    );
    assert!(arbitration
        .errors
        .contains(&"interaction_contract.delivery_without_attempt".to_string()));
}

#[test]
fn quiet_noop_conflict_fails_closed() {
    let arbitration = build_scheduler_arbitration(
        &contract(TurnMode::BoundedDelivery, true, true, true, true),
        &[],
    );
    assert!(!arbitration.ok());
    assert_eq!(
        arbitration.disposition,
        SchedulerDisposition::ConsistencyRepair
    );
    assert!(arbitration
        .errors
        .contains(&"interaction_contract.quiet_noop_conflicts_with_required_action".to_string()));
}

#[test]
fn schema_version_mismatch_fails_closed() {
    let mut c = contract(TurnMode::BoundedDelivery, false, true, true, false);
    c.schema_version = "loopx_interaction_contract_v1".to_string();
    let arbitration = build_scheduler_arbitration(&c, &[]);
    assert!(!arbitration.ok());
    assert_eq!(
        arbitration.disposition,
        SchedulerDisposition::ConsistencyRepair
    );
    assert!(arbitration
        .errors
        .contains(&"interaction_contract.schema_version_mismatch".to_string()));
}

// ── consistency_error() repair payload ─────────────────────────────────────
#[test]
fn consistency_error_has_repair_action_and_schema_version() {
    let arbitration = build_scheduler_arbitration(
        &contract(TurnMode::BoundedDelivery, false, false, true, false),
        &[],
    );
    let err = arbitration
        .consistency_error()
        .expect("inconsistent contract must expose error");
    assert_eq!(err["schema_version"], "scheduler_arbitration_v0");
    assert_eq!(
        err["reason_code"],
        "scheduler_interaction_contract_inconsistent"
    );
    assert_eq!(err["mode"], "bounded_delivery");
    assert!(err["errors"].is_array());
    assert!(err["repair_action"]
        .as_str()
        .unwrap()
        .starts_with("rebuild interaction_contract"));
}

// ── Observe-only integration: record on the packet, never block ────────────
#[test]
fn observe_only_records_disposition_without_blocking() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("T1", "Work"));
    let p = decide(&goal, now());
    let arb = p
        .scheduler_arbitration
        .as_ref()
        .expect("packet carries arbitration record");
    assert!(arb.ok());
    assert_eq!(arb.disposition, SchedulerDisposition::ActiveWork);
    assert!(p.should_run, "observe-only must not block delivery");
    assert_eq!(p.decision, "run");
    assert!(p.ok);
}

#[test]
fn observe_only_terminal_packet_is_terminal_stop() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("T1", "Work"));
    goal.todo_mut("T1").unwrap().complete(true, vec![]);
    let p = decide(&goal, now());
    let arb = p
        .scheduler_arbitration
        .as_ref()
        .expect("packet carries arbitration record");
    assert!(arb.ok());
    assert_eq!(arb.disposition, SchedulerDisposition::TerminalStop);
    assert_eq!(arb.reason_code, "terminal_no_followup");
    assert!(!p.should_run, "terminal stays a skip decision");
}

#[test]
fn observe_only_gate_packet_is_human_gate() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::user_gate("G1", "Decide X", &["T2"]));
    goal.add(Todo::advancement("T2", "Depends on gate").blocking(&["G1"]));
    let p = decide(&goal, now());
    let arb = p
        .scheduler_arbitration
        .as_ref()
        .expect("packet carries arbitration record");
    assert!(arb.ok());
    assert_eq!(arb.disposition, SchedulerDisposition::HumanGate);
    assert_eq!(arb.reason_code, "interaction_blocking_user_gate");
    assert_eq!(p.interaction_contract.mode, TurnMode::AskUser);
}

#[test]
fn observe_only_monitor_wait_is_monitor_wait() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::monitor(
        "M1",
        "Poll PR checks",
        Duration::from_secs(3600),
    ));
    let p = decide(&goal, now());
    let arb = p
        .scheduler_arbitration
        .as_ref()
        .expect("packet carries arbitration record");
    assert!(arb.ok());
    assert_eq!(arb.disposition, SchedulerDisposition::MonitorWait);
    assert_eq!(arb.reason_code, "interaction_monitor_quiet_wait");
}

// ── Enforcement: fail closed on CONSISTENCY_REPAIR ─────────────────────────
#[test]
fn enforcement_fails_closed_on_inconsistent_contract() {
    // A packet that would otherwise deliver — corrupt its final contract the
    // way a projection bug could (delivery without attempt).
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("T1", "Work"));
    let mut p = decide(&goal, now());
    p.interaction_contract.agent_channel.must_attempt = false;
    p.interaction_contract.agent_channel.delivery_allowed = true;

    // Observe-only: record the repair, keep the packet as decided.
    let before = p.should_run;
    apply_arbitration(&mut p, false);
    let arb = p.scheduler_arbitration.as_ref().unwrap();
    assert_eq!(arb.disposition, SchedulerDisposition::ConsistencyRepair);
    assert_eq!(
        p.should_run, before,
        "observe-only never rewrites the decision"
    );

    // Enforcement: fail closed to the repair cadence.
    apply_arbitration(&mut p, true);
    let arb = p.scheduler_arbitration.as_ref().unwrap();
    assert_eq!(arb.disposition, SchedulerDisposition::ConsistencyRepair);
    assert!(!p.ok);
    assert!(!p.should_run);
    assert_eq!(p.decision, "consistency_repair");
    assert_eq!(p.effective_action, "consistency_repair");
    assert_eq!(
        p.scheduler_hint.action,
        "repair_interaction_contract_projection"
    );
    assert_eq!(p.scheduler_hint.cadence_class, "control_plane_repair");
}

#[test]
fn enforcement_never_blocks_a_consistent_packet() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("T1", "Work"));
    let mut p = decide(&goal, now());
    apply_arbitration(&mut p, true);
    assert!(p.ok, "consistent contract must pass even under enforcement");
    assert!(p.should_run);
    assert_eq!(p.scheduler_hint.cadence_class, "bounded_segment");
}

// ── Serialization: the record travels on the JSON packet ───────────────────
#[test]
fn arbitration_record_serializes_on_packet() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("T1", "Work"));
    let p = decide(&goal, now());
    let json = serde_json::to_string(&p).expect("packet serializes");
    assert!(json.contains("scheduler_arbitration"));
    assert!(json.contains("active_work"));
    assert!(json.contains("interaction_agent_attempt_required"));
}
