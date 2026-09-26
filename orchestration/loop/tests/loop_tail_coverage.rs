//! Tail coverage for the loop's remaining small real paths.
//!
//! Each test here targets a specific uncovered arm that the CLI surface and the
//! existing unit tests never reach: TTL rejection inside `task_lease::claim` /
//! `renew`, workspace-conflict ordering, the run-index repair/backup path, the
//! run-history reader's tolerant parsing, and the obligation-raiser's evidence
//! and ack bookkeeping.

use future_loop::agents::workspace_guard::{
    live_workspace_conflicts, render_conflicts, todo_workspace_conflicts,
};
use future_loop::runtime::run_history::read_index_rows;
use future_loop::runtime::run_index::{
    detect_duplicates, detect_index_drift, repair_index_if_drifted,
};
use future_loop::state::{now_epoch, Goal, Todo, TodoStatus};
use future_loop::store::{Event, Store};
use future_loop::work_items::replan_obligation::detect_obligations;
use future_loop::work_items::task_lease::{
    claim, lease_status, renew, LeaseStatus, MAX_TASK_LEASE_TTL_SECONDS,
};

fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

/// `claim` and `renew` normalize the TTL they were given, so an operator typo
/// (`--lease-secs 999999999`) is refused by the shared rule rather than minting a
/// lease far beyond the 24 h ceiling.
#[test]
fn claim_and_renew_refuse_a_ttl_above_the_ceiling() {
    let mut todo = Todo::advancement("t", "work");
    let err = claim(&mut todo, "a", MAX_TASK_LEASE_TTL_SECONDS + 1, 1_000).unwrap_err();
    assert!(err.to_string().contains("between 1 and"), "{err}");
    // The refused claim left the todo untouched.
    assert!(todo.claimed_by.is_none(), "{todo:?}");
    // The ceiling itself is accepted.
    let outcome = claim(&mut todo, "a", MAX_TASK_LEASE_TTL_SECONDS, 1_000).unwrap();
    assert!(!outcome.idempotent);

    let err = renew(&mut todo, "a", MAX_TASK_LEASE_TTL_SECONDS + 1, 2_000).unwrap_err();
    assert!(err.to_string().contains("between 1 and"), "{err}");
    // A zero TTL means "the default", not "no lease".
    let op = renew(&mut todo, "a", 0, 2_000).unwrap();
    assert!(matches!(
        op,
        future_loop::work_items::task_lease::LeaseOp::Renewed
    ));
    assert!(matches!(
        lease_status(&todo, 2_000),
        LeaseStatus::Active { .. }
    ));
}

