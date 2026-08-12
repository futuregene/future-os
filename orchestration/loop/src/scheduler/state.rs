//! Scheduler state machine (G-10) — rrule recurrence + progression + host
//! update failures, persisted across decision cycles.
//!
//! Mirrors reference `control_plane/scheduler/state.py` (354 lines): the state
//! machine that turns a cadence class into a concrete MINUTELY rrule, walks
//! a `progression_minutes` backoff sequence across cycles, and retains host
//! update failures (target vs observed rrule drift) with a bounded cache.
//!
//! Scope trade-off (refactor plan §5.2 G-10): only the cadence-class subset
//! `once` / `hourly` / `daily` / `weekly` is implemented plus counter-based
//! progression — no full RFC5545 recur semantics or free-text rrule parsing
//! beyond `FREQ=MINUTELY;INTERVAL=N` (which is what reference itself emits).
//!
//! Persistence: one JSON file per (goal, agent, surface, state-key) under
//! `<store-root>/goals/<goal_id>/scheduler-state/<agent>/<surface>/<hash>.json`,
//! written atomically (tmp + rename, like reference `write_scheduler_state`).
//! `store.rs` backup/restore carries the `scheduler-state` directory so a
//! restore does not silently reset progression (P1 risk: replay/backup
//! interaction).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// File schema version (reference `SCHEDULER_STATE_SCHEMA_VERSION`).
pub const SCHEDULER_STATE_SCHEMA_VERSION: &str = "future_loop_scheduler_state_v0";
/// Host-update-failure record schema version (LoopX).
pub const SCHEDULER_HOST_UPDATE_FAILURE_SCHEMA_VERSION: &str = "scheduler_host_update_failure_v0";
/// Bounded cache of retained failures (LoopX).
pub const SCHEDULER_HOST_UPDATE_FAILURE_CACHE_LIMIT: usize = 4;
/// Retention TTL for host update failures (LoopX: 24h).
pub const SCHEDULER_HOST_UPDATE_FAILURE_TTL_SECS: u64 = 24 * 60 * 60;
/// Default surface (reference `CODEX_APP_SURFACE`).
pub const CODEX_APP_SURFACE: &str = "codex_app";
/// Default state key (reference `CODEX_APP_STATEFUL_BACKOFF_STATE_KEY`).
pub const CODEX_APP_STATEFUL_BACKOFF_STATE_KEY: &str = "scheduler_hint.codex_app.stateful_backoff";
/// Stateful-backoff payload schema (reference `CODEX_APP_STATEFUL_BACKOFF_SCHEMA_VERSION`).
pub const CODEX_APP_STATEFUL_BACKOFF_SCHEMA_VERSION: &str = "codex_app_stateful_backoff_v0";
/// Default monitor-wait backoff progression in minutes (LoopX
/// `MONITOR_WAIT_PROGRESSION_MINUTES = [15, 30, 60]`).
pub const MONITOR_WAIT_PROGRESSION_MINUTES: &[i64] = &[15, 30, 60];

/// Build `FREQ=MINUTELY;INTERVAL=N` (reference `rrule_for_minutes`).
pub fn rrule_for_minutes(minutes: i64) -> String {
    format!("FREQ=MINUTELY;INTERVAL={}", minutes.max(1))
}

/// Strip a leading `RRULE:` prefix and collapse whitespace (LoopX
/// `normalize_scheduler_rrule`).
pub fn normalize_scheduler_rrule(value: &str) -> String {
    let text = value.split_whitespace().collect::<Vec<_>>().join(" ");
    text.strip_prefix("RRULE:")
        .map(|t| t.trim().to_string())
        .unwrap_or(text)
}

