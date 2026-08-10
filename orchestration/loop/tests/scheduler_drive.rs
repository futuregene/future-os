//! Coverage drive for `scheduler/state.rs` — pure-function branch matrix
//! (rrule parsing, cadence classes, host-update-failure normalization /
//! retention / merge, scheduler-state validation, persistence).

use future_loop::scheduler::state as st;
use future_loop::scheduler::state::HostUpdateFailure;

fn failure(target: &str, observed: &str, failed_at: &str, count: u64) -> HostUpdateFailure {
    HostUpdateFailure {
        schema_version: st::SCHEDULER_HOST_UPDATE_FAILURE_SCHEMA_VERSION.to_string(),
        target_rrule: target.to_string(),
        observed_host_rrule: observed.to_string(),
        failure_kind: "host_stale_rrule".to_string(),
        failed_at: failed_at.to_string(),
        failure_count: count as u32,
    }
}

fn failure_json(target: &str, observed: &str, failed_at: &str, count: u64) -> serde_json::Value {
    serde_json::json!({
        "schema_version": st::SCHEDULER_HOST_UPDATE_FAILURE_SCHEMA_VERSION,
        "target_rrule": target,
        "observed_host_rrule": observed,
        "failure_kind": "host_stale_rrule",
        "failed_at": failed_at,
        "failure_count": count,
    })
}

#[test]
fn rrule_parsing_matrix() {
    // normalize_scheduler_rrule: collapses whitespace runs and strips an
    // RRULE: prefix (it does NOT reorder parts or change case).
    assert_eq!(
        st::normalize_scheduler_rrule("  FREQ=MINUTELY;INTERVAL=15  "),
        "FREQ=MINUTELY;INTERVAL=15"
    );
    assert_eq!(
        st::normalize_scheduler_rrule("RRULE:FREQ=MINUTELY;INTERVAL=15"),
        "FREQ=MINUTELY;INTERVAL=15"
    );
    // interval extraction.
    assert_eq!(
        st::scheduler_rrule_interval_minutes("FREQ=MINUTELY;INTERVAL=15"),
        Some(15)
    );
    assert_eq!(
        st::scheduler_rrule_interval_minutes("FREQ=HOURLY;INTERVAL=15"),
        None,
        "non-MINUTELY"
    );
    assert_eq!(
        st::scheduler_rrule_interval_minutes("FREQ=MINUTELY;INTERVAL=0"),
        None,
        "zero interval"
    );
    assert_eq!(
        st::scheduler_rrule_interval_minutes("FREQ=MINUTELY;INTERVAL=x"),
        None,
        "unparseable interval"
    );
    assert_eq!(st::scheduler_rrule_interval_minutes("no-separator"), None);
    // rrule_for_minutes / cadence_label.
    assert_eq!(st::rrule_for_minutes(90), "FREQ=MINUTELY;INTERVAL=90");
    assert_eq!(st::cadence_label(7 * 24 * 60), "1w");
    assert_eq!(st::cadence_label(48 * 60), "2d");
    assert_eq!(st::cadence_label(120), "2h");
    assert_eq!(st::cadence_label(45), "45m");
}

#[test]
fn cadence_class_matrix() {
    for (input, minutes) in [
        ("hourly", 60),
        ("hour", 60),
        ("1h", 60),
        ("daily", 24 * 60),
        ("day", 24 * 60),
        ("1d", 24 * 60),
        ("weekly", 7 * 24 * 60),
        ("week", 7 * 24 * 60),
    ] {
        let rrule = st::rrule_for_cadence_class(input).unwrap();
        assert_eq!(
            st::scheduler_rrule_interval_minutes(&rrule),
            Some(minutes),
            "{input}"
        );
    }
    for none in ["once", "", "none", "biweekly"] {
        assert!(st::rrule_for_cadence_class(none).is_none(), "{none}");
    }
    // A raw rrule passes through.
    assert_eq!(
        st::rrule_for_cadence_class("FREQ=MINUTELY;INTERVAL=7").as_deref(),
        Some("FREQ=MINUTELY;INTERVAL=7")
    );
}

