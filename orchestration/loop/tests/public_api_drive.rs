//! Drives for three public APIs the CLI happens to bypass.
//!
//! Each of these is real, exported API surface that no existing test calls:
//! - `canary::seed_premerge_fixture` + `canary::run_premerge_gate_in` (the gate
//!   CI runs, but only ever through the `--isolated` wrapper that makes its own
//!   temp root, so the in-place variant is never exercised),
//! - `scheduler::state::normalize_host_update_failure`'s SUCCESS path (only the
//!   rejection branches were covered),
//! - `Store::try_claim_todo_with_workspace` called directly (the CLI reaches it
//!   through argument parsing, so its own contract — TTL normalization, the
//!   unregistered-goal refusal, the read-under-lock fold — is untested).
//!
//! All state lives in the harness's own tempdir (`cli_root`), never the
//! operator's live root.

mod common;

use common::{cli_root, init_goal, open_store};
use future_loop::store::{Event, Store};

/// The release gate's `root_writable` smoke check must FAIL and say so when the
/// state root cannot be written. The passing branch is covered by every other
/// gate test; this is the branch that actually matters operationally — a gate
/// that cannot write its probe must not report `writable`.
///
/// The fixture only ever replaces contents of the harness's own uniquely
/// suffixed tempdir (`cli_root`), never a user path.
#[test]
fn release_gate_reports_a_non_writable_state_root() {
    use future_loop::canary::run_release_gate;
    let cr = cli_root();
    let mut store = Store::open(&cr.root).unwrap();
    store
        .register(&future_loop::state::Goal::new("g1", "objective", "."))
        .unwrap();

    // Sanity: while the root is a real directory the check passes.
    let healthy = run_release_gate(&store).unwrap();
    let ok = healthy
        .checks
        .iter()
        .find(|c| c.id == "root_writable")
        .expect("the release gate must run the root_writable check");
    assert!(ok.passed, "a writable root must pass: {ok:?}");
    assert!(ok.detail.contains("writable"), "{ok:?}");
    assert!(!ok.detail.contains("NOT writable"), "{ok:?}");

    // Now make the root unwritable by replacing the directory with a file of the
    // same name: `fs::write(root/.smoke-probe-N)` then fails cross-platform.
    std::fs::remove_dir_all(&cr.root).unwrap();
    std::fs::write(&cr.root, b"not a directory").unwrap();

    let broken = run_release_gate(&store).unwrap();
    let bad = broken
        .checks
        .iter()
        .find(|c| c.id == "root_writable")
        .expect("the check must still be reported");
    assert!(
        !bad.passed,
        "an unwritable root must FAIL the check, not pass quietly: {bad:?}"
    );
    assert!(
        bad.detail.contains("NOT writable"),
        "the failure must say the root is not writable: {bad:?}"
    );
    assert!(
        !broken.all_passed,
        "the run cannot claim all-passed: {broken:?}"
    );
}

