//! G13 goal_frontier contract tests — the four assertion groups for the
//! goal_frontier subdomain:
//! ① OutcomeContinuity — segment reset on material turns and on
//!    frontier-changing events between runs (replay-folded markers);
//! ② ReplanRules — disposition → replan decision + obligation across the
//!    ordered rule table; `ReplanRuleSetUpdated` custom set + reset;
//! ③ SemanticHistory — bounded (N=50) goal-level semantic history folded
//!    from the event ledger as a standalone goal-scoped projection;
//! ④ TerminalJudgement — terminal closure tightened with acceptance-gap
//!    semantics and explicit gap detail, aligned with the closure proof.

use future_loop::decision::goal_frontier::outcome_continuity::{
    outcome_segments, run_is_material, SEGMENT_KIND_MATERIAL, SEGMENT_KIND_SURFACE_ONLY,
};
use future_loop::decision::goal_frontier::replan_rules::{
    active_rule_set, select_replan_rule, ReplanRuleSet, RULE_ADVANCEMENT_REMAINS,
    RULE_EXISTING_OBLIGATION, RULE_MONITOR_FRONTIER_EXHAUSTED, RULE_MONITOR_NO_CHANGE_STREAK,
    RULE_TODO_SUCCESSION_GAP, RULE_VISION_ACCEPTANCE_GAP,
};
use future_loop::decision::goal_frontier::semantic_history::{
    SemanticEvent, KIND_RUN_LANDED, SEMANTIC_HISTORY_CAP, SEMANTIC_HISTORY_SCHEMA_VERSION,
};
use future_loop::decision::goal_frontier::terminal::{
    terminal_judgement, GAP_OPEN_MONITOR, GAP_OPEN_TODO, GAP_PENDING_DEFERRED, GAP_SUCCESSION,
    GAP_UNSATISFIED_ACCEPTANCE, TERMINAL_KIND_NO_FOLLOWUP, TERMINAL_SOURCE_VALIDATED,
};
use future_loop::decision::goal_frontier::{frontier_show, FRONTIER_SHOW_SCHEMA_VERSION};
use future_loop::state::{Goal, RunRecord, Todo};
use future_loop::store::{Event, Store};

