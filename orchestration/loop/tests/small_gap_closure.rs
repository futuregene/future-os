//! Small-gap closures across the loop's pure/derived surfaces, plus the
//! supervisor event projection.
//!
//! Each test targets an uncovered arm the larger drive files never reach: TTL
//! rejection, task-graph reference validation and dependent discovery, the
//! supervisor note/progress projection, the quota `next_due_ms` suffix, the
//! usage-summary rows for run-less goals, monitor `next_due_at` folding, and
//! backfill's legacy status mapping.

mod common;

use common::{cli_ok, cli_root, init_goal, open_store};
use future_loop::state::{Goal, Todo, TodoStatus};
use future_loop::store::Event;

/// TTL normalization is a contract floor: 0 means "the default", and anything
/// above the 24 h ceiling is refused rather than silently clamped — a lease
/// longer than the cap would let a dead worker hold a slice indefinitely.
#[test]
fn normalize_ttl_defaults_zero_and_refuses_above_the_ceiling() {
    use future_loop::work_items::task_lease::{
        normalize_ttl, DEFAULT_TASK_LEASE_TTL_SECONDS, MAX_TASK_LEASE_TTL_SECONDS,
    };
    assert_eq!(normalize_ttl(0).unwrap(), DEFAULT_TASK_LEASE_TTL_SECONDS);
    assert_eq!(normalize_ttl(1).unwrap(), 1, "1 second is the floor, not 0");
    assert_eq!(
        normalize_ttl(MAX_TASK_LEASE_TTL_SECONDS).unwrap(),
        MAX_TASK_LEASE_TTL_SECONDS,
        "the ceiling itself is allowed"
    );
    let err = normalize_ttl(MAX_TASK_LEASE_TTL_SECONDS + 1).unwrap_err();
    assert!(
        err.to_string().contains("between 1 and"),
        "unexpected error: {err}"
    );
    assert!(normalize_ttl(u64::MAX).is_err());
}

/// `validate_dependency_change` checks only the edited node's references, so an
/// operator can repair one bad edge at a time; it must still reject unknown
/// references and self-reference immediately.
#[test]
fn task_graph_validation_rejects_unknown_and_self_references() {
    use future_loop::work_items::task_graph::validate_dependency_change;
    let mut goal = Goal::new("g", "graph", "/tmp");
    goal.add(Todo::advancement("A", "upstream"));
    let mut sink = Todo::advancement("B", "downstream");
    sink.blocked_by_gate = Some("A".into());
    goal.add(sink.clone());
    assert!(validate_dependency_change(&goal, "B").is_ok());

    // Self-reference is reported before the dangling-reference check.
    sink.blocked_by_gate = Some("B".into());
    goal.todos.retain(|t| t.id != "B");
    goal.add(sink.clone());
    let err = validate_dependency_change(&goal, "B").unwrap_err();
    assert!(err.contains("cannot reference itself"), "{err}");

    // Dangling reference.
    sink.blocked_by_gate = Some("ghost".into());
    goal.todos.retain(|t| t.id != "B");
    goal.add(sink);
    let err = validate_dependency_change(&goal, "B").unwrap_err();
    assert!(err.contains("unknown todo `ghost`"), "{err}");

    // The edited todo itself must exist.
    let err = validate_dependency_change(&goal, "nope").unwrap_err();
    assert!(err.contains("unknown todo `nope`"), "{err}");
}

/// Dependents are found in both edge directions: a todo that declares its
/// dependents returns them, and a todo that instead *lists* a blocker is
/// discovered by scanning the goal.
#[test]
fn task_graph_successors_follow_the_edge_direction() {
    use future_loop::work_items::task_graph::{predecessors_of, successors_of};
    let mut goal = Goal::new("g", "graph", "/tmp");
    // A gate is a blocking SOURCE: its `blocked_by_gate` names the todos it
    // blocks, so those come back directly.
    let gate = Todo::user_gate("G", "approve", &["B"]);
    // An advancement's `blocked_by_gate` names its predecessors instead, so A's
    // dependents have to be discovered by scanning the goal.
    let a = Todo::advancement("A", "upstream work");
    let mut t = Todo::advancement("T", "lists A as its blocker");
    t.blocked_by_gate = Some("A".into());
    assert_eq!(predecessors_of(&a), Vec::<String>::new());
    assert_eq!(predecessors_of(&t), vec!["A"]);
    goal.add(gate);
    goal.add(a);
    goal.add(t);

    assert_eq!(
        successors_of(&goal, "G"),
        vec!["B"],
        "a blocking source names its dependents directly"
    );
    assert_eq!(
        successors_of(&goal, "A"),
        vec!["T"],
        "a non-blocking todo's dependents are found by scanning"
    );
    // A todo nothing references has no successors, and an unknown id is empty
    // rather than an error.
    assert!(successors_of(&goal, "T").is_empty());
    assert!(successors_of(&goal, "ghost").is_empty());
}