/// The premerge gate seeds its own fixture and must come back non-vacuous: the
/// whole point of the gate is that it FAILS when the project is broken, so a
/// fixture that produced zero checks would always pass and prove nothing.
#[test]
fn premerge_gate_seeds_a_real_fixture_and_reports_its_checks() {
    use future_loop::canary::{run_premerge_gate_in, seed_premerge_fixture};
    let cr = cli_root();

    let mut store = Store::open(&cr.root).unwrap();
    let fixture_id = seed_premerge_fixture(&mut store).unwrap();
    // The fixture is a real registered goal with a started ledger and one todo.
    assert!(
        store.registered(&fixture_id),
        "the fixture goal must register"
    );
    let goal = store.replay(&fixture_id).unwrap().unwrap();
    assert_eq!(goal.todos.len(), 1, "one fixture work item: {goal:?}");
    assert_eq!(goal.todos[0].id, "T1");
    assert_eq!(goal.todos[0].status, future_loop::state::TodoStatus::Open);

    // Re-seeding is idempotent: a second call must not duplicate the todo (the
    // gate may run more than once against the same root).
    seed_premerge_fixture(&mut store).unwrap();
    let again = store.replay(&fixture_id).unwrap().unwrap();
    assert_eq!(
        again.todos.len(),
        1,
        "re-seeding must not duplicate the fixture: {again:?}"
    );

    // The gate runs the real smoke checks over that root.
    let report = run_premerge_gate_in(&cr.root).unwrap();
    assert!(
        !report.schema_version.is_empty(),
        "the report must self-describe: {report:?}"
    );
    assert_eq!(report.run.profile_id, "premerge");
    assert!(
        !report.run.checks.is_empty(),
        "a vacuous gate is worthless - the fixture must produce checks: {report:?}"
    );
    // Every check carries an identity and a pass/fail verdict.
    for check in &report.run.checks {
        assert!(!check.module.is_empty(), "{check:?}");
        assert!(!check.id.is_empty(), "{check:?}");
    }
    // The gate decision is derived from those checks, so a healthy fixture and
    // an all-passing check set must agree.
    let all_passed = report.run.checks.iter().all(|c| c.passed);
    assert_eq!(
        report.gate.passed, all_passed,
        "the gate verdict must follow its checks: {report:?}"
    );
}

/// `normalize_host_update_failure` normalizes a raw record: the rrule fields are
/// canonicalized (a leading `RRULE:` and inner whitespace collapse), the text
/// fields trimmed, and a record missing any required field — or carrying a
/// stale `schema_version` — is rejected as `None` rather than half-populated.
#[test]
fn host_update_failure_normalizes_a_complete_record_and_rejects_the_rest() {
    use future_loop::scheduler::state::{
        normalize_host_update_failure, SCHEDULER_HOST_UPDATE_FAILURE_SCHEMA_VERSION,
    };
    let valid = serde_json::json!({
        "schema_version": SCHEDULER_HOST_UPDATE_FAILURE_SCHEMA_VERSION,
        "target_rrule": "  RRULE:FREQ=MINUTELY;INTERVAL=30  ",
        "observed_host_rrule": " RRULE:FREQ=MINUTELY;INTERVAL=1440 ",
        "failure_kind": "  host_stale_rrule  ",
        "failed_at": "  2026-08-05T12:00:00+00:00  ",
        "failure_count": 2,
    });
    let failure = normalize_host_update_failure(&valid).expect("a complete record must normalize");
    assert_eq!(
        failure.target_rrule, "FREQ=MINUTELY;INTERVAL=30",
        "the RRULE: prefix must be stripped and whitespace collapsed"
    );
    assert_eq!(failure.observed_host_rrule, "FREQ=MINUTELY;INTERVAL=1440");
    assert_eq!(failure.failure_kind, "host_stale_rrule", "trimmed");
    assert_eq!(failure.failed_at, "2026-08-05T12:00:00+00:00", "trimmed");
    assert_eq!(failure.failure_count, 2);
    assert_eq!(
        failure.schema_version,
        SCHEDULER_HOST_UPDATE_FAILURE_SCHEMA_VERSION
    );

    // A missing `observed_host_rrule` is tolerated (it defaults to empty), which
    // is the one optional field.
    let mut no_observed = valid.clone();
    no_observed
        .as_object_mut()
        .unwrap()
        .remove("observed_host_rrule");
    assert!(normalize_host_update_failure(&no_observed).is_some());

    // Every other required field is enforced, one at a time.
    for field in [
        "target_rrule",
        "failure_kind",
        "failed_at",
        "failure_count",
        "schema_version",
    ] {
        let mut broken = valid.clone();
        broken.as_object_mut().unwrap().remove(field);
        assert!(
            normalize_host_update_failure(&broken).is_none(),
            "a record missing `{field}` must be rejected: {broken}"
        );
    }
    // A stale schema version is rejected even when every other field is present.
    let mut stale = valid.clone();
    stale["schema_version"] = serde_json::json!("scheduler_host_update_failure_v0_stale");
    assert!(normalize_host_update_failure(&stale).is_none());
    // Blank text and a zero count are as invalid as a missing field.
    for (field, value) in [
        ("target_rrule", serde_json::json!("   ")),
        ("failure_kind", serde_json::json!("")),
        ("failed_at", serde_json::json!("  ")),
        ("failure_count", serde_json::json!(0)),
    ] {
        let mut blank = valid.clone();
        blank[field] = value;
        assert!(
            normalize_host_update_failure(&blank).is_none(),
            "a blank/zero `{field}` must be rejected: {blank}"
        );
    }
    // Non-objects and wrong types are rejected rather than panicking.
    assert!(normalize_host_update_failure(&serde_json::json!("nope")).is_none());
    assert!(normalize_host_update_failure(&serde_json::json!([1, 2])).is_none());
}