/// Parse `INTERVAL` from a normalized MINUTELY rrule (LoopX
/// `scheduler_rrule_interval_minutes`). Returns `None` for non-MINUTELY
/// recurrences or malformed intervals.
pub fn scheduler_rrule_interval_minutes(value: &str) -> Option<i64> {
    let text = normalize_scheduler_rrule(value);
    let mut freq: Option<String> = None;
    let mut interval: Option<i64> = None;
    for part in text.split(';') {
        let (key, value) = part.split_once('=')?;
        match key.trim().to_uppercase().as_str() {
            "FREQ" => freq = Some(value.trim().to_uppercase()),
            "INTERVAL" => interval = value.trim().parse().ok(),
            _ => {}
        }
    }
    if freq.as_deref() != Some("MINUTELY") {
        return None;
    }
    interval.filter(|i| *i > 0)
}

/// Map a cadence class onto a MINUTELY rrule for the supported subset
/// (`once` / `hourly` / `daily` / `weekly`). Recurring classes return
/// `Some(rrule)`; `once` and non-recurring classes return `None` (single
/// execution, no cross-cycle recurrence — reference `once` semantics).
pub fn rrule_for_cadence_class(cadence_class: &str) -> Option<String> {
    match normalize_scheduler_rrule(cadence_class)
        .to_ascii_lowercase()
        .as_str()
    {
        "hourly" | "hour" | "1h" => Some(rrule_for_minutes(60)),
        "daily" | "day" | "1d" => Some(rrule_for_minutes(24 * 60)),
        "weekly" | "week" => Some(rrule_for_minutes(7 * 24 * 60)),
        "once" | "" | "none" => None,
        // A raw rrule string (FREQ=MINUTELY;INTERVAL=N) passes through.
        text if text.starts_with("freq=") => Some(normalize_scheduler_rrule(cadence_class)),
        _ => None,
    }
}

/// Human label for a minutes interval (reference scheduler display).
pub fn cadence_label(minutes: i64) -> String {
    match minutes {
        m if m % (7 * 24 * 60) == 0 => format!("{}w", m / (7 * 24 * 60)),
        m if m % (24 * 60) == 0 => format!("{}d", m / (24 * 60)),
        m if m % 60 == 0 => format!("{}h", m / 60),
        m => format!("{m}m"),
    }
}

/// Parse a monitor cadence interval string (`15m`, `1h`, `2d`, `30s`) into
/// seconds — reference `monitor_cadence_delta` (MONITOR_CADENCE_PATTERN).
/// Returns `None` for unparsable or empty values. Cadence classes map
/// through [`rrule_for_cadence_class`].
pub fn monitor_cadence_secs(value: &str) -> Option<u64> {
    let text = value.trim();
    if text.is_empty() {
        return None;
    }
    let split = text.find(|c: char| !c.is_ascii_digit())?;
    let (count, unit) = text.split_at(split);
    let count: u64 = count.parse().ok()?;
    if count == 0 {
        return None;
    }
    let unit = unit.trim().to_ascii_lowercase();
    if unit.starts_with('s') {
        Some(count)
    } else if unit.starts_with('m') {
        Some(count * 60)
    } else if unit.starts_with('h') {
        Some(count * 3600)
    } else if unit.starts_with('d') {
        Some(count * 86400)
    } else {
        None
    }
}

// ── Host update failures ───────────────────────────────────────────────────

/// A recorded host-update failure (reference `scheduler_host_update_failure_v0`):
/// the control plane asked the host to switch to `target_rrule` but observed
/// `observed_host_rrule` instead. Retained for a bounded TTL so the next tick
/// can suppress a redundant update and surface drift diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostUpdateFailure {
    pub schema_version: String,
    pub target_rrule: String,
    pub observed_host_rrule: String,
    pub failure_kind: String,
    pub failed_at: String,
    pub failure_count: u32,
}

impl HostUpdateFailure {
    fn pair(&self) -> (String, String) {
        (self.target_rrule.clone(), self.observed_host_rrule.clone())
    }
}