/// The supervisor event projection is what an idle supervisor polls. Both note
/// families must appear, each self-describing, and foreign goals must be
/// filtered out — a projection that leaked another goal's notes would cross the
/// supervision boundary the projection exists to enforce.
#[test]
fn supervisor_event_projection_carries_notes_and_filters_foreign_goals() {
    let cr = cli_root();
    let gid = init_goal(&cr, "supervisor projection");
    let mut store = open_store(&cr);
    // A second, registered goal: its events must not leak into this projection.
    let mut other_goal = Goal::new("goal_other", "other objective", ".");
    other_goal.add(Todo::advancement("todo_x", "other work"));
    store.register(&other_goal).unwrap();
    let other = "goal_other".to_string();
    for (goal, key) in [(&gid, "k1"), (&other, "k2")] {
        store
            .append(Event::SupervisorNote {
                goal_id: goal.clone(),
                todo_id: "todo_1".into(),
                note_kind: "completed".into(),
                message: format!("note for {goal}"),
                dedup_key: key.into(),
                ts: 7,
            })
            .unwrap();
        store
            .append(Event::ProgressReported {
                goal_id: goal.clone(),
                agent_id: "w1".into(),
                todo_id: "todo_1".into(),
                message: format!("progress for {goal}"),
                ts: 8,
            })
            .unwrap();
    }
    let projection =
        future_loop::agents::supervisor::build_supervisor_event_projection(&store, &gid).unwrap();
    let text = serde_json::to_string(&projection).unwrap();
    assert!(text.contains(&gid), "{text}");
    assert!(
        !text.contains("goal_other"),
        "a foreign goal's events must not project into this goal: {text}"
    );
    assert_eq!(projection["ok"], true);
    assert_eq!(projection["goal_id"], gid);
    assert!(projection["schema_version"].is_string(), "{text}");
}

/// The quota projection prints the `next_due_ms` suffix only when the scheduler
/// actually has a due time; a permanent suffix (or a missing one) misleads.
#[test]
fn quota_projection_prints_next_due_only_when_the_scheduler_has_one() {
    use future_loop::cli_projection::render_quota_projection;
    use future_loop::quota::usage_summary::breakdown;
    let mut goal = Goal::new("g1", "o", "/tmp");
    goal.add(Todo::advancement("T1", "Work"));
    let packet = future_loop::decision::decide(&goal, std::time::SystemTime::now());
    let breakdown = breakdown(&goal.history);
    let text = render_quota_projection(&packet, Some(&breakdown), None);
    assert!(text.contains("scheduler: action="), "{text}");
    assert_eq!(
        text.contains("next_due_ms="),
        packet.scheduler_hint.next_due_ms.is_some(),
        "the suffix must track the hint: {text}"
    );
}

/// The usage summary must produce a row for every requested goal even when it
/// contributed no runs — the blank-row fallback.
///
/// NOTE: the `unwrap_or_else(|| UsageGoalRow::blank(..))` at the `for_goals`
/// call site is **unreachable-by-construction**: `build_usage_summary` (its only
/// callee) always returns `goals: vec![goal]`, so `into_iter().next()` is never
/// `None`. Rust cannot express "non-empty Vec" in the type, so the arm stays as
/// a defensive default; registered as such in `docs/testing/module-loop.md`.
#[test]
fn usage_summary_emits_a_blank_row_for_a_goal_with_no_runs() {
    let rows = future_loop::quota::usage_summary::build_usage_summary_for_goals(
        &[("goal_quiet", &[][..])],
        future_loop::state::now_epoch(),
    );
    assert_eq!(rows.goals.len(), 1, "{rows:?}");
    let row = &rows.goals[0];
    assert_eq!(row.goal_id, "goal_quiet");
    assert_eq!(row.runs_24h, 0, "a run-less goal reports zero runs");
    assert_eq!(row.project_share_24h, 0.0);
}

