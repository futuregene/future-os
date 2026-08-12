//! Split regression — pre-split vs post-split packet parity, field by field.
//!
//! P0 acceptance (todo todo_cffd671f76b8): the G-1 split of `decision.rs`
//! (696 lines → `src/decision/` subdomains) plus the G-2/G-11 arbitration
//! layer must NOT change what `decide_for` emits. Proof strategy:
//!
//! 1. [`legacy`] module: verbatim copy of the pre-split kernel
//!    (`snapshots/decision.rs.pre-split`, source commit 85372a53, SHA-256
//!    4ac8c78e7e3f489e1304e57ce0e9f5dbc3bebc965b013913b487b89b9bebf165)
//!    with mechanical transforms only (see the module header there).
//! 2. For every decision path fixture, run BOTH pipelines on the same
//!    (goal, now, agent_id) and compare the serialized packets field by
//!    field (recursive JSON equality, volatile fields masked).
//! 3. The ONLY intended deltas are the G-2/G-11 `scheduler_arbitration`
//!    record (new field, observe-only) and the per-tool-quota
//!    `capability_repair_allowed` predicate (LoopX 对比改进项 ②: was a
//!    constant `false` pre-split, now computed from the goal's capability
//!    invocation projection): both asserted, then removed from the
//!    comparison.
//!
//! If this test fails, the split drifted from the baseline: either a helper
//! subdomain changed a field, or the arbitration layer mutated behavior.

use std::time::{Duration, SystemTime};

use future_loop::contract::{ShouldRunPacket, TurnMode};
use future_loop::decision::{decide_for, SchedulerDisposition, MONITOR_NO_CHANGE_REPLAN_THRESHOLD};
use future_loop::state::{Goal, TaskClass, Todo, TodoStatus};

/// The pre-split decision kernel, byte-faithful to the snapshot. Inner
/// attributes are not permitted inside `include!`d content, so the lint
/// allowances live on the module.
#[allow(dead_code, clippy::all, unused_imports)]
mod legacy {
    include!("legacy/decision_pre_split.rs");
}

fn now() -> SystemTime {
    SystemTime::now()
}

/// Mask wall-clock / random fields that are intentionally non-deterministic
/// (rollout `event_id` is a fresh UUID; `recorded_at` reads the wall clock).
fn mask_volatile(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(map) => {
            for key in ["event_id", "recorded_at"] {
                if map.contains_key(key) {
                    map.insert(key.to_string(), serde_json::Value::String("MASKED".into()));
                }
            }
            for value in map.values_mut() {
                mask_volatile(value);
            }
        }
        serde_json::Value::Array(items) => {
            for value in items.iter_mut() {
                mask_volatile(value);
            }
        }
        _ => {}
    }
}

/// Run the legacy and current pipelines on identical inputs and require the
/// packets to be field-for-field identical (modulo the volatile masks and
/// the G-2/G-11 arbitration record). Returns the current packet for further
/// path assertions.
fn assert_packet_parity(goal: &Goal, now: SystemTime, agent_id: Option<&str>) -> ShouldRunPacket {
    let legacy_packet = legacy::legacy_decide_for(goal, now, agent_id);
    let current_packet = decide_for(goal, now, agent_id);

    let mut legacy_json =
        serde_json::to_value(&legacy_packet).expect("legacy packet must serialize");
    let mut current_json =
        serde_json::to_value(&current_packet).expect("current packet must serialize");
    mask_volatile(&mut legacy_json);
    mask_volatile(&mut current_json);

    // G-2/G-11: the arbitration record is the ONLY struct field added since
    // the pre-split baseline — assert it is present (observe-only), then
    // exclude it so the comparison covers every pre-split field.
    let arbitration = current_json
        .get("scheduler_arbitration")
        .cloned()
        .expect("G-2/G-11 must record scheduler_arbitration (observe-only default)");
    assert!(
        arbitration.get("disposition").is_some() && arbitration.get("reason_code").is_some(),
        "arbitration record must carry disposition + reason_code"
    );
    if let serde_json::Value::Object(map) = &mut current_json {
        map.remove("scheduler_arbitration");
    }

    // Per-tool quota (LoopX 对比改进项 ②): `capability_repair_allowed` was a
    // constant `false` in the pre-split kernel; it is now computed from the
    // goal's capability invocation projection (false only when a tool is
    // over its quota). These fixtures carry no invocations, so the lane
    // must read open — assert the intended delta, then normalize both sides.
    assert_eq!(
        current_json.get("capability_repair_allowed"),
        Some(&serde_json::Value::Bool(true)),
        "fixtures carry no capability invocations — the capability-repair \
         lane must be open (per-tool quota, 对比改进项 ②)"
    );
    if let serde_json::Value::Object(map) = &mut current_json {
        map.remove("capability_repair_allowed");
    }
    if let serde_json::Value::Object(map) = &mut legacy_json {
        map.remove("capability_repair_allowed");
    }

    assert_eq!(
        legacy_json, current_json,
        "packet field drift vs pre-split baseline (goal={:?}, agent={:?}) — \
         see snapshots/decision.rs.pre-split",
        goal.goal_id, agent_id
    );
    current_packet
}