/// Normalize one raw failure record; `None` when invalid (LoopX
/// `normalize_scheduler_host_update_failure`).
pub fn normalize_host_update_failure(value: &serde_json::Value) -> Option<HostUpdateFailure> {
    let obj = value.as_object()?;
    if obj.get("schema_version").and_then(|v| v.as_str())
        != Some(SCHEDULER_HOST_UPDATE_FAILURE_SCHEMA_VERSION)
    {
        return None;
    }
    let target_rrule = normalize_scheduler_rrule(obj.get("target_rrule")?.as_str()?);
    let observed_host_rrule = normalize_scheduler_rrule(
        obj.get("observed_host_rrule")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
    );
    let failure_kind = obj.get("failure_kind")?.as_str()?.trim().to_string();
    let failed_at = obj.get("failed_at")?.as_str()?.trim().to_string();
    let failure_count = obj.get("failure_count")?.as_u64()? as u32;
    if target_rrule.is_empty()
        || failure_kind.is_empty()
        || failed_at.is_empty()
        || failure_count < 1
    {
        return None;
    }
    Some(HostUpdateFailure {
        schema_version: SCHEDULER_HOST_UPDATE_FAILURE_SCHEMA_VERSION.to_string(),
        target_rrule,
        observed_host_rrule,
        failure_kind,
        failed_at,
        failure_count,
    })
}

/// Dedup by (target, observed) pair, keep the latest, cap at the cache limit
/// (reference `normalize_scheduler_host_update_failures`).
pub fn normalize_host_update_failures(value: &[serde_json::Value]) -> Vec<HostUpdateFailure> {
    let mut normalized: Vec<HostUpdateFailure> = vec![];
    for candidate in value {
        let Some(failure) = normalize_host_update_failure(candidate) else {
            continue;
        };
        normalized.retain(|item| item.pair() != failure.pair());
        normalized.push(failure);
    }
    normalized
        .into_iter()
        .rev()
        .take(SCHEDULER_HOST_UPDATE_FAILURE_CACHE_LIMIT)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

/// Keep failures inside the TTL (and matching an expected observed rrule,
/// when given) — reference `retained_scheduler_host_update_failures`.
pub fn retained_host_update_failures(
    failures: &[HostUpdateFailure],
    now_epoch: u64,
    expected_observed_rrule: Option<&str>,
) -> Vec<HostUpdateFailure> {
    let cutoff = now_epoch.saturating_sub(SCHEDULER_HOST_UPDATE_FAILURE_TTL_SECS);
    failures
        .iter()
        .filter(|f| {
            let failed_at = parse_epoch(&f.failed_at).unwrap_or(0);
            failed_at >= cutoff
        })
        .filter(|f| {
            expected_observed_rrule
                .map(|r| {
                    normalize_scheduler_rrule(&f.observed_host_rrule)
                        == normalize_scheduler_rrule(r)
                })
                .unwrap_or(true)
        })
        .cloned()
        .collect()
}

/// Merge one failure into an existing list: retain (TTL + pair dedup) then
/// append — reference `merge_scheduler_host_update_failure`. A re-recorded
/// (target, observed) pair replaces the previous record (latest wins).
pub fn merge_host_update_failure(
    failures: &[HostUpdateFailure],
    failure: HostUpdateFailure,
    now_epoch: u64,
) -> Vec<HostUpdateFailure> {
    let retained =
        retained_host_update_failures(failures, now_epoch, Some(&failure.observed_host_rrule));
    let mut combined: Vec<HostUpdateFailure> = retained;
    combined.push(failure);
    // Dedup by pair, keeping the LATEST occurrence (reference normalize appends
    // after removing the old pair).
    let mut seen = std::collections::HashSet::new();
    let mut result: Vec<HostUpdateFailure> = Vec::with_capacity(combined.len());
    for f in combined.into_iter().rev() {
        if seen.insert(f.pair()) {
            result.push(f);
        }
    }
    result.reverse();
    result
}

/// Best-effort parse of a reference ISO-8601 timestamp (`failed_at`) to epoch.
pub fn parse_epoch(iso: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(iso)
        .ok()
        .map(|dt| dt.timestamp().max(0) as u64)
}

// ── Scheduler state ────────────────────────────────────────────────────────

/// The persisted scheduler state machine record (LoopX
/// `future_loop_scheduler_state_v0`): scope identity, progression cursor, the last
/// applied rrule, and retained host update failures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchedulerState {
    pub schema_version: String,
    pub goal_id: String,
    pub agent_id: String,
    pub surface: String,
    pub state_key: String,
    pub reset_token: String,
    pub identity_signature: String,
    pub progression_index: usize,
    pub progression_minutes: Vec<i64>,
    pub last_applied_rrule: String,
    pub updated_at: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub host_update_failures: Vec<HostUpdateFailure>,
}