/// `Store::try_claim_todo_with_workspace` is the atomic claim the CLI depends on.
/// Its own contract: it normalizes the TTL, refuses an unregistered goal, folds
/// the ledger under the lock, and reports whether the claim was taken.
#[test]
fn atomic_claim_normalizes_ttl_refuses_unregistered_and_records_the_claim() {
    let cr = cli_root();
    let gid = init_goal(&cr, "atomic claim api");
    let mut store = open_store(&cr);
    let todo = store.replay(&gid).unwrap().unwrap().todos[0].id.clone();

    // An out-of-range TTL is refused by the shared rule before any write.
    let too_long = future_loop::work_items::task_lease::MAX_TASK_LEASE_TTL_SECONDS + 1;
    let err = store
        .try_claim_todo_with_workspace(&gid, &todo, "a1", too_long, None, false)
        .map(|_| ())
        .expect_err("an over-ceiling TTL must be refused");
    assert!(format!("{err:#}").contains("between 1 and"), "{err:#}");

    // A TTL of 0 means "the default", not "already expired".
    let before = future_loop::state::now_epoch();
    let outcome = store
        .try_claim_todo_with_workspace(&gid, &todo, "a1", 0, Some(std::process::id()), false)
        .unwrap();
    assert!(outcome.claimed, "the atomic claim must be taken");
    let claimed = store.replay(&gid).unwrap().unwrap();
    let t = claimed.todo(&todo).unwrap();
    assert_eq!(t.claimed_by.as_deref(), Some("a1"));
    assert!(
        t.lease_expires_at.unwrap() > before,
        "TTL 0 must mint a live lease: {t:?}"
    );
    assert_eq!(
        t.holder_pid,
        Some(std::process::id()),
        "the holder pid must be recorded for liveness"
    );

    // The same owner re-claiming is idempotent, not a second event storm.
    let again = store
        .try_claim_todo_with_workspace(&gid, &todo, "a1", 60, None, false)
        .unwrap();
    assert!(again.claimed, "an idempotent re-claim must report claimed");

    // An unregistered goal is refused with the actionable hint.
    let err = store
        .try_claim_todo_with_workspace("goal_not_registered", "t1", "a1", 60, None, false)
        .map(|_| ())
        .expect_err("an unregistered goal must be refused");
    assert!(format!("{err:#}").contains("not registered"), "{err:#}");

    // An unknown todo in a registered goal is refused too.
    let err = store
        .try_claim_todo_with_workspace(&gid, "todo_ghost", "a1", 60, None, false)
        .map(|_| ())
        .expect_err("an unknown todo must be refused");
    assert!(format!("{err:#}").contains("unknown todo"), "{err:#}");

    // A completed todo cannot be claimed.
    store
        .append(Event::TodoCompleted {
            goal_id: gid.clone(),
            todo_id: todo.clone(),
            no_follow_up: true,
            successor_ids: vec![],
            evidence: Some("done".into()),
            ts: future_loop::state::now_epoch(),
        })
        .unwrap();
    let done = store
        .try_claim_todo_with_workspace(&gid, &todo, "a2", 60, None, false)
        .unwrap();
    assert!(!done.claimed, "a terminal todo must not be claimable");
}