#[test]
fn monitor_cadence_secs_matrix() {
    assert_eq!(st::monitor_cadence_secs(""), None);
    assert_eq!(st::monitor_cadence_secs("15m"), Some(900));
    assert_eq!(st::monitor_cadence_secs("30s"), Some(30));
    assert_eq!(st::monitor_cadence_secs("2h"), Some(7200));
    assert_eq!(st::monitor_cadence_secs("1d"), Some(86400));
    assert_eq!(st::monitor_cadence_secs("0m"), None);
    assert_eq!(st::monitor_cadence_secs("123"), None, "no unit separator");
    assert_eq!(st::monitor_cadence_secs("xm"), None, "bad count");
    assert_eq!(st::monitor_cadence_secs("5w"), None, "unknown unit");
}

#[test]
fn host_update_failure_normalization() {
    // Non-object → None.
    assert!(st::normalize_host_update_failure(&serde_json::json!("x")).is_none());
    // Wrong schema → None.
    assert!(st::normalize_host_update_failure(&serde_json::json!({"schema_version": "nope"}))
        .is_none());
    // Missing fields → None.
    let mut v = failure_json("FREQ=MINUTELY;INTERVAL=15", "", "2026-08-10T00:00:00+00:00", 1);
    v.as_object_mut().unwrap().remove("failure_kind");
    assert!(st::normalize_host_update_failure(&v).is_none());
    // Zero count → None.
    let v = failure_json("FREQ=MINUTELY;INTERVAL=15", "", "2026-08-10T00:00:00+00:00", 0);
    assert!(st::normalize_host_update_failure(&v).is_none());
    // Valid; missing observed_rrule defaults to "".
    let v = failure_json("FREQ=MINUTELY;INTERVAL=15", "", "2026-08-10T00:00:00+00:00", 2);
    let f = st::normalize_host_update_failure(&v).unwrap();
    assert_eq!(f.failure_count, 2);
    // Dedup by pair keeps the latest; cache limit caps the list.
    let now = "2026-08-10T00:00:00+00:00";
    let mut list: Vec<serde_json::Value> = vec![];
    for i in 0..10 {
        list.push(failure_json("FREQ=MINUTELY;INTERVAL=15", "A", now, i + 1));
    }
    list.push(failure_json("FREQ=MINUTELY;INTERVAL=30", "B", now, 1));
    let normalized = st::normalize_host_update_failures(&list);
    assert_eq!(normalized.len(), 2, "same-pair dedup");
    let a = normalized
        .iter()
        .find(|f| f.observed_host_rrule == "A")
        .unwrap();
    assert_eq!(a.failure_count, 10, "latest wins");
}

#[test]
fn retained_failures_ttl_and_observed_filter() {
    let now = 1_800_000_000u64;
    let fresh = chrono::DateTime::from_timestamp((now - 60) as i64, 0)
        .unwrap()
        .to_rfc3339();
    let stale = chrono::DateTime::from_timestamp(1_000_000, 0)
        .unwrap()
        .to_rfc3339();
    let failures = vec![
        failure("T", "OBS", &fresh, 1),
        failure("T2", "OBS2", &stale, 1),   // TTL-expired
        failure("T3", "OTHER", &fresh, 1),  // different observed rrule
    ];
    // No observed filter: TTL only.
    let kept = st::retained_host_update_failures(&failures, now, None);
    assert_eq!(kept.len(), 2);
    // With observed filter: only the matching fresh one.
    let kept = st::retained_host_update_failures(&failures, now, Some("OBS"));
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].target_rrule, "T");
    // Unparseable failed_at parses as 0 → TTL-expired.
    let weird = vec![failure("T", "OBS", "not-a-date", 1)];
    assert!(st::retained_host_update_failures(&weird, now, None).is_empty());
}

#[test]
fn merge_host_update_failure_replaces_pair() {
    let now = 1_800_000_000u64;
    let ts = chrono::DateTime::from_timestamp(now as i64, 0).unwrap().to_rfc3339();
    let existing = vec![failure("T", "OBS", &ts, 1)];
    let merged = st::merge_host_update_failure(&existing, failure("T", "OBS", &ts, 5), now);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].failure_count, 5, "latest replaces the pair");
    // Merging retains only failures matching the NEW failure's observed
    // rrule (the retained-set filter), so a new observed rrule drops others.
    let merged = st::merge_host_update_failure(&existing, failure("T", "NEW", &ts, 1), now);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].observed_host_rrule, "NEW");
}