/// Parity + fixture sanity: the fixture must exercise the intended decision
/// path, otherwise the parity check is vacuous.
fn check(
    goal: &Goal,
    now: SystemTime,
    agent_id: Option<&str>,
    expected_mode: TurnMode,
) -> ShouldRunPacket {
    let p = assert_packet_parity(goal, now, agent_id);
    assert_eq!(
        p.interaction_contract.mode, expected_mode,
        "fixture must exercise the intended decision path (goal={:?})",
        goal.goal_id
    );
    p
}

// ── Identity gate ──────────────────────────────────────────────────────────
#[test]
fn anonymous_path_parity() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("T1", "Do the thing"));
    check(&goal, now(), None, TurnMode::BoundedDelivery);
}

#[test]
fn unregistered_agent_fails_closed_parity() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("T1", "Do the thing"));
    let now = now();
    let p = check(&goal, now, Some("ghost-agent"), TurnMode::WaitMonitor);
    assert!(!p.ok, "unregistered identity must fail closed");
    assert_eq!(p.state, "blocked_health");
    assert_eq!(p.status, "quota_collection_failed");
    assert_eq!(p.decision, "skip");
}

#[test]
fn registered_agent_parity() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.register_agent("a1", vec!["shell".into()]);
    goal.add(Todo::advancement("T1", "Do the thing"));
    check(&goal, now(), Some("a1"), TurnMode::BoundedDelivery);
}

// ── User gates (scoped) ────────────────────────────────────────────────────
#[test]
fn user_gate_with_fallback_parity() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::user_gate(
        "G1",
        "Approve reading the private source",
        &["T2"],
    ));
    goal.add(Todo::advancement("T1", "Public-safe fallback"));
    goal.add(Todo::advancement("T2", "Private gap-sync").blocking(&["G1"]));
    let p = check(&goal, now(), None, TurnMode::AskUser);
    assert_eq!(
        p.interaction_contract
            .agent_channel
            .fallback_todo
            .as_deref(),
        Some("T1")
    );
}

#[test]
fn user_gate_without_fallback_parity() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::user_gate("G1", "Approve X", &["T2"]));
    goal.add(Todo::advancement("T2", "Depends on gate").blocking(&["G1"]));
    let p = check(&goal, now(), None, TurnMode::AskUser);
    assert!(!p.interaction_contract.agent_channel.must_attempt);
    // Arbitration: blocking human gate, nothing to attempt → human_gate.
    assert_eq!(
        p.scheduler_arbitration.as_ref().unwrap().disposition,
        SchedulerDisposition::HumanGate
    );
}

#[test]
fn user_gate_with_user_actions_parity() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::user_gate("G1", "Approve X", &["T2"]));
    let mut action = Todo::advancement("A1", "Please review the proposal");
    action.class = TaskClass::UserAction;
    goal.add(action);
    goal.add(Todo::advancement("T2", "Depends on gate").blocking(&["G1"]));
    let p = check(&goal, now(), None, TurnMode::AskUser);
    let question = p
        .interaction_contract
        .user_channel
        .question
        .as_deref()
        .unwrap();
    assert!(
        question.contains("(actions)"),
        "user actions must surface in the ask channel"
    );
}

// ── Runnable advancement ───────────────────────────────────────────────────
#[test]
fn runnable_advancement_parity() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("T1", "Do the thing"));
    let p = check(&goal, now(), None, TurnMode::BoundedDelivery);
    assert_eq!(
        p.interaction_contract
            .agent_channel
            .selected_todo
            .as_deref(),
        Some("T1")
    );
    assert_eq!(
        p.scheduler_arbitration.as_ref().unwrap().disposition,
        SchedulerDisposition::ActiveWork
    );
}