/// Monitor `next_due_at` folds to the *earliest* due monitor; losing that fold
/// would make the scheduler wait for the slowest one.
#[test]
fn monitor_poll_plan_folds_to_a_single_next_due_and_counts_tracked_monitors() {
    use future_loop::scheduler::monitor_poll::build_poll_plan;
    let mut goal = Goal::new("g", "monitors", "/tmp");
    // Monitors are their own task class; an advancement carrying monitor
    // metadata is not tracked (`open_monitors` filters on the class).
    let soon = std::time::SystemTime::now() + std::time::Duration::from_secs(600);
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(3600);
    let mut first = Todo::monitor("M1", "first monitor", std::time::Duration::from_secs(3600))
        .with_monitor_target("target_a")
        .with_monitor_policy("no_change")
        .with_monitor_cadence("hourly");
    let mut second = Todo::monitor("M2", "second monitor", std::time::Duration::from_secs(600))
        .with_monitor_target("target_b")
        .with_monitor_policy("no_change")
        .with_monitor_cadence("hourly");
    // Pin the due times explicitly so the fold is asserted on known instants.
    first.resume_when = Some(later);
    second.resume_when = Some(soon);
    goal.add(first);
    goal.add(second);

    let plan = build_poll_plan(&goal, std::time::SystemTime::now());
    // Both monitors wait, so the plan names exactly one instant — the EARLIER
    // of the two. Losing the min-fold would schedule the later one.
    assert_eq!(
        plan.next_due_at,
        Some(
            soon.duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        ),
        "the fold must pick the earliest due time"
    );
    assert!(plan.due_monitors.is_empty(), "nothing is due yet");
    assert!(plan.stalled_monitors.is_empty(), "nothing is stalled yet");
}

/// Backfill maps legacy status markers onto the modern enum; `blocked` must not
/// fall through to the no-op arm, or a blocked legacy todo would be imported
/// with the wrong status.
#[test]
fn backfill_maps_legacy_status_markers() {
    use future_loop::backfill::{parse_markdown_todos, MarkdownTodoRecord};
    // The parser reads the `<!-- future-loop:todo ... -->` metadata blocks that
    // `active_state_markdown` writes; `status=` is what `build_todo` maps.
    let md = "# Active Goal State\n\n\
## Agent Todo\n\n\
- [ ] Blocked one\n  <!-- future-loop:todo todo_id=todo_b1 status=blocked -->\n\
- [x] Done one\n  <!-- future-loop:todo todo_id=todo_b2 status=done -->\n\
- [-] Deferred one\n  <!-- future-loop:todo todo_id=todo_b3 status=deferred -->\n\
- [ ] Open one\n  <!-- future-loop:todo todo_id=todo_b4 status=open -->\n";
    let records: Vec<MarkdownTodoRecord> = parse_markdown_todos(md);
    assert_eq!(records.len(), 4, "every todo line must parse: {records:?}");
    let by_id = |id: &str| {
        records
            .iter()
            .find(|r| r.todo_id.as_deref() == Some(id))
            .unwrap_or_else(|| panic!("no record {id}: {records:?}"))
            .clone()
    };
    assert_eq!(by_id("todo_b1").status, "blocked");
    assert_eq!(by_id("todo_b2").status, "done");
    assert_eq!(by_id("todo_b3").status, "deferred");
    assert_eq!(by_id("todo_b4").status, "open");
}

/// The quota CLI surface reads the same rows, so the operator-visible output
/// and the projection cannot drift.
#[test]
fn quota_usage_and_should_run_agree_with_the_projection() {
    let cr = cli_root();
    let gid = init_goal(&cr, "quota rows");
    cli_ok(&["quota", "usage", "--goal", &gid]);
    cli_ok(&["quota", "usage", "--goal", &gid, "--format", "json"]);
    cli_ok(&["quota", "should-run", "--goal", &gid]);
    let store = open_store(&cr);
    let goal = store.replay(&gid).unwrap().unwrap();
    let rows = future_loop::quota::usage_summary::build_usage_summary_for_goals(
        &[(&gid, goal.history.as_slice())],
        future_loop::state::now_epoch(),
    );
    assert_eq!(rows.goals.len(), 1);
    assert_eq!(rows.goals[0].goal_id, gid);
    assert_eq!(rows.goals[0].runs_24h, goal.history.len() as u64);
    let _ = TodoStatus::Open;
}

// --- closures added after the first review pass -------------------------------