/// Validate a state against its scope + required fields (LoopX
/// `normalize_scheduler_state`). `None` when it does not match the target
/// scope or misses a required persisted field.
pub fn normalize_scheduler_state(
    state: &SchedulerState,
    goal_id: &str,
    agent_id: &str,
    surface: &str,
    state_key: &str,
) -> Option<SchedulerState> {
    if state.schema_version != SCHEDULER_STATE_SCHEMA_VERSION
        && state.schema_version != "loopx_scheduler_state_v0"
    {
        return None;
    }
    if state.goal_id != goal_id || state.agent_id != agent_id {
        return None;
    }
    if state.surface != surface || state.state_key != state_key {
        return None;
    }
    if state.reset_token.trim().is_empty() || state.identity_signature.trim().is_empty() {
        return None;
    }
    if state.last_applied_rrule.trim().is_empty() && state.host_update_failures.is_empty() {
        return None;
    }
    if state.progression_minutes.iter().any(|m| *m <= 0) {
        return None;
    }
    Some(state.clone())
}

/// Build a validated scheduler state (reference `build_scheduler_state`).
#[allow(clippy::too_many_arguments)]
pub fn build_scheduler_state(
    goal_id: &str,
    agent_id: &str,
    surface: &str,
    state_key: &str,
    reset_token: &str,
    identity_signature: &str,
    progression_index: usize,
    progression_minutes: Vec<i64>,
    last_applied_rrule: &str,
    updated_at: u64,
    host_update_failures: Vec<HostUpdateFailure>,
) -> Result<SchedulerState> {
    let state = SchedulerState {
        schema_version: SCHEDULER_STATE_SCHEMA_VERSION.to_string(),
        goal_id: goal_id.to_string(),
        agent_id: agent_id.to_string(),
        surface: surface.to_string(),
        state_key: state_key.to_string(),
        reset_token: reset_token.to_string(),
        identity_signature: identity_signature.to_string(),
        progression_index,
        progression_minutes,
        last_applied_rrule: last_applied_rrule.to_string(),
        updated_at,
        host_update_failures,
    };
    normalize_scheduler_state(&state, goal_id, agent_id, surface, state_key).ok_or_else(|| {
        anyhow::anyhow!("scheduler state is missing required persisted-state fields")
    })
}

/// Stable digest (reference `_stable_digest`): FNV-1a over the joined parts,
/// hex-encoded to `length` chars. Deterministic across processes and
/// restarts — the identity anchor for persisted state.
pub fn stable_digest(parts: &[&str], length: usize) -> String {
    let mut hasher = DefaultHasher::new();
    for part in parts {
        part.hash(&mut hasher);
        "\u{1f}".hash(&mut hasher);
    }
    let digest = hasher.finish();
    format!("{digest:016x}")[..length.min(16)].to_string()
}

/// Identity signature: stable digest of the scope keys (LoopX
/// `identity_signature`, 12 hex chars).
pub fn identity_signature(goal_id: &str, agent_id: &str, surface: &str) -> String {
    stable_digest(&[goal_id, agent_id, surface], 12)
}

/// Reset token: stable digest of the cadence action + identity + profile
/// (reference `reset_token`, 16 hex chars). When the cadence action or identity
/// changes, the token changes and the host must reset progression to the
/// initial interval.
pub fn reset_token(action: &str, identity_sig: &str, initial_rrule: &str) -> String {
    stable_digest(&[action, identity_sig, initial_rrule], 16)
}