#[test]
fn parse_epoch_variants() {
    assert!(st::parse_epoch("2026-08-10T00:00:00+00:00").unwrap() > 0);
    assert!(st::parse_epoch("garbage").is_none());
    // Pre-epoch timestamps clamp to 0.
    assert_eq!(st::parse_epoch("1900-01-01T00:00:00+00:00"), Some(0));
}

#[test]
fn scheduler_state_validation_matrix() {
    let good = st::build_scheduler_state(
        "g1",
        "a1",
        st::CODEX_APP_SURFACE,
        st::CODEX_APP_STATEFUL_BACKOFF_STATE_KEY,
        "reset-token",
        "identity",
        0,
        vec![15, 30],
        "FREQ=MINUTELY;INTERVAL=15",
        1_800_000_000,
        vec![],
    )
    .unwrap();
    // Scope mismatches → None.
    assert!(st::normalize_scheduler_state(&good, "OTHER", "a1", st::CODEX_APP_SURFACE, st::CODEX_APP_STATEFUL_BACKOFF_STATE_KEY).is_none());
    assert!(st::normalize_scheduler_state(&good, "g1", "OTHER", st::CODEX_APP_SURFACE, st::CODEX_APP_STATEFUL_BACKOFF_STATE_KEY).is_none());
    assert!(st::normalize_scheduler_state(&good, "g1", "a1", "OTHER", st::CODEX_APP_STATEFUL_BACKOFF_STATE_KEY).is_none());
    assert!(st::normalize_scheduler_state(&good, "g1", "a1", st::CODEX_APP_SURFACE, "OTHER").is_none());
    // Bad schema → None; legacy schema token accepted.
    let mut s = good.clone();
    s.schema_version = "nope".into();
    assert!(st::normalize_scheduler_state(&s, "g1", "a1", st::CODEX_APP_SURFACE, st::CODEX_APP_STATEFUL_BACKOFF_STATE_KEY).is_none());
    let mut s = good.clone();
    s.schema_version = "loopx_scheduler_state_v0".into();
    assert!(st::normalize_scheduler_state(&s, "g1", "a1", st::CODEX_APP_SURFACE, st::CODEX_APP_STATEFUL_BACKOFF_STATE_KEY).is_some());
    // Empty required fields → None.
    let mut s = good.clone();
    s.reset_token = "  ".into();
    assert!(st::normalize_scheduler_state(&s, "g1", "a1", st::CODEX_APP_SURFACE, st::CODEX_APP_STATEFUL_BACKOFF_STATE_KEY).is_none());
    let mut s = good.clone();
    s.identity_signature = String::new();
    assert!(st::normalize_scheduler_state(&s, "g1", "a1", st::CODEX_APP_SURFACE, st::CODEX_APP_STATEFUL_BACKOFF_STATE_KEY).is_none());
    // Empty rrule with no failures → None; with failures → Some.
    let mut s = good.clone();
    s.last_applied_rrule = String::new();
    assert!(st::normalize_scheduler_state(&s, "g1", "a1", st::CODEX_APP_SURFACE, st::CODEX_APP_STATEFUL_BACKOFF_STATE_KEY).is_none());
    let mut s = good.clone();
    s.last_applied_rrule = String::new();
    s.host_update_failures = vec![failure("T", "O", "2026-08-10T00:00:00+00:00", 1)];
    assert!(st::normalize_scheduler_state(&s, "g1", "a1", st::CODEX_APP_SURFACE, st::CODEX_APP_STATEFUL_BACKOFF_STATE_KEY).is_some());
    // Non-positive progression entry → None (and the build error path).
    let mut s = good.clone();
    s.progression_minutes = vec![15, 0];
    assert!(st::normalize_scheduler_state(&s, "g1", "a1", st::CODEX_APP_SURFACE, st::CODEX_APP_STATEFUL_BACKOFF_STATE_KEY).is_none());
    assert!(st::build_scheduler_state(
        "g1", "a1", st::CODEX_APP_SURFACE, st::CODEX_APP_STATEFUL_BACKOFF_STATE_KEY,
        "tok", "id", 0, vec![0], "R", 1, vec![],
    )
    .is_err());
}

