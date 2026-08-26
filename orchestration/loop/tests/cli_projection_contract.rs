//! P1 contract tests — G-9 turn envelope + CLI projection: the envelope
//! carries instruction + context + evidence + decision summary, and the
//! text projections render quota / usage / scheduler state for hosts.

use future_loop::cli_projection::{
    render_cadence_plan, render_quota_projection, render_scheduler_state, render_usage_summary,
};
use future_loop::decision::decide;
use future_loop::state::{Goal, Todo};
use future_loop::turn_envelope::{
    compose_turn_envelope, compose_turn_message, TURN_ENVELOPE_SCHEMA_VERSION,
};

// ── Turn envelope: instruction + context + evidence + decision summary ─────
#[test]
fn turn_envelope_carries_decision_context() {
    let mut goal = Goal::new("g1", "Ship the platform", "/tmp");
    goal.add(Todo::advancement("T1", "Implement the adapter"));
    let packet = decide(&goal, std::time::SystemTime::now());
    let msg = compose_turn_envelope(&goal, goal.todo("T1").unwrap(), Some(&packet), None);
    assert!(msg.contains(TURN_ENVELOPE_SCHEMA_VERSION));
    assert!(msg.contains("decision: run"), "decision banner");
    assert!(msg.contains("objective: Ship the platform"), "goal context");
    assert!(
        msg.contains("execution: cadence="),
        "execution profile context"
    );
    assert!(
        msg.contains("TODO T1: Implement the adapter"),
        "instruction"
    );
    assert!(msg.contains("Complete the todo and report what you did and observed."));
    assert!(msg.contains("--no-follow-up"), "completion contract footer");
    assert!(
        msg.contains("arbitration:"),
        "scheduler arbitration in envelope"
    );
}

#[test]
fn turn_envelope_includes_prior_evidence_and_gate_decisions() {
    let mut goal = Goal::new("g1", "o", "/tmp");
    goal.add(Todo::user_gate("G1", "Approve", &["T2"]));
    goal.todo_mut("G1").unwrap().decision = Some("approved".into());
    goal.todo_mut("G1").unwrap().status = future_loop::state::TodoStatus::Done;
    goal.add(Todo::advancement("T2", "Blocked work").blocking(&["G1"]));
    let prev = future_loop::state::RunRecord {
        turn: 1,
        todo_id: "T0".into(),
        run_id: "run-1".into(),
        terminal_state: "completed".into(),
        error: None,
        tokens_in_delta: 0,
        tokens_out_delta: 0,
        cost_delta: 0.0,
        tools: vec![],
        evidence: "prior evidence payload".into(),
        recorded_at: 0,
        spend_source: Some("run".into()),
        validation: None,
        failure_kind: None,
        truncation: None,
    };
    let msg = compose_turn_message(&goal, goal.todo("T2").unwrap(), Some(&prev));
    assert!(msg.contains("Resolved gate decision(s): G1: approved"));
    assert!(msg.contains("Evidence from the previous turn (todo T0)"));
    assert!(msg.contains("prior evidence payload"));
}

// ── CLI projection: quota output includes breakdown + usage summary ────────
#[test]
fn quota_projection_renders_usage_summary_and_spend_breakdown() {
    let mut goal = Goal::new("g1", "o", "/tmp");
    goal.add(Todo::advancement("T1", "Work"));
    let packet = decide(&goal, std::time::SystemTime::now());
    let breakdown = future_loop::quota::usage_summary::breakdown(&goal.history);
    let usage = future_loop::quota::usage_summary::build_usage_summary(
        "g1",
        &goal.history,
        future_loop::state::now_epoch(),
    );
    let proj = render_quota_projection(&packet, Some(&breakdown), None);
    // P1 acceptance: the quota command output contains the usage summary.
    let usage_text = render_usage_summary(&usage);
    assert!(proj.contains("allowed=1440"));
    assert!(proj.contains("spent=0"));
    assert!(proj.contains("arbitration:"));
    assert!(usage_text.contains("usage summary"));
    assert!(usage_text.contains("goal g1"));
}

#[test]
fn quota_projection_annotates_replan_stall() {
    let mut goal = Goal::new("g1", "o", "/tmp");
    let mut t = Todo::advancement("T1", "done without intent");
    t.status = future_loop::state::TodoStatus::Done;
    goal.add(t);
    let packet = decide(&goal, std::time::SystemTime::now());
    assert_eq!(
        packet.interaction_contract.mode,
        future_loop::contract::TurnMode::Replan
    );
    let stall = future_loop::quota::stall_repair::detect_stall(&goal);
    let proj = render_quota_projection(&packet, None, stall.as_ref());
    assert!(proj.contains("stall: succession_obligation"));
    assert!(proj.contains("replan hint:"));
}

// ── CLI projection: scheduler state + cadence plan ─────────────────────────
#[test]
fn scheduler_state_projection_renders_progression_and_failures() {
    use future_loop::scheduler::state::*;
    let identity = identity_signature("g1", "a", CODEX_APP_SURFACE);
    let state = build_scheduler_state(
        "g1",
        "a",
        CODEX_APP_SURFACE,
        CODEX_APP_STATEFUL_BACKOFF_STATE_KEY,
        &reset_token("tick", &identity, "FREQ=MINUTELY;INTERVAL=15"),
        &identity,
        1,
        MONITOR_WAIT_PROGRESSION_MINUTES.to_vec(),
        "FREQ=MINUTELY;INTERVAL=30",
        1_784_000_000,
        vec![HostUpdateFailure {
            schema_version: SCHEDULER_HOST_UPDATE_FAILURE_SCHEMA_VERSION.into(),
            target_rrule: "FREQ=MINUTELY;INTERVAL=30".into(),
            observed_host_rrule: "FREQ=MINUTELY;INTERVAL=1440".into(),
            failure_kind: "host_stale_rrule".into(),
            failed_at: "2026-08-05T12:00:00+00:00".into(),
            failure_count: 2,
        }],
    )
    .unwrap();
    let text = render_scheduler_state(&state);
    assert!(text.contains("progression"));
    assert!(text.contains("FREQ=MINUTELY;INTERVAL=30"));
    assert!(text.contains("host failures  : 1 retained"));
    assert!(text.contains("host_stale_rrule"));
    let plan = render_cadence_plan("monitor_backoff", &[15, 30, 60], 1);
    assert!(plan.contains("15m → 30m → 1h"));
}