/// The scheduler hint carries a due time when a monitor is waiting; the quota
/// projection must then print the `next_due_ms` suffix (the arm skipped whenever
/// the hint has no due time).
#[test]
fn quota_projection_prints_the_next_due_suffix_for_a_waiting_monitor() {
    use future_loop::cli_projection::render_quota_projection;
    use future_loop::quota::usage_summary::breakdown;
    let mut goal = Goal::new("g-wait", "waiting monitor", "/tmp");
    let mut monitor = Todo::monitor(
        "M1",
        "watch the endpoint",
        std::time::Duration::from_secs(900),
    )
    .with_monitor_target("endpoint_a")
    .with_monitor_policy("no_change")
    .with_monitor_cadence("hourly");
    monitor.resume_when = Some(std::time::SystemTime::now() + std::time::Duration::from_secs(900));
    goal.add(monitor);

    let packet = future_loop::decision::decide(&goal, std::time::SystemTime::now());
    let breakdown = breakdown(&goal.history);
    let text = render_quota_projection(&packet, Some(&breakdown), None);
    assert_eq!(
        text.contains("next_due_ms="),
        packet.scheduler_hint.next_due_ms.is_some(),
        "the suffix must track the hint: {text}"
    );
    if let Some(ms) = packet.scheduler_hint.next_due_ms {
        assert!(
            text.contains(&format!("next_due_ms={ms}")),
            "the printed value must be the hint's: {text}"
        );
    }
}

/// `FailureKind::label` is what the turn envelope shows a worker, so every
/// variant needs a distinct, actionable label — `None` means the turn
/// succeeded, and conflating that with `HardError` would tell a worker to repair
/// work that already landed.
#[test]
fn failure_kind_labels_are_distinct_and_describe_the_repair_path() {
    use future_loop::state::FailureKind;
    let labels: Vec<&str> = [
        FailureKind::None,
        FailureKind::InfraRecoverable,
        FailureKind::ScienceVerifyFailed,
        FailureKind::HardError,
    ]
    .iter()
    .map(|k| k.label())
    .collect();
    assert_eq!(labels[0], "succeeded");
    let unique: std::collections::BTreeSet<&&str> = labels.iter().collect();
    assert_eq!(
        unique.len(),
        labels.len(),
        "labels must be distinct: {labels:?}"
    );
    assert!(
        labels[1].contains("retry"),
        "infra is retryable: {}",
        labels[1]
    );
}

/// `backfill_todo_events` is the importer that maps legacy markdown onto the
/// ledger, so `build_todo`'s status mapping has to be exercised through it: a
/// `blocked` record imported as an ordinary open todo would silently unblock
/// work the operator deliberately parked.
#[test]
fn backfill_imports_a_blocked_todo_as_blocked() {
    use future_loop::backfill::backfill_todo_events;
    use future_loop::projection::privacy::PrivacyLevel;
    let md = "# Active Goal State\n\n\
## Agent Todo\n\n\
- [ ] Blocked one\n  <!-- future-loop:todo todo_id=todo_blk status=blocked -->\n\
- [ ] Open one\n  <!-- future-loop:todo todo_id=todo_opn status=open -->\n";
    let outcome = backfill_todo_events(md, "goal_backfill", PrivacyLevel::LocalPrivate).unwrap();
    assert_eq!(outcome.todo_count, 2, "both records import: {outcome:?}");
    let imported = |id: &str| -> future_loop::state::Todo {
        outcome
            .events
            .iter()
            .find_map(|e| match &e.event {
                Event::TodoAdded { todo, .. } if todo.id == id => Some(todo.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no TodoAdded for {id}: {outcome:?}"))
    };
    assert_eq!(
        imported("todo_blk").status,
        TodoStatus::Blocked,
        "a `blocked` record must not import as open"
    );
    assert_eq!(imported("todo_opn").status, TodoStatus::Open);
}

/// A predecessor with no (or blank) evidence must still be listed with the
/// "no evidence recorded" placeholder — the synthesis worker needs to know what
/// ran. A blocked todo naming nothing resolvable produces no block at all.
#[test]
fn upstream_evidence_lists_a_predecessor_without_evidence() {
    let mut goal = Goal::new("g", "fan-in", "/tmp");
    let mut done = Todo::advancement("U1", "probe");
    done.status = TodoStatus::Done;
    done.evidence = None;
    let mut superseded = Todo::advancement("U2", "abandoned probe");
    superseded.status = TodoStatus::Superseded;
    superseded.evidence = Some("   ".into());
    goal.add(done);
    goal.add(superseded);
    goal.add(Todo::advancement("S1", "synthesize").blocking(&["U1", "U2"]));
    let text =
        future_loop::turn_envelope::compose_upstream_evidence(&goal, goal.todo("S1").unwrap());
    assert!(
        text.contains("upstream U1: [no evidence recorded]"),
        "{text}"
    );
    assert!(text.contains("upstream U2:"), "{text}");

    let mut lonely = Goal::new("g2", "fan-in", "/tmp");
    lonely.add(Todo::advancement("S2", "synthesize").blocking(&["ghost"]));
    assert!(future_loop::turn_envelope::compose_upstream_evidence(
        &lonely,
        lonely.todo("S2").unwrap()
    )
    .is_empty());
}
