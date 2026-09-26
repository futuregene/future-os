//! Direct API drives for the store, runtime-index and webui read-model paths
//! that the CLI surface does not reach.
//!
//! These are public library APIs, so the tests call them the way an embedder
//! would: real registry + ledger on disk, real index files, real corruption.

use future_loop::state::{Goal, RunRecord, Todo};
use future_loop::store::{Event, Store};

fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn registered(root: &str, goal_id: &str) -> Store {
    let mut store = Store::open(root).unwrap();
    let mut goal = Goal::new(goal_id, "store api", ".");
    goal.add(Todo::advancement("t1", "work"));
    store.register(&goal).unwrap();
    store
}

/// The append path is the ledger's integrity boundary: the same `event_id` with
/// **different** content must fail closed (`StateEventConflictError`), because
/// that is how a replayed or corrupted writer is caught. Identical content is an
/// idempotent no-op instead.
#[test]
fn append_fails_closed_on_a_conflicting_event_id_and_skips_an_identical_one() {
    let dir = tmp();
    let root = dir.path().to_str().unwrap();
    let mut store = registered(root, "goal_conflict");
    let event = || Event::GoalStarted {
        goal_id: "goal_conflict".into(),
        ts: 1,
    };
    // First append under an explicit id.
    store
        .append_with_meta(
            event(),
            Some("evt-fixed".into()),
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    // Identical content under the same id: idempotent, no second row.
    let id = store
        .append_with_meta(
            event(),
            Some("evt-fixed".into()),
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    assert_eq!(id, "evt-fixed");
    let rows = store.raw_ledger_lines("goal_conflict").unwrap();
    assert_eq!(
        rows.len(),
        1,
        "an identical re-append must not duplicate: {rows:?}"
    );

    // Same id, DIFFERENT content: fail closed and leave the ledger untouched.
    let err = store
        .append_with_meta(
            Event::GoalStarted {
                goal_id: "goal_conflict".into(),
                ts: 999,
            },
            Some("evt-fixed".into()),
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("conflicting event_id"), "{msg}");
    assert_eq!(
        store.raw_ledger_lines("goal_conflict").unwrap().len(),
        1,
        "a refused append must not write"
    );
}

/// `append_todo_change` is the dependency-validating path: it must refuse a
/// non-todo event outright (the caller is validating todo edits) and validate the
/// dependency edges of a `TodoUpdated` that changes `blocks`.
#[test]
fn append_todo_change_refuses_non_todo_events_and_validates_edges() {
    let dir = tmp();
    let root = dir.path().to_str().unwrap();
    let mut store = registered(root, "goal_edges");

    // A goal-level event is not a todo change.
    let err = store
        .append_todo_change(Event::GoalStarted {
            goal_id: "goal_edges".into(),
            ts: 1,
        })
        .unwrap_err();
    assert!(
        format!("{err:#}").contains("expected a todo add/update event"),
        "{err:#}"
    );

    // A valid add, then an update whose `blocks` names a todo that does not
    // exist: the dependency guard must reject it.
    store
        .append_todo_change(Event::TodoAdded {
            goal_id: "goal_edges".into(),
            todo: Todo::advancement("t1", "first"),
            ts: 1,
        })
        .unwrap();
    let err = store
        .append_todo_change(Event::TodoUpdated {
            goal_id: "goal_edges".into(),
            todo_id: "t1".into(),
            text: None,
            status: None,
            evidence: None,
            note: None,
            priority: None,
            resume_when: None,
            blocks: Some(vec!["todo_ghost".into()]),
            acceptance: None,
            owner: None,
            ts: 2,
        })
        .unwrap_err();
    assert!(format!("{err:#}").contains("unknown todo"), "{err:#}");

    // A `TodoUpdated` that does NOT touch `blocks` skips the edge validation
    // (nothing to re-check) and still succeeds.
    let id = store
        .append_todo_change(Event::TodoUpdated {
            goal_id: "goal_edges".into(),
            todo_id: "t1".into(),
            text: Some("retitled".into()),
            status: None,
            evidence: None,
            note: None,
            priority: None,
            resume_when: None,
            blocks: None,
            acceptance: None,
            owner: None,
            ts: 3,
        })
        .unwrap();
    assert!(!id.is_empty());
    let goal = store.replay("goal_edges").unwrap().unwrap();
    assert_eq!(goal.todo("t1").unwrap().text, "retitled");
}

/// The active-state projections are plain files next to the ledger; both must be
/// written for the goal dir and survive a reopen (the read side is what the
/// dashboard and `status` consume).
#[test]
fn next_action_and_run_appends_land_in_the_goal_dir() {
    let dir = tmp();
    let root = dir.path().to_str().unwrap();
    let store = registered(root, "goal_proj");
    store
        .set_next_action("goal_proj", "finish the ledger work")
        .unwrap();
    let record = RunRecord {
        agent_id: Some("w1".into()),
        turn: 1,
        todo_id: "t1".into(),
        run_id: "run-p".into(),
        terminal_state: "completed".into(),
        error: None,
        tokens_in_delta: 3,
        tokens_out_delta: 4,
        cost_delta: 0.02,
        tools: vec!["shell".into()],
        evidence: "artifact written".into(),
        recorded_at: future_loop::state::now_epoch(),
        spend_source: Some("run".into()),
        validation: None,
        failure_kind: Some(future_loop::state::FailureKind::None),
        truncation: None,
    };
    store.append_run("goal_proj", &record).unwrap();
    store.append_run("goal_proj", &record).unwrap();

    let goal_dir = std::path::Path::new(root).join("goals").join("goal_proj");
    let next_action = std::fs::read_to_string(goal_dir.join("next_action.txt")).unwrap();
    assert_eq!(next_action, "finish the ledger work");
    let runs = std::fs::read_to_string(goal_dir.join("runs.jsonl")).unwrap();
    assert_eq!(
        runs.lines().filter(|l| !l.trim().is_empty()).count(),
        2,
        "both run appends must land: {runs}"
    );
    // A goal dir that does not exist yet is created on write.
    store.set_next_action("goal_proj", "second value").unwrap();
    assert_eq!(
        std::fs::read_to_string(goal_dir.join("next_action.txt")).unwrap(),
        "second value"
    );
}

/// Registering a goal is what persists the registry; the second registration of
/// the same id must be a no-op (not a duplicate row), and the file on disk must
/// be readable JSON that names the goal.
#[test]
fn register_persists_the_registry_once_per_goal() {
    let dir = tmp();
    let root = dir.path().to_str().unwrap();
    let mut store = Store::open(root).unwrap();
    let goal = Goal::new("goal_reg", "registry", ".");
    store.register(&goal).unwrap();
    assert!(store.registered("goal_reg"));
    // Idempotent: a second register must not append a second entry.
    store.register(&goal).unwrap();
    drop(store);

    let registry_path = std::path::Path::new(root).join("registry.json");
    let text = std::fs::read_to_string(&registry_path)
        .unwrap_or_else(|e| panic!("register must write {}: {e}", registry_path.display()));
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("registry must be JSON");
    let entries = parsed.as_array().expect("registry is an array");
    assert_eq!(entries.len(), 1, "one row per goal: {parsed}");
    assert_eq!(entries[0]["goal_id"], "goal_reg");

    // A reopened store sees the registration (the file is the source of truth).
    let reopened = Store::open(root).unwrap();
    assert!(
        reopened.registered("goal_reg"),
        "a reopened store must load its registry"
    );
}

/// The registry is the authority for appends: an unregistered goal must be
/// refused with the actionable hint rather than silently creating a ledger.
#[test]
fn appending_for_an_unregistered_goal_is_refused() {
    let dir = tmp();
    let root = dir.path().to_str().unwrap();
    let mut store = Store::open(root).unwrap();
    let err = store
        .append(Event::GoalStarted {
            goal_id: "goal_unknown".into(),
            ts: 1,
        })
        .unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("not registered"), "{msg}");
    assert!(
        !std::path::Path::new(root)
            .join("goals")
            .join("goal_unknown")
            .join("events.jsonl")
            .exists(),
        "a refused append must not create a ledger file"
    );

    // Same refusal on the dependency-validating append path.
    let err = store
        .append_todo_change(Event::TodoAdded {
            goal_id: "goal_unknown".into(),
            todo: Todo::advancement("t", "work"),
            ts: 1,
        })
        .unwrap_err();
    assert!(format!("{err:#}").contains("not registered"));
}

/// `append_todo_change` is the dependency-validating path the CLI uses for
/// todo edits; it must succeed and stamp the schema for a registered goal.
#[test]
fn append_todo_change_succeeds_and_returns_the_event_id() {
    let dir = tmp();
    let root = dir.path().to_str().unwrap();
    let mut store = registered(root, "goal_tc");
    let id = store
        .append_todo_change(Event::TodoAdded {
            goal_id: "goal_tc".into(),
            todo: Todo::advancement("t1", "first"),
            ts: 1,
        })
        .unwrap();
    assert!(!id.is_empty(), "an append must return its event id");
    // The event is replayable and the schema was stamped on the goal dir.
    assert_eq!(store.replay("goal_tc").unwrap().unwrap().todos.len(), 1);
    let goal_dir = std::path::Path::new(root).join("goals").join("goal_tc");
    assert!(goal_dir.join("events.jsonl").is_file());
    assert!(
        goal_dir.join("schema.json").exists(),
        "the append must stamp the schema: {:?}",
        std::fs::read_dir(&goal_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
    );

    // A second append for the same content is idempotent (content-derived id).
    let again = store
        .append_todo_change(Event::TodoAdded {
            goal_id: "goal_tc".into(),
            todo: Todo::advancement("t1", "first"),
            ts: 1,
        })
        .unwrap();
    assert_eq!(again, id, "the same content must derive the same event id");
}

/// The run index is a projection over run files. A missing index is not an
/// error (nothing recorded yet), and a rebuild reproduces rows from the files.
#[test]
fn run_index_reads_an_absent_index_as_empty_and_rebuilds_from_files() {
    use future_loop::runtime::run_index::{detect_duplicates, load_run_index, rebuild_index};
    let dir = tmp();
    let root = dir.path().to_str().unwrap();
    let mut store = registered(root, "goal_idx");
    let _ = store
        .append_todo_change(Event::TodoAdded {
            goal_id: "goal_idx".into(),
            todo: Todo::advancement("t1", "work"),
            ts: 1,
        })
        .unwrap();

    // Nothing recorded yet: an absent index reads as empty, not as an error.
    let rows = load_run_index(root, "goal_idx").unwrap();
    assert!(rows.is_empty(), "{rows:?}");

    // Seed a real run file, then rebuild the index from it.
    let record = RunRecord {
        agent_id: Some("w1".into()),
        turn: 1,
        todo_id: "t1".into(),
        run_id: "run-a".into(),
        terminal_state: "completed".into(),
        error: None,
        tokens_in_delta: 1,
        tokens_out_delta: 2,
        cost_delta: 0.01,
        tools: vec!["shell".into()],
        evidence: "artifact written".into(),
        recorded_at: 1_700_000_000,
        spend_source: Some("run".into()),
        validation: None,
        failure_kind: Some(future_loop::state::FailureKind::None),
        truncation: None,
    };
    future_loop::compat::write_run(
        &std::path::Path::new(root).join("goals").join("goal_idx"),
        "goal_idx",
        &record,
    )
    .unwrap();

    let report = rebuild_index(root, "goal_idx").unwrap();
    assert!(report.rows_written >= 1, "{report:?}");
    let rows = load_run_index(root, "goal_idx").unwrap();
    assert_eq!(
        rows.len(),
        1,
        "the rebuild must index the run file: {rows:?}"
    );
    assert_eq!(rows[0].goal_id, "goal_idx");
    assert_eq!(rows[0].turn, 1);
    assert_eq!(rows[0].classification, "completed");

    // Duplicate detection over a healthy index reports no duplicate groups.
    let report = detect_duplicates(&std::path::Path::new(root).join("run_index.jsonl")).unwrap();
    assert!(report.duplicate_groups.is_empty(), "{report:?}");
}

/// `build_run_history` buckets the index rows into windows; an unregistered goal
/// has no history at all (not an error), which is what `history --goal` relies on.
#[test]
fn run_history_is_none_for_an_unknown_goal_and_projects_a_known_one() {
    use future_loop::runtime::run_history::build_run_history;
    let dir = tmp();
    let root = dir.path().to_str().unwrap();
    let _store = registered(root, "goal_hist");
    // Unknown goal: nothing to project.
    assert!(build_run_history(root, "goal_not_there", 1_800_000_000)
        .unwrap()
        .is_none());
    // A registered goal with no runs indexed projects nothing — the projection is
    // driven by the run index, so it needs a run file to exist at all.
    assert!(
        build_run_history(root, "goal_hist", 1_800_000_000)
            .unwrap()
            .is_none(),
        "no indexed runs means no projection"
    );

    // With a run file present the projection appears and carries its goal id.
    let record = RunRecord {
        agent_id: Some("w1".into()),
        turn: 1,
        todo_id: "t1".into(),
        run_id: "run-h".into(),
        terminal_state: "completed".into(),
        error: None,
        tokens_in_delta: 1,
        tokens_out_delta: 2,
        cost_delta: 0.01,
        tools: vec!["shell".into()],
        evidence: "artifact written".into(),
        recorded_at: future_loop::state::now_epoch(),
        spend_source: Some("run".into()),
        validation: None,
        failure_kind: Some(future_loop::state::FailureKind::None),
        truncation: None,
    };
    future_loop::compat::write_run(
        &std::path::Path::new(root).join("goals").join("goal_hist"),
        "goal_hist",
        &record,
    )
    .unwrap();
    let projection = build_run_history(root, "goal_hist", future_loop::state::now_epoch())
        .unwrap()
        .expect("a goal with an indexed run must project");
    assert_eq!(projection.goal_id, "goal_hist");
    assert!(
        projection.sample_run_count >= 1,
        "the run must be sampled: {projection:?}"
    );
}

/// The raw ledger is the operator's escape hatch when the projection cannot
/// parse a line: `raw_ledger_lines` must return the file verbatim (corrupt line
/// included) rather than failing, so a broken ledger can still be inspected.
#[test]
fn raw_ledger_lines_returns_a_corrupt_line_verbatim() {
    let dir = tmp();
    let root = dir.path().to_str().unwrap();
    let mut store = registered(root, "goal_raw");
    store
        .append(Event::GoalStarted {
            goal_id: "goal_raw".into(),
            ts: 1,
        })
        .unwrap();
    drop(store);

    let ledger = std::path::Path::new(root)
        .join("goals")
        .join("goal_raw")
        .join("events.jsonl");
    let mut text = std::fs::read_to_string(&ledger).unwrap();
    text.push_str("this line is not json\n");
    std::fs::write(&ledger, text).unwrap();

    let store = Store::open(root).unwrap();
    let raw = store.raw_ledger_lines("goal_raw").unwrap();
    assert!(
        raw.iter().any(|l| l.contains("not json")),
        "the corrupt line must be returned verbatim: {raw:?}"
    );
    assert!(
        raw.iter().any(|l| l.contains("goal_started")),
        "the valid lines must come back too: {raw:?}"
    );
    // An unknown goal has no ledger: empty, not an error.
    let absent = store.raw_ledger_lines("goal_missing").unwrap();
    assert!(absent.is_empty(), "{absent:?}");
}