/// The interval minutes at the current progression index (LoopX
/// `progression_minutes[progression_index]`).
pub fn current_progression_minutes(state: &SchedulerState) -> Option<i64> {
    state
        .progression_minutes
        .get(state.progression_index)
        .copied()
}

/// The rrule for the current progression step.
pub fn current_rrule(state: &SchedulerState) -> Option<String> {
    current_progression_minutes(state).map(rrule_for_minutes)
}

/// Advance the progression cursor to the next step, wrapping at the end
/// (LoopX: `progression_index = (index + 1) % len`). Returns `true` when the
/// cursor wrapped (a full backoff cycle elapsed). No-op when progression is
/// empty.
pub fn advance_progression(state: &mut SchedulerState) -> bool {
    if state.progression_minutes.is_empty() {
        return false;
    }
    state.progression_index += 1;
    if state.progression_index >= state.progression_minutes.len() {
        state.progression_index = 0;
        return true;
    }
    false
}

/// Apply the next progression step: advance the cursor and return the new
/// current rrule (None when progression is empty). Also bumps `updated_at`.
pub fn apply_next_progression(state: &mut SchedulerState, now_epoch: u64) -> Option<String> {
    if state.progression_minutes.is_empty() {
        return None;
    }
    advance_progression(state);
    state.last_applied_rrule = current_rrule(state)?;
    state.updated_at = now_epoch;
    Some(state.last_applied_rrule.clone())
}

// ── Persistence ────────────────────────────────────────────────────────────

/// Path for one scheduler state file (reference `scheduler_state_path`, rooted
/// at the store's goal directory): the state-key hash keeps distinct state
/// machines (e.g. future scheduler hint kinds) from colliding.
pub fn scheduler_state_path(
    goal_dir: &Path,
    agent_id: &str,
    surface: &str,
    state_key: &str,
) -> PathBuf {
    let state_hash = stable_digest(&[state_key], 16);
    goal_dir
        .join("scheduler-state")
        .join(safe_segment(agent_id))
        .join(safe_segment(surface))
        .join(format!("{state_hash}.json"))
}