fn tmp_root(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "future-loop-g13-goal-frontier-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}

fn open_goal(store: &mut Store, goal_id: &str) -> Goal {
    let goal = Goal::new(goal_id, "objective", "/tmp");
    store.register(&goal).unwrap();
    store
        .append(Event::GoalStarted {
            goal_id: goal_id.into(),
            ts: goal.created_at,
        })
        .unwrap();
    goal
}

fn run(turn: u32, todo_id: &str, at: u64, tools: &[&str], evidence: &str) -> RunRecord {
    RunRecord {
        turn,
        todo_id: todo_id.to_string(),
        run_id: format!("r{turn}"),
        validation: None,
        terminal_state: "completed".to_string(),
        error: None,
        tokens_in_delta: 0,
        tokens_out_delta: 0,
        cost_delta: 0.0,
        tools: tools.iter().map(|s| s.to_string()).collect(),
        evidence: evidence.to_string(),
        recorded_at: at,
        spend_source: None,
        failure_kind: None,
        truncation: None,
    }
}

// ── ① OutcomeContinuity: segment reset ─────────────────────────────────────

#[test]
fn outcome_segments_surface_streak_and_material_flip() {
    let mut g = Goal::new("g", "o", "/tmp");
    g.history.push(run(1, "T1", 10, &[], ""));
    g.history.push(run(2, "T1", 20, &[], ""));
    g.history.push(run(3, "T1", 30, &["shell"], "artifact"));
    g.history.push(run(4, "T1", 40, &[], ""));
    let segments = outcome_segments(&g);
    assert_eq!(segments.len(), 3);
    assert_eq!(segments[0].kind, SEGMENT_KIND_SURFACE_ONLY);
    assert_eq!(segments[0].start_turn, 1);
    assert_eq!(segments[0].length, 2);
    assert_eq!(segments[1].kind, SEGMENT_KIND_MATERIAL);
    assert_eq!(segments[1].length, 1);
    assert_eq!(segments[2].kind, SEGMENT_KIND_SURFACE_ONLY);
    assert_eq!(segments[2].start_turn, 4);
    // Materiality rule: tools + evidence.
    assert!(run_is_material(&g.history[2]));
    assert!(!run_is_material(&g.history[1]));
}

#[test]
fn frontier_change_between_runs_resets_segment() {
    let mut g = Goal::new("g", "o", "/tmp");
    g.history.push(run(1, "T1", 10, &[], ""));
    g.history.push(run(2, "T1", 20, &[], ""));
    // A frontier change (marker at ts 25 — e.g. a todo completed after the
    // second run) splits the streak even though run 3 is surface-only.
    g.frontier_change_ts.push(25);
    g.history.push(run(3, "T2", 30, &[], ""));
    g.history.push(run(4, "T2", 40, &[], ""));
    let segments = outcome_segments(&g);
    assert_eq!(segments.len(), 2, "frontier change resets the segment");
    assert_eq!(segments[0].length, 2);
    assert_eq!(segments[1].start_turn, 3);
    assert_eq!(segments[1].length, 2);
}

#[test]
fn replay_folds_frontier_markers_and_resets_segments() {
    let mut store = Store::open(&tmp_root("segments")).unwrap();
    open_goal(&mut store, "g");
    store
        .append(Event::TodoAdded {
            goal_id: "g".into(),
            todo: Todo::advancement("T1", "first"),
            ts: 1,
        })
        .unwrap();
    // Two surface-only runs, then a completed todo (frontier change), then
    // one more surface-only run — replay must fold the marker and split.
    store.append_run("g", &run(1, "T1", 10, &[], "")).unwrap();
    store.append_run("g", &run(2, "T1", 20, &[], "")).unwrap();
    store
        .append(Event::TodoCompleted {
            goal_id: "g".into(),
            todo_id: "T1".into(),
            no_follow_up: true,
            successor_ids: vec![],
            evidence: None,
            ts: 25,
        })
        .unwrap();
    store.append_run("g", &run(3, "T2", 30, &[], "")).unwrap();
    let goal = store.replay("g").unwrap().expect("goal exists");
    assert!(goal.frontier_change_ts.contains(&25));
    let segments = outcome_segments(&goal);
    assert_eq!(segments.len(), 2, "completion marker resets the segment");
    assert_eq!(segments[1].start_turn, 3);
}

// ── ② ReplanRules: rule trigger + rule set ─────────────────────────────────

#[test]
fn succession_gap_selects_rule_and_derives_obligation() {
    let mut g = Goal::new("g", "o", "/tmp");
    let mut todo = Todo::advancement("T1", "work");
    todo.complete(false, vec![]); // silent completion → succession gap
    g.add(todo);
    let d = select_replan_rule(&g);
    assert_eq!(d.rule, RULE_TODO_SUCCESSION_GAP);
    assert!(d.derives_obligation);
    assert_eq!(d.obligation_kind.as_deref(), Some("succession_gap"));
    assert!(d.reason.contains("successor"));
}

#[test]
fn acceptance_gap_on_empty_frontier_selects_vision_rule() {
    let mut g = Goal::new("g", "o", "/tmp").with_acceptance(vec![("A1", "match")]);
    let mut todo = Todo::advancement("T1", "work");
    todo.complete(true, vec![]);
    g.add(todo);
    let d = select_replan_rule(&g);
    assert_eq!(d.rule, RULE_VISION_ACCEPTANCE_GAP);
    assert!(d.derives_obligation);
    assert_eq!(d.obligation_kind.as_deref(), Some("vision_acceptance_gap"));
}

#[test]
fn monitor_streak_on_monitor_only_lane_triggers_monitor_rule() {
    let mut g = Goal::new("g", "o", "/tmp");
    let mut m = Todo::monitor("M1", "watch", std::time::Duration::from_secs(60));
    m.consecutive_no_change = 3;
    g.add(m);
    let d = select_replan_rule(&g);
    assert_eq!(d.rule, RULE_MONITOR_NO_CHANGE_STREAK);
    assert!(d.derives_obligation);
    assert_eq!(
        d.obligation_kind.as_deref(),
        Some("monitor_no_change_streak")
    );
    // A runnable todo reopens the lane: not_monitor_only, no obligation.
    let mut g2 = Goal::new("g2", "o", "/tmp");
    g2.add(Todo::advancement("T1", "work"));
    let d2 = select_replan_rule(&g2);
    assert_eq!(d2.rule, "not_monitor_only");
    assert!(!d2.derives_obligation);
}

#[test]
fn surface_only_streak_obligation_is_authoritative_and_first_in_policy_order() {
    let mut g = Goal::new("g", "o", "/tmp");
    g.add(Todo::advancement("T1", "work"));
    // The outcome floor breach puts a surface-only-streak obligation on
    // record: EXISTING_OBLIGATION fires before any lane rule.
    g.execution_profile.outcome_floor_streak_threshold = 2;
    g.outcome_streak = 2;
    let d = select_replan_rule(&g);
    assert_eq!(d.rule, RULE_EXISTING_OBLIGATION);
    assert!(!d.derives_obligation);
    assert!(d.reason.contains("authoritative"));
}

#[test]
fn replan_rule_set_updated_event_customizes_and_resets() {
    let mut store = Store::open(&tmp_root("rules")).unwrap();
    open_goal(&mut store, "g");
    store
        .append(Event::TodoAdded {
            goal_id: "g".into(),
            todo: Todo::advancement("T1", "work"),
            ts: 1,
        })
        .unwrap();
    // Default: not_monitor_only (runnable advancement, no obligations).
    let goal = store.replay("g").unwrap().unwrap();
    assert_eq!(select_replan_rule(&goal).rule, "not_monitor_only");
    // Custom set with the exhausted rule first: it fires instead.
    store
        .append(Event::ReplanRuleSetUpdated {
            goal_id: "g".into(),
            rule_set_version: "goal_frontier_replan_rules_v0".into(),
            rule_ids: vec![RULE_MONITOR_FRONTIER_EXHAUSTED.to_string()],
            ts: 2,
        })
        .unwrap();
    let goal = store.replay("g").unwrap().unwrap();
    assert_eq!(
        select_replan_rule(&goal).rule,
        RULE_MONITOR_FRONTIER_EXHAUSTED
    );
    let active = active_rule_set(&goal);
    assert_eq!(
        active.effective_rule_ids(),
        vec![RULE_MONITOR_FRONTIER_EXHAUSTED.to_string()],
        "a custom set with a known rule fully replaces the default order"
    );
    // Empty rule ids on the wire reset to the default set.
    store
        .append(Event::ReplanRuleSetUpdated {
            goal_id: "g".into(),
            rule_set_version: "goal_frontier_replan_rules_v0".into(),
            rule_ids: vec![],
            ts: 3,
        })
        .unwrap();
    let goal = store.replay("g").unwrap().unwrap();
    assert_eq!(select_replan_rule(&goal).rule, "not_monitor_only");
    assert_eq!(
        active_rule_set(&goal),
        ReplanRuleSet {
            schema_version: "goal_frontier_replan_rules_v0".to_string(),
            rule_ids: vec![
                RULE_EXISTING_OBLIGATION.to_string(),
                "open_user_todo".to_string(),
                RULE_TODO_SUCCESSION_GAP.to_string(),
                RULE_VISION_ACCEPTANCE_GAP.to_string(),
                RULE_MONITOR_NO_CHANGE_STREAK.to_string(),
                "not_monitor_only".to_string(),
                "no_open_monitor".to_string(),
                RULE_ADVANCEMENT_REMAINS.to_string(),
                RULE_MONITOR_FRONTIER_EXHAUSTED.to_string(),
            ],
        }
    );
}

// ── ③ SemanticHistory: bounded goal-scoped fold ────────────────────────────

#[test]
fn semantic_history_is_bounded_to_cap() {
    let mut g = Goal::new("g", "o", "/tmp");
    for i in 0..(SEMANTIC_HISTORY_CAP + 20) {
        g.record_semantic_event(KIND_RUN_LANDED, Some("T1"), &format!("run {i}"), i as u64);
    }
    assert_eq!(g.semantic_history.len(), SEMANTIC_HISTORY_CAP);
    assert_eq!(g.semantic_history.first().unwrap().ts, 20, "oldest dropped");
    assert_eq!(
        g.semantic_history.last().unwrap().ts,
        (SEMANTIC_HISTORY_CAP + 19) as u64,
        "newest kept"
    );
}

#[test]
fn replay_folds_semantic_events_from_the_ledger() {
    let mut store = Store::open(&tmp_root("semantic")).unwrap();
    open_goal(&mut store, "g");
    store
        .append(Event::TodoAdded {
            goal_id: "g".into(),
            todo: Todo::advancement("T1", "work"),
            ts: 1,
        })
        .unwrap();
    let r1 = run(1, "T1", 10, &["shell"], "artifact one");
    store.append_run("g", &r1).unwrap();
    store
        .append(Event::RunRecorded {
            goal_id: "g".into(),
            record: r1,
            ts: 10,
        })
        .unwrap();
    store
        .append(Event::TodoCompleted {
            goal_id: "g".into(),
            todo_id: "T1".into(),
            no_follow_up: true,
            successor_ids: vec![],
            evidence: None,
            ts: 11,
        })
        .unwrap();
    store
        .append(Event::ReplanAcked {
            goal_id: "g".into(),
            delta_kinds: vec!["vision_patch".to_string()],
            ts: 12,
        })
        .unwrap();
    store
        .append(Event::SuccessionOccurred {
            goal_id: "g".into(),
            primary: "p".into(),
            backup: "b".into(),
            reason: "offline".into(),
            ts: 13,
        })
        .unwrap();
    let goal = store.replay("g").unwrap().unwrap();
    let kinds: Vec<&str> = goal
        .semantic_history
        .iter()
        .map(|e| e.kind.as_str())
        .collect();
    assert_eq!(
        kinds,
        vec![
            "run_landed",
            "todo_completed",
            "replan_acked",
            "role_succession"
        ]
    );
    assert_eq!(
        goal.semantic_history[0].summary, "completed — artifact one",
        "run summary = terminal state + evidence excerpt"
    );
    assert_eq!(goal.semantic_history[1].todo_id.as_deref(), Some("T1"));
    assert_eq!(goal.semantic_history[3].summary, "p→b (offline)");
}

#[test]
fn semantic_history_projection_folds_goal_scoped_events() {
    let mut g = Goal::new("g", "o", "/tmp");
    g.record_semantic_event(KIND_RUN_LANDED, Some("T1"), "completed", 42);
    // The projection is goal-scoped and replay-folded; the schema version
    // contract stays pinned so downstream readers can detect drift.
    assert_eq!(SEMANTIC_HISTORY_SCHEMA_VERSION, "goal_semantic_history_v0");
    assert_eq!(g.semantic_history.len(), 1);
    assert_eq!(g.semantic_history[0].kind, KIND_RUN_LANDED);
    assert_eq!(g.semantic_history[0].todo_id.as_deref(), Some("T1"));
    assert_eq!(g.semantic_history[0].ts, 42);
    assert_eq!(g.semantic_history[0].summary, "completed");
    // Bounded ring: oldest events drop past the cap.
    for i in 0..(SEMANTIC_HISTORY_CAP + 3) as u64 {
        g.record_semantic_event(KIND_RUN_LANDED, None, "more", 100 + i);
    }
    assert_eq!(g.semantic_history.len(), SEMANTIC_HISTORY_CAP);
    assert_eq!(g.semantic_history.first().unwrap().ts, 103);
}

// ── ④ TerminalJudgement: tightened terminal closure with gap detail ────────

#[test]
fn open_todo_and_acceptance_gap_enumerate_gap_details() {
    let mut g = Goal::new("g", "o", "/tmp").with_acceptance(vec![("A1", "match the spec")]);
    g.add(Todo::advancement("T1", "work"));
    let j = terminal_judgement(&g);
    assert!(!j.terminal);
    assert_eq!(j.kind, None);
    assert_eq!(j.source, None);
    assert_eq!(j.gaps.len(), 2);
    let open = j.gaps.iter().find(|gap| gap.kind == GAP_OPEN_TODO).unwrap();
    assert_eq!(open.todo_id.as_deref(), Some("T1"));
    let acceptance = j
        .gaps
        .iter()
        .find(|gap| gap.kind == GAP_UNSATISFIED_ACCEPTANCE)
        .unwrap();
    assert_eq!(acceptance.gap_id.as_deref(), Some("A1"));
    assert_eq!(acceptance.description, "match the spec");
    assert!(!acceptance.satisfied, "acceptance-gap semantics preserved");
    // Closing both gaps → terminal no_followup from validated sources.
    g.satisfy_gap("A1");
    g.todo_mut("T1").unwrap().complete(true, vec![]);
    let j = terminal_judgement(&g);
    assert!(j.terminal);
    assert_eq!(j.kind.as_deref(), Some(TERMINAL_KIND_NO_FOLLOWUP));
    assert_eq!(j.source.as_deref(), Some(TERMINAL_SOURCE_VALIDATED));
    assert!(j.gaps.is_empty());
}

#[test]
fn monitor_and_deferred_gaps_have_distinct_kinds_and_align_with_proof() {
    let mut g = Goal::new("g", "o", "/tmp");
    let mut done = Todo::advancement("T1", "work");
    done.complete(true, vec![]);
    g.add(done);
    g.add(Todo::monitor(
        "M1",
        "watch",
        std::time::Duration::from_secs(600),
    ));
    g.add(Todo::deferred(
        "D1",
        "later",
        std::time::Duration::from_secs(600),
    ));
    let j = terminal_judgement(&g);
    assert!(!j.terminal);
    assert!(j.gaps.iter().any(|gap| gap.kind == GAP_OPEN_MONITOR));
    assert!(j.gaps.iter().any(|gap| gap.kind == GAP_PENDING_DEFERRED));
    // Closure proof alignment: monitor_open_count mirrors the gap detail.
    assert_eq!(j.closure_proof.monitor_open_count, 1);
    assert!(!j.closure_proof.all_todos_done);
}

#[test]
fn succession_gap_blocks_terminal_and_matches_is_terminal() {
    let mut g = Goal::new("g", "o", "/tmp");
    let mut todo = Todo::advancement("T1", "work");
    todo.complete(false, vec![]); // no successor, no no-follow-up
    g.add(todo);
    let j = terminal_judgement(&g);
    assert!(!j.terminal);
    assert_eq!(j.gaps.len(), 1);
    assert_eq!(j.gaps[0].kind, GAP_SUCCESSION);
    assert_eq!(j.gaps[0].todo_id.as_deref(), Some("T1"));
    assert_eq!(j.closure_proof.successor_gap_count, 1);
    // Judgement ⇔ kernel terminal predicate across mixed states.
    assert_eq!(j.terminal, g.is_terminal());
    g.todo_mut("T1").unwrap().no_follow_up = true;
    g.todo_mut("T1").unwrap().successor_ids = vec![];
    assert!(g.is_terminal());
    assert!(terminal_judgement(&g).terminal);
}

#[test]
fn frontier_show_composes_all_four_layers() {
    let mut g = Goal::new("g", "o", "/tmp").with_acceptance(vec![("A1", "match")]);
    g.add(Todo::advancement("T1", "work"));
    g.record_semantic_event(KIND_RUN_LANDED, Some("T1"), "completed", 1);
    let show = frontier_show(&g);
    assert_eq!(show.schema_version, FRONTIER_SHOW_SCHEMA_VERSION);
    assert_eq!(show.goal_id, "g");
    assert_eq!(show.lane, "advancement_task");
    assert_eq!(show.frontier_projection.acceptance_gaps, 1);
    assert_eq!(show.frontier_projection.unclaimed_advancement, 1);
    assert!(show.outcome_segments.is_empty(), "no runs yet");
    assert_eq!(show.replan_rule.rule, "not_monitor_only");
    assert!(!show.terminal_judgement.terminal);
    assert_eq!(show.semantic_history.len(), 1);
    let _: Vec<SemanticEvent> = show.semantic_history;
}