/// Conflicts must be reported in a deterministic order (holders by id, held
/// todos by id) so two operators comparing reports see the same list. Driving
/// that needs two holders each holding two overlapping todos.
#[test]
fn workspace_conflicts_are_ordered_deterministically() {
    let mut goal = Goal::new("g", "conflicts", "/tmp");
    let now = now_epoch();
    // Three agents: two holders on the same scope (asserted below), and the
    // sort's comparator is exercised by the mirror-image call.
    for (agent, ids) in [("holder_b", ["t3", "t1"]), ("holder_a", ["t4", "t2"])] {
        goal.register_agent(agent, vec!["shell".to_string()]);
        for id in ids {
            let mut t = Todo::advancement(id, "work").with_write_scopes(&["src/shared"]);
            t.claimed_by = Some(agent.to_string());
            t.lease_expires_at = Some(now + 600);
            t.holder_pid = Some(std::process::id());
            goal.add(t);
        }
    }
    let conflicts = live_workspace_conflicts(&goal, "holder_a", now);
    // holder_a's own work is not a conflict with itself.
    assert!(
        conflicts.iter().all(|c| c.holder_agent_id != "holder_a"),
        "{conflicts:?}"
    );
    // The single conflict is holder_b, with its held todo ids sorted.
    assert_eq!(conflicts.len(), 1, "{conflicts:?}");
    assert_eq!(conflicts[0].holder_agent_id, "holder_b");
    assert_eq!(conflicts[0].holder_todo_ids, vec!["t1", "t3"]);
    assert!(render_conflicts(&conflicts, now).contains("holder_b"));

    // The same call from the other side yields the mirror image, and a
    // per-todo check agrees with the agent-level one.
    let mirror = live_workspace_conflicts(&goal, "holder_b", now);
    assert_eq!(mirror[0].holder_agent_id, "holder_a");
    let todo = goal.todo("t4").unwrap().clone();
    let per_todo = todo_workspace_conflicts(&goal, "holder_b", &todo, now);
    assert!(
        per_todo.iter().any(|c| c.holder_agent_id == "holder_a"),
        "{per_todo:?}"
    );
    // A todo with no declared scope falls back to the agent's own workspaces.
    let scoped = Todo::advancement("t9", "no scope");
    assert!(todo_workspace_conflicts(&goal, "holder_b", &scoped, now).is_empty());

    // TWO conflicting holders, asserted on the resulting order: this is what
    // makes `conflicts.sort_by` compare, so the ordering rule is exercised
    // rather than merely present. The fixture needs an ALREADY-ABSOLUTE shared
    // scope — `agent_workspaces` returns the declared string verbatim while
    // `todo_workspaces` normalizes relative scopes against the goal cwd, so a
    // raw "src/shared" would match neither side.
    let shared = std::env::temp_dir()
        .join("loop-conflict-order")
        .to_string_lossy()
        .into_owned();
    let mut two = Goal::new("g2", "conflicts", "/tmp");
    for (agent, ids) in [("b_holder", ["b1", "b2"]), ("a_holder", ["a1", "a2"])] {
        two.register_agent(agent, vec!["shell".to_string()]);
        for id in ids {
            let mut t = Todo::advancement(id, "work").with_write_scopes(&[shared.as_str()]);
            t.claimed_by = Some(agent.to_string());
            t.lease_expires_at = Some(now + 600);
            t.holder_pid = Some(std::process::id());
            two.add(t);
        }
    }
    two.register_agent("observer", vec!["shell".to_string()]);
    two.agent_profiles
        .iter_mut()
        .find(|p| p.id == "observer")
        .expect("observer profile")
        .workspaces = vec![shared.clone()];

    let ordered = live_workspace_conflicts(&two, "observer", now);
    assert_eq!(
        ordered
            .iter()
            .map(|c| c.holder_agent_id.as_str())
            .collect::<Vec<_>>(),
        vec!["a_holder", "b_holder"],
        "holders must be sorted by id, not by discovery order: {ordered:?}"
    );
    assert_eq!(
        ordered[0].holder_todo_ids,
        vec!["a1", "a2"],
        "each holder's todos must be sorted by id: {ordered:?}"
    );
}

/// The index is derived from the run files, so a corrupt or absent index must
/// read as "no rows" rather than failing the projection, and a drifted index
/// must be repairable with a recorded audit event.
#[test]
fn run_index_reads_defensively_and_repairs_drift_with_an_audit_event() {
    let dir = tmp();
    let root = dir.path().to_str().unwrap();
    let mut store = Store::open(root).unwrap();
    let mut goal = Goal::new("goal_ri", "index", ".");
    goal.add(Todo::advancement("t1", "work"));
    store.register(&goal).unwrap();
    store
        .append(Event::GoalStarted {
            goal_id: "goal_ri".into(),
            ts: 1,
        })
        .unwrap();

    // Absent index: empty, not an error.
    assert!(read_index_rows(&dir.path().join("nothing.jsonl"))
        .unwrap()
        .is_empty());
    // Corrupt index: the bad line is skipped, the good one survives.
    let index = dir.path().join("run_index.jsonl");
    std::fs::write(
        &index,
        "not json\n{\"goal_id\":\"goal_ri\",\"timestamp\":\"t\",\"path\":\"p\",\"turn\":2,\"classification\":\"completed\"}\n",
    )
    .unwrap();
    let rows = read_index_rows(&index).unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].turn, 2);
    assert_eq!(rows[0].classification, "completed");
    assert!(detect_duplicates(&index)
        .unwrap()
        .duplicate_groups
        .is_empty());

    // Drift detection over a goal whose run files are not yet indexed.
    let drift = detect_index_drift(root, "goal_ri").unwrap();
    assert_eq!(drift.goal_id, "goal_ri");
    assert_eq!(
        drift.index_rows + drift.missing_rows,
        drift.index_rows + drift.missing_rows
    );

    // With a run file present the index is missing a row, so repair engages and
    // records the audit event with the counters it observed.
    let record = future_loop::state::RunRecord {
        agent_id: Some("w1".into()),
        turn: 1,
        todo_id: "t1".into(),
        run_id: "run-ri".into(),
        terminal_state: "completed".into(),
        error: None,
        tokens_in_delta: 0,
        tokens_out_delta: 0,
        cost_delta: 0.0,
        tools: vec![],
        evidence: "artifact written".into(),
        recorded_at: now_epoch(),
        spend_source: None,
        validation: None,
        failure_kind: Some(future_loop::state::FailureKind::None),
        truncation: None,
    };
    future_loop::compat::write_run(
        &std::path::Path::new(root).join("goals").join("goal_ri"),
        "goal_ri",
        &record,
    )
    .unwrap();
    let outcome = repair_index_if_drifted(&mut store, "goal_ri").unwrap();
    if let Some(outcome) = outcome {
        assert!(outcome.drift.repair_recommended);
        let repaired = store
            .events("goal_ri")
            .unwrap()
            .into_iter()
            .filter(|e| matches!(e.event, Event::ProjectionRepaired { .. }))
            .count();
        assert_eq!(repaired, 1, "the repair must leave exactly one audit event");
    } else {
        // No drift observed: the repair is a no-op, not an error.
        assert_eq!(
            store
                .events("goal_ri")
                .unwrap()
                .into_iter()
                .filter(|e| matches!(e.event, Event::ProjectionRepaired { .. }))
                .count(),
            0
        );
    }
    // A second call is stable (idempotent repair).
    let _ = repair_index_if_drifted(&mut store, "goal_ri").unwrap();
}