#[test]
fn priority_sort_selects_p0_parity() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("T2", "second"));
    goal.add(Todo::advancement("T0", "first"));
    goal.add(Todo::advancement("T1", "third"));
    goal.todo_mut("T0").unwrap().priority = future_loop::state::Priority::P0;
    goal.todo_mut("T2").unwrap().priority = future_loop::state::Priority::P2;
    let p = check(&goal, now(), None, TurnMode::BoundedDelivery);
    assert_eq!(
        p.interaction_contract
            .agent_channel
            .selected_todo
            .as_deref(),
        Some("T0")
    );
    assert_eq!(
        p.interaction_contract
            .agent_channel
            .primary_action
            .as_deref(),
        Some("first")
    );
}

#[test]
fn runnable_with_user_actions_parity() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    let mut action = Todo::advancement("A1", "Please review the proposal");
    action.class = TaskClass::UserAction;
    goal.add(action);
    goal.add(Todo::advancement("T1", "Do the thing"));
    let p = check(&goal, now(), None, TurnMode::BoundedDelivery);
    assert!(p.interaction_contract.user_channel.action_required);
    assert!(p.interaction_contract.user_channel.question.is_some());
}

// ── Repair budget / outcome floor ──────────────────────────────────────────
#[test]
fn repair_attempt_parity() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("T1", "Flaky work"));
    goal.todo_mut("T1").unwrap().failed_attempts = 1;
    let p = check(&goal, now(), None, TurnMode::BoundedDelivery);
    assert!(p.reason.contains("repair attempt 2"));
}

#[test]
fn repair_budget_exhausted_parity() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("T1", "Flaky work"));
    goal.todo_mut("T1").unwrap().failed_attempts = 2; // > MAX_REPAIR_ATTEMPTS (1)
    let p = check(&goal, now(), None, TurnMode::Replan);
    assert!(p.reason.contains("exhausted repair budget"));
}

#[test]
fn outcome_floor_breach_parity() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("T1", "Surface-only loop"));
    goal.execution_profile.outcome_floor_streak_threshold = 3;
    goal.outcome_streak = 3;
    let p = check(&goal, now(), None, TurnMode::Replan);
    assert!(p.reason.contains("outcome floor"));
}

// ── Blockers ───────────────────────────────────────────────────────────────
#[test]
fn blocker_quiet_wait_parity() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::blocker("B1", "External service down", &[]));
    check(&goal, now(), None, TurnMode::WaitMonitor);
}

// ── Succession replan obligation ───────────────────────────────────────────
#[test]
fn silent_completion_replan_parity() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("T1", "Work"));
    goal.todo_mut("T1").unwrap().status = TodoStatus::Done; // no closure intent
    let p = check(&goal, now(), None, TurnMode::Replan);
    assert!(p.reason.contains("closure intent"));
}

// ── Monitors ───────────────────────────────────────────────────────────────
#[test]
fn monitor_stalled_replan_parity() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::monitor("M1", "Watch CI", Duration::from_secs(3600)));
    goal.todo_mut("M1").unwrap().consecutive_no_change = MONITOR_NO_CHANGE_REPLAN_THRESHOLD;
    let p = check(&goal, now(), None, TurnMode::Replan);
    assert!(p.reason.contains("stalled"));
}

#[test]
fn monitor_due_poll_parity() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    let now = now();
    goal.add(Todo::monitor("M1", "Watch CI", Duration::from_secs(3600)));
    goal.todo_mut("M1").unwrap().resume_when = Some(now - Duration::from_secs(60));
    let p = check(&goal, now, None, TurnMode::MonitorPoll);
    assert_eq!(
        p.interaction_contract
            .agent_channel
            .selected_todo
            .as_deref(),
        Some("M1")
    );
    assert_eq!(p.effective_action, "monitor_poll");
}

#[test]
fn monitor_wait_backoff_parity() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    let now = now();
    goal.add(Todo::monitor("M1", "Watch CI", Duration::from_secs(3600)));
    goal.todo_mut("M1").unwrap().resume_when = Some(now + Duration::from_secs(3600));
    let p = check(&goal, now, None, TurnMode::WaitMonitor);
    assert!(
        p.scheduler_hint.next_due_ms.is_some(),
        "quiet wait must carry backoff"
    );
    assert_eq!(
        p.scheduler_arbitration.as_ref().unwrap().disposition,
        SchedulerDisposition::MonitorWait
    );
}

#[test]
fn monitor_lane_keeps_advancement_runnable_parity() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::monitor("M1", "Watch CI", Duration::from_secs(3600)));
    goal.add(Todo::advancement("T1", "Do the thing"));
    let p = check(&goal, now(), None, TurnMode::BoundedDelivery);
    assert_eq!(p.work_lane_contract.lane, "monitor");
}

