//! P1 contract tests — G-10 scheduler state machine: rrule recurrence,
//! progression walk, restart-safe persistence, and backup/restore carrying
//! the scheduler-state directory (P1 acceptance: progression survives
//! restarts; the new persisted file participates in backup/restore).

use std::path::Path;

use future_loop::scheduler::state::*;
use future_loop::store::Store;

fn tmp_root(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("future-loop-sched-test-{tag}-{}", uuid_like()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    format!(
        "{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}

const GOAL: &str = "g1";
const AGENT: &str = "codex-agent";
const KEY: &str = CODEX_APP_STATEFUL_BACKOFF_STATE_KEY;

fn bootstrap_state(goal_dir: &Path) -> SchedulerState {
    let identity = identity_signature(GOAL, AGENT, CODEX_APP_SURFACE);
    let initial = rrule_for_minutes(MONITOR_WAIT_PROGRESSION_MINUTES[0]);
    let state = build_scheduler_state(
        GOAL,
        AGENT,
        CODEX_APP_SURFACE,
        KEY,
        &reset_token("tick_next", &identity, &initial),
        &identity,
        0,
        MONITOR_WAIT_PROGRESSION_MINUTES.to_vec(),
        &initial,
        1_784_000_000,
        vec![],
    )
    .unwrap();
    write_scheduler_state(goal_dir, &state).unwrap();
    state
}

// ── rrule recurrence + cadence classes ─────────────────────────────────────
#[test]
fn rrule_helpers_match_future_loop_contract() {
    assert_eq!(rrule_for_minutes(15), "FREQ=MINUTELY;INTERVAL=15");
    assert_eq!(
        normalize_scheduler_rrule("RRULE:FREQ=MINUTELY;INTERVAL=30"),
        "FREQ=MINUTELY;INTERVAL=30"
    );
    assert_eq!(
        scheduler_rrule_interval_minutes("FREQ=MINUTELY;INTERVAL=1440"),
        Some(1440)
    );
    assert_eq!(scheduler_rrule_interval_minutes("FREQ=DAILY"), None);
    // Cadence-class subset (plan §5.2 trade-off).
    assert_eq!(
        rrule_for_cadence_class("hourly").as_deref(),
        Some("FREQ=MINUTELY;INTERVAL=60")
    );
    assert_eq!(
        rrule_for_cadence_class("daily").as_deref(),
        Some("FREQ=MINUTELY;INTERVAL=1440")
    );
    assert_eq!(
        rrule_for_cadence_class("weekly").as_deref(),
        Some("FREQ=MINUTELY;INTERVAL=10080")
    );
    assert_eq!(rrule_for_cadence_class("once"), None);
}

// ── progression walks the backoff sequence, wrapping at the end ────────────
#[test]
fn progression_walks_monitor_wait_sequence() {
    let dir = std::env::temp_dir().join("progression-walk");
    std::fs::create_dir_all(&dir).unwrap();
    let mut state = bootstrap_state(&dir);
    assert_eq!(current_progression_minutes(&state), Some(15));
    let r2 = apply_next_progression(&mut state, 1_784_000_100).unwrap();
    assert_eq!(r2, "FREQ=MINUTELY;INTERVAL=30");
    let r3 = apply_next_progression(&mut state, 1_784_000_200).unwrap();
    assert_eq!(r3, "FREQ=MINUTELY;INTERVAL=60");
    // Wrap: [15, 30, 60] → back to 15 (reference modulo progression).
    let r1 = apply_next_progression(&mut state, 1_784_000_300).unwrap();
    assert_eq!(r1, "FREQ=MINUTELY;INTERVAL=15");
}

// ── P1 acceptance: progression persists across a "restart" ────────────────
#[test]
fn progression_survives_restart_via_disk() {
    let dir = std::env::temp_dir().join("sched-restart");
    std::fs::create_dir_all(&dir).unwrap();
    let mut state = bootstrap_state(&dir);

    // Cycle 1 advances 15 → 30.
    apply_next_progression(&mut state, 1_784_000_100);
    write_scheduler_state(&dir, &state).unwrap();

    // "Restart": a fresh process reads the persisted file — cursor did NOT reset.
    let reloaded = load_scheduler_state(&dir, AGENT, CODEX_APP_SURFACE, KEY).unwrap();
    assert_eq!(reloaded.progression_index, 1);
    assert_eq!(reloaded.last_applied_rrule, "FREQ=MINUTELY;INTERVAL=30");
    assert_eq!(current_progression_minutes(&reloaded), Some(30));

    // A second tick after restart advances 30 → 60.
    let mut again = reloaded;
    let r = apply_next_progression(&mut again, 1_784_000_200).unwrap();
    assert_eq!(r, "FREQ=MINUTELY;INTERVAL=60");
}

// ── identity signature: scope-stable, agent-specific ───────────────────────
#[test]
fn identity_signature_is_scope_stable_and_agent_specific() {
    let a = identity_signature(GOAL, AGENT, CODEX_APP_SURFACE);
    assert_eq!(a, identity_signature(GOAL, AGENT, CODEX_APP_SURFACE));
    assert_ne!(a, identity_signature(GOAL, "other", CODEX_APP_SURFACE));
    assert_ne!(
        a,
        identity_signature("other-goal", AGENT, CODEX_APP_SURFACE)
    );
    assert_eq!(a.len(), 12);
}

// ── host update failures: bounded cache + TTL retention ────────────────────
#[test]
fn host_update_failures_are_deduped_capped_and_ttl_dropped() {
    let now = parse_epoch("2026-08-05T12:00:00+00:00").unwrap();
    let mk = |kind: &str, at: &str| HostUpdateFailure {
        schema_version: SCHEDULER_HOST_UPDATE_FAILURE_SCHEMA_VERSION.to_string(),
        target_rrule: "FREQ=MINUTELY;INTERVAL=15".to_string(),
        observed_host_rrule: "FREQ=MINUTELY;INTERVAL=1440".to_string(),
        failure_kind: kind.to_string(),
        failed_at: at.to_string(),
        failure_count: 1,
    };
    let mut failures = vec![mk("host_stale_rrule", "2026-08-05T12:00:00+00:00")];
    // Same (target, observed) pair: latest wins, no growth.
    failures = merge_host_update_failure(
        &failures,
        mk("host_stale_rrule", "2026-08-05T13:00:00+00:00"),
        now,
    );
    assert_eq!(failures.len(), 1);
    // Distinct kinds accumulate but stay inside the cache limit.
    for (i, kind) in ["timeout", "rejected", "drift", "conflict", "overflow"]
        .iter()
        .enumerate()
    {
        failures = merge_host_update_failure(
            &failures,
            mk(kind, &format!("2026-08-05T{:02}:00:00+00:00", 14 + i)),
            now,
        );
    }
    assert!(failures.len() <= SCHEDULER_HOST_UPDATE_FAILURE_CACHE_LIMIT);
    assert_eq!(failures.last().unwrap().failure_kind, "overflow");
    // Stale (>24h) failures drop out of retention.
    let stale = mk("stale", "2026-08-01T12:00:00+00:00");
    let retained = retained_host_update_failures(&[stale], now, None);
    assert!(retained.is_empty());
}

// ── P1 risk: backup/restore carries scheduler-state (no progression reset) ─
#[test]
fn backup_and_restore_carry_scheduler_state() {
    let root = tmp_root("backup");
    let mut store = Store::open(&root).unwrap();
    let goal = future_loop::state::Goal::new(GOAL, "objective", "/tmp");
    store.register(&goal).unwrap();
    store
        .append(future_loop::store::Event::GoalStarted {
            goal_id: GOAL.into(),
            ts: goal.created_at,
        })
        .unwrap();

    // Persist a scheduler state with advanced progression (index=1).
    let goal_dir = store.goal_dir(GOAL);
    let mut state = bootstrap_state(&goal_dir);
    apply_next_progression(&mut state, 1_784_000_100);
    write_scheduler_state(&goal_dir, &state).unwrap();

    // Backup → restore into a fresh store rooted at the same place.
    let backup = store.backup_goal(GOAL).unwrap();
    let store2 = Store::open(&root).unwrap();
    store2.restore_goal(GOAL, &backup).unwrap();

    // The scheduler-state dir survived restore; progression did not reset.
    let reloaded = load_scheduler_state(&store2.goal_dir(GOAL), AGENT, CODEX_APP_SURFACE, KEY)
        .expect("scheduler state restored");
    assert_eq!(reloaded.progression_index, 1);
    assert_eq!(reloaded.last_applied_rrule, "FREQ=MINUTELY;INTERVAL=30");
}

// ── normalization rejects scope mismatch / missing fields ──────────────────
#[test]
fn normalize_rejects_bad_scope_and_missing_fields() {
    let dir = std::env::temp_dir().join("sched-norm");
    std::fs::create_dir_all(&dir).unwrap();
    let state = bootstrap_state(&dir);
    assert!(normalize_scheduler_state(&state, GOAL, AGENT, CODEX_APP_SURFACE, KEY).is_some());
    assert!(normalize_scheduler_state(&state, "other", AGENT, CODEX_APP_SURFACE, KEY).is_none());
    let mut broken = state.clone();
    broken.progression_minutes = vec![0, -5];
    assert!(normalize_scheduler_state(&broken, GOAL, AGENT, CODEX_APP_SURFACE, KEY).is_none());
}