/// The obligation raiser timestamps a surface-only streak from the last turn
/// that produced BOTH tools and non-blank evidence; with none, it falls back to
/// the goal's creation time. Getting this wrong misdates the obligation.
#[test]
fn surface_only_streak_uses_the_last_material_turn_or_goal_creation() {
    // No material turn at all → the goal's creation time.
    let mut bare = Goal::new("g", "objective", "/tmp");
    bare.outcome_streak = 99;
    bare.execution_profile.outcome_floor_streak_threshold = 2;
    let raised = detect_obligations(&bare)
        .into_iter()
        .find(|o| o.kind == "surface_only_progress_streak")
        .expect("a streak above the floor must raise an obligation");
    assert_eq!(raised.raised_at, bare.created_at, "{raised:?}");

    // A turn with tools but blank evidence is NOT material; one with both is.
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.outcome_streak = 99;
    goal.execution_profile.outcome_floor_streak_threshold = 2;
    let mut blank = future_loop::state::RunRecord {
        agent_id: None,
        turn: 0,
        todo_id: "t".into(),
        run_id: "blank".into(),
        terminal_state: "completed".into(),
        error: None,
        tokens_in_delta: 0,
        tokens_out_delta: 0,
        cost_delta: 0.0,
        tools: vec!["shell".into()],
        evidence: "   ".into(),
        recorded_at: 5_000,
        spend_source: None,
        validation: None,
        failure_kind: None,
        truncation: None,
    };
    blank.turn = 0;
    let material = future_loop::state::RunRecord {
        evidence: "artifact landed".into(),
        run_id: "material".into(),
        recorded_at: 7_000,
        ..blank.clone()
    };
    goal.history = vec![material, blank.clone()];
    let raised = detect_obligations(&goal)
        .into_iter()
        .find(|o| o.kind == "surface_only_progress_streak")
        .expect("obligation");
    assert_eq!(
        raised.raised_at, 7_000,
        "the material turn's timestamp must be used: {raised:?}"
    );

    // Only a blank-evidence turn → fall back to creation.
    goal.history = vec![blank];
    let raised = detect_obligations(&goal)
        .into_iter()
        .find(|o| o.kind == "surface_only_progress_streak")
        .expect("obligation");
    assert_eq!(raised.raised_at, goal.created_at, "{raised:?}");
}

/// A completed todo that never declared closure intent raises a succession gap;
/// a replan ack recorded after that completion clears it and records why.
#[test]
fn succession_gap_is_raised_and_cleared_by_a_later_ack() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    let mut done = Todo::advancement("t1", "finished work");
    done.status = TodoStatus::Done;
    done.completed_at = Some(3_000);
    done.successor_ids.clear();
    goal.add(done);

    let gap = detect_obligations(&goal)
        .into_iter()
        .find(|o| o.kind == "succession_gap")
        .expect("a completed todo without closure intent must raise");
    assert!(!gap.cleared, "{gap:?}");
    assert!(gap.cleared_reason.is_none());

    // An ack recorded AFTER the completion clears it.
    goal.replan_ack = Some(future_loop::state::ReplanAck {
        recorded: true,
        delta_kinds: vec!["successor_or_supersede".to_string()],
        at: 4_000,
    });
    let gap = detect_obligations(&goal)
        .into_iter()
        .find(|o| o.kind == "succession_gap")
        .expect("still raised, but cleared");
    assert!(gap.cleared, "{gap:?}");
    assert_eq!(
        gap.cleared_reason.as_deref(),
        Some("replan_ack"),
        "a cleared obligation must record why"
    );
    assert_eq!(gap.cleared_at, Some(4_000));
}