/// Sanitize a path segment (reference `_safe_segment`).
fn safe_segment(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches(['-', '_', '.']);
    if trimmed.is_empty() {
        "default".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Load + validate a scheduler state file. Returns `None` when the file is
/// absent, unparsable, or no longer matches the target scope.
pub fn load_scheduler_state(
    goal_dir: &Path,
    agent_id: &str,
    surface: &str,
    state_key: &str,
) -> Option<SchedulerState> {
    if agent_id.trim().is_empty() {
        return None;
    }
    let path = scheduler_state_path(goal_dir, agent_id, surface, state_key);
    let text = std::fs::read_to_string(&path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&text).ok()?;
    let state: SchedulerState = serde_json::from_value(parsed).ok()?;
    normalize_scheduler_state(
        &state,
        &state.goal_id,
        &state.agent_id,
        &state.surface,
        &state.state_key,
    )
}

/// Write a validated scheduler state atomically (tmp + rename, LoopX
/// `write_scheduler_state`). Refuses to write when the state does not match
/// its own scope or schema.
pub fn write_scheduler_state(goal_dir: &Path, state: &SchedulerState) -> Result<PathBuf> {
    let normalized = normalize_scheduler_state(
        state,
        &state.goal_id,
        &state.agent_id,
        &state.surface,
        &state.state_key,
    )
    .ok_or_else(|| anyhow::anyhow!("scheduler state does not match target scope or schema"))?;
    let path = scheduler_state_path(goal_dir, &state.agent_id, &state.surface, &state.state_key);
    path.parent()
        .map(|parent| std::fs::create_dir_all(parent).context("create scheduler-state dir"))
        .transpose()?;
    let tmp = path.with_extension("json.tmp");
    let payload = serde_json::to_string_pretty(&normalized)?;
    std::fs::write(&tmp, format!("{payload}\n")).context("write scheduler state tmp")?;
    std::fs::rename(&tmp, &path).context("rename scheduler state")?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOAL: &str = "goal-1";
    const AGENT: &str = "codex-agent";

    fn sample_state() -> SchedulerState {
        let identity = identity_signature(GOAL, AGENT, CODEX_APP_SURFACE);
        build_scheduler_state(
            GOAL,
            AGENT,
            CODEX_APP_SURFACE,
            CODEX_APP_STATEFUL_BACKOFF_STATE_KEY,
            &reset_token("tick_next", &identity, &rrule_for_minutes(15)),
            &identity,
            0,
            MONITOR_WAIT_PROGRESSION_MINUTES.to_vec(),
            &rrule_for_minutes(15),
            1_700_000_000,
            vec![],
        )
        .unwrap()
    }

    #[test]
    fn rrule_unknown_keys_are_ignored() {
        assert_eq!(
            scheduler_rrule_interval_minutes("FREQ=MINUTELY;INTERVAL=9;BYDAY=MO"),
            Some(9)
        );
    }

    #[test]
    fn host_update_failures_skip_unparseable_entries() {
        let bad = serde_json::json!({"schema_version": "wrong"});
        let normalized = normalize_host_update_failures(&[bad]);
        assert!(normalized.is_empty());
    }

    #[test]
    fn safe_segment_defaults_when_nothing_remains() {
        assert_eq!(safe_segment("---..."), "default");
        assert_eq!(safe_segment("agent.a-1"), "agent.a-1");
    }

    #[test]
    fn rrule_roundtrip() {
        assert_eq!(rrule_for_minutes(15), "FREQ=MINUTELY;INTERVAL=15");
        assert_eq!(
            normalize_scheduler_rrule(" RRULE:FREQ=MINUTELY;INTERVAL=30 "),
            "FREQ=MINUTELY;INTERVAL=30"
        );
        assert_eq!(
            scheduler_rrule_interval_minutes("RRULE:FREQ=MINUTELY;INTERVAL=7"),
            Some(7)
        );
        assert_eq!(scheduler_rrule_interval_minutes("FREQ=DAILY"), None);
        assert_eq!(
            scheduler_rrule_interval_minutes("FREQ=MINUTELY;INTERVAL=0"),
            None
        );
        assert_eq!(rrule_for_minutes(0), "FREQ=MINUTELY;INTERVAL=1");
    }

    #[test]
    fn cadence_classes_map_to_minutely_rrules() {
        assert_eq!(
            rrule_for_cadence_class("hourly"),
            Some(rrule_for_minutes(60))
        );
        assert_eq!(
            rrule_for_cadence_class("daily"),
            Some(rrule_for_minutes(1440))
        );
        assert_eq!(
            rrule_for_cadence_class("weekly"),
            Some(rrule_for_minutes(10080))
        );
        assert_eq!(rrule_for_cadence_class("once"), None);
        assert_eq!(rrule_for_cadence_class("bounded_segment"), None);
        // Raw rrule passthrough.
        assert_eq!(
            rrule_for_cadence_class("FREQ=MINUTELY;INTERVAL=5"),
            Some("FREQ=MINUTELY;INTERVAL=5".to_string())
        );
        assert_eq!(cadence_label(60), "1h");
        assert_eq!(cadence_label(15), "15m");
    }

    #[test]
    fn monitor_cadence_interval_parsing() {
        assert_eq!(monitor_cadence_secs("15m"), Some(15 * 60));
        assert_eq!(monitor_cadence_secs("1h"), Some(3600));
        assert_eq!(monitor_cadence_secs("2d"), Some(2 * 86400));
        assert_eq!(monitor_cadence_secs("30s"), Some(30));
        assert_eq!(monitor_cadence_secs(" 5 min "), Some(300));
        assert_eq!(monitor_cadence_secs("0h"), None);
        assert_eq!(monitor_cadence_secs("hourly"), None);
        assert_eq!(monitor_cadence_secs(""), None);
    }

    #[test]
    fn identity_and_reset_token_are_stable() {
        let a = identity_signature(GOAL, AGENT, CODEX_APP_SURFACE);
        let b = identity_signature(GOAL, AGENT, CODEX_APP_SURFACE);
        assert_eq!(a, b);
        assert_eq!(a.len(), 12);
        let c = identity_signature(GOAL, "other-agent", CODEX_APP_SURFACE);
        assert_ne!(a, c);
        // Token changes when the cadence action changes (reset trigger).
        let t1 = reset_token("tick_next", &a, "FREQ=MINUTELY;INTERVAL=15");
        let t2 = reset_token("wait_until_due", &a, "FREQ=MINUTELY;INTERVAL=15");
        assert_eq!(t1.len(), 16);
        assert_ne!(t1, t2);
    }

    #[test]
    fn progression_walks_and_wraps() {
        let mut s = sample_state();
        assert_eq!(current_progression_minutes(&s), Some(15));
        assert_eq!(
            current_rrule(&s).as_deref(),
            Some("FREQ=MINUTELY;INTERVAL=15")
        );
        assert!(!advance_progression(&mut s));
        assert_eq!(current_progression_minutes(&s), Some(30));
        assert!(!advance_progression(&mut s));
        assert_eq!(current_progression_minutes(&s), Some(60));
        // Wrap back to 15 after the full [15, 30, 60] cycle.
        assert!(advance_progression(&mut s));
        assert_eq!(current_progression_minutes(&s), Some(15));
    }

    #[test]
    fn apply_next_progression_updates_last_applied_rrule() {
        let mut s = sample_state();
        let rrule = apply_next_progression(&mut s, 1_700_000_100).expect("progression");
        assert_eq!(rrule, "FREQ=MINUTELY;INTERVAL=30");
        assert_eq!(s.last_applied_rrule, rrule);
        assert_eq!(s.updated_at, 1_700_000_100);
    }

    #[test]
    fn empty_progression_is_stable() {
        let mut s = sample_state();
        s.progression_minutes = vec![];
        s.progression_index = 0;
        assert!(!advance_progression(&mut s));
        assert_eq!(apply_next_progression(&mut s, 1_700_000_100), None);
        assert_eq!(current_progression_minutes(&s), None);
    }

    #[test]
    fn host_failure_merge_dedups_and_caps() {
        let now = parse_epoch("2026-08-05T12:00:00+00:00").unwrap();
        let mk = |kind: &str, n: u32| HostUpdateFailure {
            schema_version: SCHEDULER_HOST_UPDATE_FAILURE_SCHEMA_VERSION.to_string(),
            target_rrule: "FREQ=MINUTELY;INTERVAL=30".to_string(),
            observed_host_rrule: "FREQ=MINUTELY;INTERVAL=1440".to_string(),
            failure_kind: kind.to_string(),
            failed_at: "2026-08-05T12:00:00+00:00".to_string(),
            failure_count: n,
        };
        let mut failures = vec![mk("host_stale_rrule", 1)];
        failures = merge_host_update_failure(&failures, mk("host_stale_rrule", 2), now);
        // Same (target, observed) pair replaces the old record (latest wins).
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].failure_count, 2);
        // Distinct kinds accumulate up to the cache limit.
        for kind in ["timeout", "rejected", "drift", "conflict", "overflow"] {
            let f = HostUpdateFailure {
                failure_kind: kind.to_string(),
                ..mk("x", 1)
            };
            failures = merge_host_update_failure(&failures, f, now);
        }
        assert!(failures.len() <= SCHEDULER_HOST_UPDATE_FAILURE_CACHE_LIMIT);
        assert_eq!(failures.last().unwrap().failure_kind, "overflow");
    }

    #[test]
    fn stale_failures_drop_out_of_retention() {
        let now = parse_epoch("2026-08-05T12:00:00+00:00").unwrap();
        let stale_ts = "2026-08-01T12:00:00+00:00"; // 4 days before now (> 24h TTL)
        let stale = HostUpdateFailure {
            failed_at: stale_ts.to_string(),
            ..sample_failure()
        };
        let fresh = HostUpdateFailure {
            failed_at: "2026-08-05T12:00:00+00:00".to_string(),
            ..sample_failure()
        };
        let retained = retained_host_update_failures(&[stale, fresh.clone()], now, None);
        assert_eq!(retained.len(), 1);
        assert_eq!(retained[0].failed_at, fresh.failed_at);
    }

    fn sample_failure() -> HostUpdateFailure {
        HostUpdateFailure {
            schema_version: SCHEDULER_HOST_UPDATE_FAILURE_SCHEMA_VERSION.to_string(),
            target_rrule: "FREQ=MINUTELY;INTERVAL=30".to_string(),
            observed_host_rrule: "FREQ=MINUTELY;INTERVAL=1440".to_string(),
            failure_kind: "host_stale_rrule".to_string(),
            failed_at: "2026-08-05T12:00:00+00:00".to_string(),
            failure_count: 1,
        }
    }

    #[test]
    fn normalize_rejects_scope_mismatch_and_missing_fields() {
        let s = sample_state();
        assert!(normalize_scheduler_state(
            &s,
            GOAL,
            AGENT,
            CODEX_APP_SURFACE,
            CODEX_APP_STATEFUL_BACKOFF_STATE_KEY
        )
        .is_some());
        assert!(normalize_scheduler_state(
            &s,
            "other-goal",
            AGENT,
            CODEX_APP_SURFACE,
            CODEX_APP_STATEFUL_BACKOFF_STATE_KEY
        )
        .is_none());
        let mut broken = s.clone();
        broken.reset_token.clear();
        assert!(normalize_scheduler_state(
            &broken,
            GOAL,
            AGENT,
            CODEX_APP_SURFACE,
            CODEX_APP_STATEFUL_BACKOFF_STATE_KEY
        )
        .is_none());
    }

    #[test]
    fn persistence_roundtrip_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let s = sample_state();
        let path = write_scheduler_state(dir.path(), &s).unwrap();
        assert!(path.exists());
        // "Restart": load from disk and confirm progression did not reset.
        let loaded = load_scheduler_state(
            dir.path(),
            AGENT,
            CODEX_APP_SURFACE,
            CODEX_APP_STATEFUL_BACKOFF_STATE_KEY,
        )
        .unwrap();
        assert_eq!(loaded, s);
        assert_eq!(loaded.progression_index, 0);
        assert_eq!(loaded.last_applied_rrule, "FREQ=MINUTELY;INTERVAL=15");
        // Advance, persist again, reload.
        let mut advanced = loaded;
        let rrule = apply_next_progression(&mut advanced, 1_700_000_100).unwrap();
        assert_eq!(rrule, "FREQ=MINUTELY;INTERVAL=30");
        write_scheduler_state(dir.path(), &advanced).unwrap();
        let reloaded = load_scheduler_state(
            dir.path(),
            AGENT,
            CODEX_APP_SURFACE,
            CODEX_APP_STATEFUL_BACKOFF_STATE_KEY,
        )
        .unwrap();
        assert_eq!(reloaded.progression_index, 1);
        assert_eq!(reloaded.last_applied_rrule, "FREQ=MINUTELY;INTERVAL=30");
        // Unknown agent has no state.
        assert!(load_scheduler_state(
            dir.path(),
            "nobody",
            CODEX_APP_SURFACE,
            CODEX_APP_STATEFUL_BACKOFF_STATE_KEY
        )
        .is_none());
        // Empty agent id never loads.
        assert!(load_scheduler_state(
            dir.path(),
            "",
            CODEX_APP_SURFACE,
            CODEX_APP_STATEFUL_BACKOFF_STATE_KEY
        )
        .is_none());
    }
}