#[test]
fn progression_cursor_behaviors() {
    let mut state = st::build_scheduler_state(
        "g1", "a1", st::CODEX_APP_SURFACE, st::CODEX_APP_STATEFUL_BACKOFF_STATE_KEY,
        &st::reset_token("tick_next", &st::identity_signature("g1", "a1", st::CODEX_APP_SURFACE), "FREQ=MINUTELY;INTERVAL=15"),
        &st::identity_signature("g1", "a1", st::CODEX_APP_SURFACE),
        0,
        vec![15, 30],
        "FREQ=MINUTELY;INTERVAL=15",
        1,
        vec![],
    )
    .unwrap();
    // advance → 30m (index 1); next advance wraps to 15m (returns true).
    assert_eq!(st::apply_next_progression(&mut state, 2).as_deref(), Some("FREQ=MINUTELY;INTERVAL=30"));
    assert!(st::advance_progression(&mut state), "wraps to start");
    assert!(!st::advance_progression(&mut state), "index 1, no wrap");
    // current_rrule / current_progression_minutes.
    assert_eq!(st::current_progression_minutes(&state), Some(15));
    assert_eq!(st::current_rrule(&state).as_deref(), Some("FREQ=MINUTELY;INTERVAL=15"));
    // Empty progression: no-op advance, None rrules.
    state.progression_minutes = vec![];
    assert!(!st::advance_progression(&mut state));
    assert_eq!(st::apply_next_progression(&mut state, 3), None);
    assert_eq!(st::current_progression_minutes(&state), None);
    assert_eq!(st::current_rrule(&state), None);
}

#[test]
fn persistence_roundtrip_and_scope_guard() {
    let dir = tempfile::tempdir().unwrap();
    let goal_dir = dir.path();
    let state = st::build_scheduler_state(
        "g1", "a1", st::CODEX_APP_SURFACE, st::CODEX_APP_STATEFUL_BACKOFF_STATE_KEY,
        "tok", &st::identity_signature("g1", "a1", st::CODEX_APP_SURFACE), 0, vec![15],
        "FREQ=MINUTELY;INTERVAL=15", 1, vec![],
    )
    .unwrap();
    st::write_scheduler_state(goal_dir, &state).unwrap();
    let loaded = st::load_scheduler_state(
        goal_dir,
        "a1",
        st::CODEX_APP_SURFACE,
        st::CODEX_APP_STATEFUL_BACKOFF_STATE_KEY,
    );
    assert_eq!(loaded, Some(state));
    // Wrong scope → None; missing file → None; corrupt file → None.
    assert!(st::load_scheduler_state(goal_dir, "other", st::CODEX_APP_SURFACE, st::CODEX_APP_STATEFUL_BACKOFF_STATE_KEY).is_none());
    assert!(st::load_scheduler_state(goal_dir, "a1", st::CODEX_APP_SURFACE, "other-key").is_none());
    let path = st::scheduler_state_path(goal_dir, "a1", st::CODEX_APP_SURFACE, st::CODEX_APP_STATEFUL_BACKOFF_STATE_KEY);
    std::fs::write(&path, "{corrupt").unwrap();
    assert!(st::load_scheduler_state(goal_dir, "a1", st::CODEX_APP_SURFACE, st::CODEX_APP_STATEFUL_BACKOFF_STATE_KEY).is_none());
    // safe_segment: path-hostile characters are stripped (agent ids with
    // separators cannot escape the scheduler-state dir).
    let p = st::scheduler_state_path(goal_dir, "../evil/agent", "surf ace", "key");
    assert!(p.starts_with(goal_dir), "{p:?}");
    assert!(!p.to_string_lossy().contains(".."), "{p:?}");
}

#[test]
fn misc_helpers() {
    // identity_signature / reset_token are deterministic.
    assert_eq!(
        st::identity_signature("g", "a", "s"),
        st::identity_signature("g", "a", "s")
    );
    assert_ne!(st::identity_signature("g", "a", "s"), st::identity_signature("g", "a", "x"));
    let tok = st::reset_token("tick_next", "identity", "RRULE");
    assert!(!tok.is_empty());
    // stable_digest length cap.
    assert_eq!(st::stable_digest(&["a", "b"], 8).len(), 8);
}