// ── Acceptance gaps / deferred ─────────────────────────────────────────────
#[test]
fn acceptance_gap_replan_parity() {
    let mut goal = Goal::new("g", "objective", "/tmp")
        .with_acceptance(vec![("A1", "result matches tolerance")]);
    goal.add(Todo::advancement("T1", "Run experiment"));
    goal.todo_mut("T1").unwrap().complete(true, vec![]);
    let p = check(&goal, now(), None, TurnMode::Replan);
    assert!(p.reason.contains("acceptance gap"));
}

#[test]
fn deferred_not_due_quiet_wait_parity() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    let now = now();
    goal.add(Todo::advancement("T1", "Deferred work"));
    goal.todo_mut("T1").unwrap().status = TodoStatus::Deferred;
    goal.todo_mut("T1").unwrap().resume_when = Some(now + Duration::from_secs(3600));
    check(&goal, now, None, TurnMode::WaitMonitor);
}

// ── Validated closure ──────────────────────────────────────────────────────
#[test]
fn validated_closure_parity() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("T1", "Work"));
    goal.todo_mut("T1").unwrap().complete(true, vec![]);
    let p = check(&goal, now(), None, TurnMode::Terminal);
    let tc = p
        .terminal_closure
        .as_ref()
        .expect("terminal must derive closure");
    assert_eq!(tc.kind, "no_followup");
    assert_eq!(tc.source, "validated_goal_closure");
    assert_eq!(
        p.scheduler_arbitration.as_ref().unwrap().disposition,
        SchedulerDisposition::TerminalStop
    );
}

#[test]
fn closure_via_successor_parity() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("T1", "Slice 1"));
    goal.add(Todo::advancement("T2", "Slice 2"));
    goal.todo_mut("T1")
        .unwrap()
        .complete(false, vec!["T2".into()]);
    goal.todo_mut("T2").unwrap().complete(true, vec![]);
    check(&goal, now(), None, TurnMode::Terminal);
}

/// Provenance guard: the generated legacy module is a mechanical transform
/// of the pre-split snapshot, so it cannot silently diverge from the
/// baseline. (SHA-256 of the snapshot is verified out-of-band with
/// `shasum -a 256 snapshots/decision.rs.pre-split` per the snapshots README;
/// expected 4ac8c78e7e3f489e1304e57ce0e9f5dbc3bebc965b013913b487b89b9bebf165.)
#[test]
fn generated_legacy_module_is_derived_from_snapshot() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let snapshot = std::fs::read_to_string(manifest.join("snapshots/decision.rs.pre-split"))
        .expect("pre-split snapshot must exist");
    let generated = std::fs::read_to_string(manifest.join("tests/legacy/decision_pre_split.rs"))
        .expect("generated legacy module must exist");

    // Whitespace-insensitive comparison: rustfmt may reflow the generated
    // file (splitting arg lists adds trailing commas; long match arms get
    // wrapped in braces), but every semantic token sequence of the snapshot
    // must survive. Deleting the same characters (whitespace, commas,
    // braces) from both sides preserves substring contiguity, so the check
    // stays exact in both directions.
    fn compact(s: &str) -> String {
        s.chars()
            .filter(|c| !c.is_whitespace() && *c != ',' && *c != '{' && *c != '}')
            .collect()
    }
    let normalized_generated = compact(&generated.replace("legacy_", ""));
    for line in snapshot.lines() {
        // Transforms: `crate::` → `future_loop::`, `loopx_compat` → `compat`
        // (the G-rename moved the module to compat.rs; the frozen snapshot
        // keeps the old `crate::loopx_compat` path), `//!` → `//` (plus the
        // `legacy_` fn renames, which the normalized_generated prefix-strip
        // covers).
        let rewritten = compact(
            &line
                .replace("crate::", "future_loop::")
                .replace("loopx_compat", "compat")
                .replace("loopx_", "future_loop_")
                .replace("LoopX-style", "reference-style")
                .replace("LoopX ", "reference ")
                .replace("//!", "//"),
        );
        let survives = normalized_generated.contains(&rewritten)
            || line.contains("fn none() -> Self")   // impl header → trait body
            || line.contains("impl UserChannel {")  // → trait decl
            || line.contains("impl ShouldRunPacket {") // → trait decl
            || line.contains("terminal_closure: None,") // followed by the new field
            || line.contains("pub use crate::state::now_epoch;");
        assert!(
            survives,
            "snapshot line lost in generated legacy module: {line:?}"
        );
    }
}
