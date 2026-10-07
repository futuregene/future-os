//! Deterministic kernel for `future-tasks`.
//!
//! Everything here is a pure function of the inputs: no I/O, no clock, no
//! agent. `next_due` computes schedule anchors; `join_claim` decides whether
//! a task's dependency edges have fired; `compose_upstream_block` builds the
//! fan-in evidence block (loop-style budget + head/tail truncation + index
//! retention); `truncate` is the shared byte-budget truncator.

use crate::types::{DepJoin, Task, TaskDep, TaskDepState};
use chrono::Datelike;
use serde::Deserialize;

/// Shared byte budget across all upstream sources (loop: `UPSTREAM_EVIDENCE_CHARS`).
pub const UPSTREAM_SUMMARY_CHARS: usize = 1_200;
/// Budget below which a source still gets an index entry but no snippet.
pub const MIN_SNIPPET_CHARS: usize = 12;
/// Stored `result_summary` cap (head…tail).
pub const RESULT_SUMMARY_CHARS: usize = 2_000;
/// Maximum chain depth when cycle-checking (defense in depth).
pub const MAX_DEP_DEPTH: usize = 32;

/// Truncate to `max` bytes, keeping the head (context) and tail (conclusion)
/// with a single `…` marker. Byte-budget safe for any UTF-8 content.
pub fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    const MARK: &str = "…";
    let head_bytes = max * 3 / 4;
    let tail_bytes = max - head_bytes - MARK.len();
    let head = &text[..floor_char_boundary(text, head_bytes)];
    let tail_start = floor_char_boundary(text, text.len() - tail_bytes);
    let tail = &text[tail_start..];
    format!("{head}{MARK}{tail}")
}

fn floor_char_boundary(s: &str, idx: usize) -> usize {
    let mut i = idx.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

// ─── next_due ─────────────────────────────────────────────────────────────

/// One schedule mode (parsed from `tasks.trigger_json`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum ScheduleMode {
    Once { date: String, time: String },
    Interval { every_minutes: i64, anchor: i64 },
    Daily { time: String },
    Weekly { days: Vec<String>, time: String },
    Monthly { day: i64, time: String },
}

/// Parse `trigger_json` into a schedule mode (only for `trigger_kind=schedule`).
pub fn parse_schedule(trigger_json: &serde_json::Value) -> Option<ScheduleMode> {
    serde_json::from_value(trigger_json.clone()).ok()
}

/// Compute the next due epoch-ms strictly after `after_ms` for a schedule.
/// Returns `None` for `manual`, `once` already past, or an unparseable trigger.
pub fn next_due(task: &Task, after_ms: i64) -> Option<i64> {
    if !task.enabled || task.deleted_at.is_some() {
        return None;
    }
    match task.trigger_kind {
        crate::types::TriggerKind::Manual => None,
        crate::types::TriggerKind::Schedule => {
            let mode = parse_schedule(&task.trigger_json)?;
            next_due_for_mode(&mode, after_ms)
        }
    }
}

fn next_due_for_mode(mode: &ScheduleMode, after_ms: i64) -> Option<i64> {
    use chrono::{DateTime, Duration, Local, NaiveDate, TimeZone};
    let after = DateTime::from_timestamp_millis(after_ms)?.with_timezone(&Local);
    match mode {
        ScheduleMode::Once { date, time } => {
            let dt = parse_date_time(date, time)?;
            let due = Local.from_local_datetime(&dt).single()?;
            (due.timestamp_millis() > after_ms).then(|| due.timestamp_millis())
        }
        ScheduleMode::Interval {
            every_minutes,
            anchor,
        } => {
            let step = Duration::minutes(*every_minutes)
                .num_milliseconds()
                .max(60_000);
            let mut due = *anchor;
            if due <= after_ms {
                due = *anchor + ((after_ms - *anchor) / step + 1) * step;
            }
            Some(due)
        }
        ScheduleMode::Daily { time } => {
            let t = parse_time(time)?;
            let due = next_calendar_time(&after, t, 1)?;
            (due.timestamp_millis() > after_ms).then(|| due.timestamp_millis())
        }
        ScheduleMode::Weekly { days, time } => {
            let t = parse_time(time)?;
            let day_nums: Vec<u32> = days.iter().filter_map(|d| weekday_num(d)).collect();
            if day_nums.is_empty() {
                return None;
            }
            let mut best: Option<DateTime<Local>> = None;
            for delta in 0..=7 {
                let cand_date = (after + Duration::days(delta)).date_naive();
                let cand_wd = cand_date.weekday().num_days_from_monday();
                if !day_nums.contains(&cand_wd) {
                    continue;
                }
                let cand_dt = cand_date.and_time(t);
                let cand = Local.from_local_datetime(&cand_dt).single()?;
                if cand.timestamp_millis() > after_ms {
                    best = Some(best.map_or(cand, |b| b.min(cand)));
                }
            }
            best.map(|d| d.timestamp_millis())
        }
        ScheduleMode::Monthly { day, time } => {
            let t = parse_time(time)?;
            let mut best: Option<DateTime<Local>> = None;
            for month_delta in 0..=12 {
                let (y, m) = add_months(after.year(), after.month(), month_delta);
                let days_in_month = days_in_month(y, m);
                let dom = (*day).min(days_in_month as i64) as u32;
                let Some(date) = NaiveDate::from_ymd_opt(y, m, dom) else {
                    continue;
                };
                let cand_dt = date.and_time(t);
                let Some(cand) = Local.from_local_datetime(&cand_dt).single() else {
                    continue;
                };
                if cand.timestamp_millis() > after_ms {
                    best = Some(best.map_or(cand, |b| b.min(cand)));
                }
            }
            best.map(|d| d.timestamp_millis())
        }
    }
}

fn parse_date_time(date: &str, time: &str) -> Option<chrono::NaiveDateTime> {
    let d = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    let t = parse_time(time)?;
    Some(d.and_time(t))
}

fn parse_time(time: &str) -> Option<chrono::NaiveTime> {
    chrono::NaiveTime::parse_from_str(time, "%H:%M").ok()
}

fn weekday_num(day: &str) -> Option<u32> {
    match day.to_ascii_lowercase().as_str() {
        "mon" => Some(0),
        "tue" => Some(1),
        "wed" => Some(2),
        "thu" => Some(3),
        "fri" => Some(4),
        "sat" => Some(5),
        "sun" => Some(6),
        _ => None,
    }
}

fn add_months(year: i32, month: u32, delta: i64) -> (i32, u32) {
    let total = year as i64 * 12 + month as i64 - 1 + delta;
    let y = (total.div_euclid(12)) as i32;
    let m = (total.rem_euclid(12) + 1) as u32;
    (y, m)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let (y, m) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let first = chrono::NaiveDate::from_ymd_opt(y, m, 1).unwrap();
    (first - chrono::Duration::days(1)).day()
}

fn next_calendar_time(
    after: &chrono::DateTime<chrono::Local>,
    time: chrono::NaiveTime,
    min_days_ahead: i64,
) -> Option<chrono::DateTime<chrono::Local>> {
    use chrono::{Duration, Local, TimeZone};
    let mut date = after.date_naive();
    let time_already_passed = after.time() >= time;
    if time_already_passed {
        date += Duration::days(1);
    }
    let extra_days = if min_days_ahead > 0 {
        min_days_ahead - 1
    } else {
        0
    };
    date += Duration::days(extra_days);
    Local.from_local_datetime(&date.and_time(time)).single()
}

// ─── join claim ───────────────────────────────────────────────────────────

/// Decide whether a task's dependency edges have fired, and which runs to
/// consume. Pure: the caller persists the consumption.
///
/// Returns `Some(consume)` when the task should trigger; `consume` is the list
/// of `(upstream_task_id, run_id)` pairs to clear from `task_dep_state`.
pub fn join_claim(
    task: &Task,
    deps: &[TaskDep],
    states: &[TaskDepState],
) -> Option<Vec<(String, String)>> {
    if deps.is_empty() {
        return None;
    }
    match task.dep_join {
        DepJoin::All => {
            let mut consume = Vec::new();
            for dep in deps {
                let st = states.iter().find(|s| {
                    s.task_id == dep.task_id && s.upstream_task_id == dep.upstream_task_id
                })?;
                let run_id = st.satisfied_run_id.clone()?;
                consume.push((dep.upstream_task_id.clone(), run_id));
            }
            Some(consume)
        }
        DepJoin::Any => {
            let mut consume = Vec::new();
            for dep in deps {
                if let Some(st) = states.iter().find(|s| {
                    s.task_id == dep.task_id && s.upstream_task_id == dep.upstream_task_id
                }) {
                    if let Some(run_id) = &st.satisfied_run_id {
                        consume.push((dep.upstream_task_id.clone(), run_id.clone()));
                    }
                }
            }
            (!consume.is_empty()).then_some(consume)
        }
    }
}

/// Whether a finished upstream run satisfies a dependency edge.
pub fn run_satisfies(dep_on: crate::types::DepOn, run_status: crate::types::RunStatus) -> bool {
    use crate::types::{DepOn, RunStatus};
    matches!(
        (dep_on, run_status),
        (DepOn::Success, RunStatus::Completed)
            | (DepOn::Failure, RunStatus::Failed)
            | (DepOn::Completed, RunStatus::Completed | RunStatus::Failed)
    )
}

/// Cycle-check: following `deps` from `task_id`, do we reach `task_id` again?
/// Iterative DFS with depth cap; pure.
pub fn would_cycle(task_id: &str, deps: &[TaskDep]) -> bool {
    let mut stack: Vec<(String, usize)> = vec![(task_id.to_string(), 0)];
    let mut visited = std::collections::HashSet::new();
    while let Some((id, depth)) = stack.pop() {
        if depth > MAX_DEP_DEPTH {
            return true;
        }
        if !visited.insert(id.clone()) {
            continue;
        }
        for dep in deps.iter().filter(|d| d.upstream_task_id == id) {
            if dep.task_id == task_id {
                return true;
            }
            stack.push((dep.task_id.clone(), depth + 1));
        }
    }
    false
}

// ─── upstream block (fan-in evidence) ────────────────────────────────────

/// One upstream source for the envelope: task id, name, run id, status, and
/// the stored (already 2000-char truncated) result summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamSource {
    pub task_id: String,
    pub task_name: String,
    pub run_id: String,
    pub status: crate::types::RunStatus,
    pub finished_at: Option<i64>,
    pub result_summary: Option<String>,
}

/// Compose the fan-in upstream block. Every source gets an index entry; the
/// snippets share `UPSTREAM_SUMMARY_CHARS` fairly, truncated head…tail.
/// Sources whose budget is below `MIN_SNIPPET_CHARS` keep the index line with
/// an empty snippet (loop's "index retention" rule).
pub fn compose_upstream_block(sources: &[UpstreamSource]) -> String {
    if sources.is_empty() {
        return String::new();
    }
    let per_source = UPSTREAM_SUMMARY_CHARS / sources.len().max(1);
    let mut lines = Vec::new();
    for src in sources {
        let summary = src.result_summary.as_deref().unwrap_or("[no summary]");
        let snippet = if per_source >= MIN_SNIPPET_CHARS {
            truncate(summary, per_source)
        } else {
            String::new()
        };
        let finished = src
            .finished_at
            .map(|ms| {
                chrono::DateTime::from_timestamp_millis(ms)
                    .map(|d| d.with_timezone(&chrono::Local).to_rfc3339())
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        lines.push(format!(
            "- upstream {} \"{}\" [run {} status={:?} finished={}]: {}",
            src.task_id, src.task_name, src.run_id, src.status, finished, snippet
        ));
    }
    let mut out = String::new();
    out.push_str(
        "Upstream results: (summaries only — read a source's full output before relying on it)\n",
    );
    for line in lines {
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str(
        "Full output of one source: future task output <run-id>     (--format json / --tail N)\n",
    );
    out.push_str("Run ledger of one source:  future task runs <task-id> --limit 20\n");
    out
}

/// Compose the task envelope header (schema + identity + trigger context).
pub fn compose_envelope_header(
    task: &Task,
    kind: crate::types::RunKind,
    due_ms: Option<i64>,
    occurrence: Option<i64>,
) -> String {
    let due = due_ms
        .and_then(|ms| {
            chrono::DateTime::from_timestamp_millis(ms)
                .map(|d| d.with_timezone(&chrono::Local).to_rfc3339())
        })
        .unwrap_or_default();
    let mut out = format!(
        "<task schema=\"{}\" name=\"{}\" id=\"{}\" kind=\"{:?}\" prompt-version=\"{}\"",
        crate::types::TASK_ENVELOPE_SCHEMA_VERSION,
        task.name,
        task.id,
        kind,
        task.prompt_version
    );
    if !due.is_empty() {
        out.push_str(&format!(" due=\"{}\"", due));
    }
    if let Some(n) = occurrence {
        out.push_str(&format!(" occurrence=\"{}\"", n));
    }
    out.push_str(" />\n\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{DepOn, RunStatus, SessionPolicy, Task, TriggerKind};
    use chrono::{Local, NaiveDate, TimeZone, Timelike};

    fn task(trigger: serde_json::Value, kind: TriggerKind) -> Task {
        Task {
            id: "tsk_test".into(),
            name: "test".into(),
            enabled: true,
            prompt: "p".into(),
            prompt_version: 1,
            cwd: "/tmp".into(),
            model_id: None,
            thinking_level: None,
            session_policy: SessionPolicy::New,
            thread_id: None,
            trigger_kind: kind,
            trigger_json: trigger,
            dep_join: crate::types::DepJoin::All,
            next_due_at: None,
            last_run_at: None,
            pending_request_at: None,
            pending_origin: None,
            pending_actor: None,
            reflection: crate::types::Reflection::Ask,
            created_at: 0,
            updated_at: 0,
            deleted_at: None,
        }
    }

    fn ms_of(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        Local
            .from_local_datetime(
                &NaiveDate::from_ymd_opt(y, mo, d)
                    .unwrap()
                    .and_hms_opt(h, mi, 0)
                    .unwrap(),
            )
            .single()
            .unwrap()
            .timestamp_millis()
    }

    // ─── next_due ─────────────────────────────────────────────────────────

    #[test]
    fn interval_anchors_to_grid_without_drift() {
        let anchor = ms_of(2026, 1, 1, 9, 0);
        let t = task(
            serde_json::json!({"mode":"interval","every_minutes":30,"anchor":anchor}),
            TriggerKind::Schedule,
        );
        // After 9:20 → next grid point 9:30.
        assert_eq!(
            next_due(&t, ms_of(2026, 1, 1, 9, 20)),
            Some(ms_of(2026, 1, 1, 9, 30))
        );
        // After 9:30 exactly → next grid point 10:00.
        assert_eq!(
            next_due(&t, ms_of(2026, 1, 1, 9, 30)),
            Some(ms_of(2026, 1, 1, 10, 0))
        );
        // Way past (7h) → still the next grid point, not a burst.
        let due = next_due(&t, ms_of(2026, 1, 1, 16, 20)).unwrap();
        let due_dt = chrono::DateTime::from_timestamp_millis(due)
            .unwrap()
            .with_timezone(&Local);
        assert_eq!(due_dt.minute() % 30, 0);
    }

    #[test]
    fn daily_moves_to_tomorrow_after_time_passes() {
        let t = task(
            serde_json::json!({"mode":"daily","time":"09:00"}),
            TriggerKind::Schedule,
        );
        // 2026-01-05 08:00 → same day 09:00.
        assert_eq!(
            next_due(&t, ms_of(2026, 1, 5, 8, 0)),
            Some(ms_of(2026, 1, 5, 9, 0))
        );
        // 2026-01-05 09:00 → tomorrow 09:00.
        assert_eq!(
            next_due(&t, ms_of(2026, 1, 5, 9, 0)),
            Some(ms_of(2026, 1, 6, 9, 0))
        );
    }

    #[test]
    fn weekly_picks_next_selected_weekday() {
        let t = task(
            serde_json::json!({"mode":"weekly","days":["mon","wed","fri"],"time":"10:00"}),
            TriggerKind::Schedule,
        );
        // 2026-01-05 is Monday 09:00 → Monday 10:00.
        assert_eq!(
            next_due(&t, ms_of(2026, 1, 5, 9, 0)),
            Some(ms_of(2026, 1, 5, 10, 0))
        );
        // Monday 10:00 → Wednesday 10:00.
        assert_eq!(
            next_due(&t, ms_of(2026, 1, 5, 10, 0)),
            Some(ms_of(2026, 1, 7, 10, 0))
        );
        // Friday 10:00 → next Monday 10:00.
        assert_eq!(
            next_due(&t, ms_of(2026, 1, 9, 10, 0)),
            Some(ms_of(2026, 1, 12, 10, 0))
        );
    }

    #[test]
    fn monthly_clamps_short_month_to_last_day() {
        let t = task(
            serde_json::json!({"mode":"monthly","day":31,"time":"09:00"}),
            TriggerKind::Schedule,
        );
        // After 2026-01-15 → 2026-01-31.
        assert_eq!(
            next_due(&t, ms_of(2026, 1, 15, 9, 0)),
            Some(ms_of(2026, 1, 31, 9, 0))
        );
        // After 2026-01-31 → 2026-02-28 (Feb has 28 days in 2026).
        assert_eq!(
            next_due(&t, ms_of(2026, 1, 31, 9, 0)),
            Some(ms_of(2026, 2, 28, 9, 0))
        );
        // After 2026-02-28 → 2026-03-31.
        assert_eq!(
            next_due(&t, ms_of(2026, 2, 28, 9, 0)),
            Some(ms_of(2026, 3, 31, 9, 0))
        );
    }

    #[test]
    fn once_in_past_returns_none() {
        let t = task(
            serde_json::json!({"mode":"once","date":"2026-01-01","time":"09:00"}),
            TriggerKind::Schedule,
        );
        assert_eq!(next_due(&t, ms_of(2026, 1, 2, 0, 0)), None);
    }

    #[test]
    fn manual_has_no_next_due() {
        let t = task(serde_json::json!({}), TriggerKind::Manual);
        assert_eq!(next_due(&t, ms_of(2026, 1, 1, 0, 0)), None);
    }

    // ─── join_claim ───────────────────────────────────────────────────────

    fn dep(task: &str, upstream: &str, on: DepOn) -> TaskDep {
        TaskDep {
            task_id: task.into(),
            upstream_task_id: upstream.into(),
            on,
        }
    }

    fn state(task: &str, upstream: &str, run: Option<&str>) -> TaskDepState {
        TaskDepState {
            task_id: task.into(),
            upstream_task_id: upstream.into(),
            satisfied_run_id: run.map(|s| s.to_string()),
            satisfied_at: run.map(|_| 1_000),
        }
    }

    #[test]
    fn join_all_fires_when_every_edge_satisfied() {
        let mut t = task(serde_json::json!({}), TriggerKind::Manual);
        t.dep_join = crate::types::DepJoin::All;
        let deps = vec![dep("C", "A", DepOn::Success), dep("C", "B", DepOn::Success)];
        // Only A satisfied → no fire.
        let states = vec![state("C", "A", Some("trn_a")), state("C", "B", None)];
        assert!(join_claim(&t, &deps, &states).is_none());
        // Both satisfied → fire, consume both.
        let states = vec![
            state("C", "A", Some("trn_a")),
            state("C", "B", Some("trn_b")),
        ];
        let consume = join_claim(&t, &deps, &states).unwrap();
        assert_eq!(consume.len(), 2);
    }

    #[test]
    fn join_any_fires_on_first_edge() {
        let mut t = task(serde_json::json!({}), TriggerKind::Manual);
        t.dep_join = crate::types::DepJoin::Any;
        let deps = vec![dep("C", "A", DepOn::Success), dep("C", "B", DepOn::Failure)];
        let states = vec![state("C", "A", None), state("C", "B", Some("trn_b"))];
        let consume = join_claim(&t, &deps, &states).unwrap();
        assert_eq!(consume, vec![("B".to_string(), "trn_b".to_string())]);
    }

    #[test]
    fn run_satisfies_edge_conditions() {
        assert!(run_satisfies(DepOn::Success, RunStatus::Completed));
        assert!(!run_satisfies(DepOn::Success, RunStatus::Failed));
        assert!(run_satisfies(DepOn::Failure, RunStatus::Failed));
        assert!(!run_satisfies(DepOn::Failure, RunStatus::Completed));
        assert!(run_satisfies(DepOn::Completed, RunStatus::Completed));
        assert!(run_satisfies(DepOn::Completed, RunStatus::Failed));
        assert!(!run_satisfies(DepOn::Completed, RunStatus::Skipped));
    }

    // ─── cycle check ──────────────────────────────────────────────────────

    #[test]
    fn detects_self_cycle() {
        let deps = vec![dep("A", "A", DepOn::Success)];
        assert!(would_cycle("A", &deps));
    }

    #[test]
    fn detects_indirect_cycle() {
        let deps = vec![
            dep("B", "A", DepOn::Success),
            dep("C", "B", DepOn::Success),
            dep("A", "C", DepOn::Success),
        ];
        assert!(would_cycle("A", &deps));
    }

    #[test]
    fn acyclic_chain_is_fine() {
        let deps = vec![dep("B", "A", DepOn::Success), dep("C", "B", DepOn::Success)];
        assert!(!would_cycle("A", &deps));
        assert!(!would_cycle("C", &deps));
    }

    // ─── truncate ─────────────────────────────────────────────────────────

    #[test]
    fn truncate_keeps_head_and_tail() {
        let short = "short";
        assert_eq!(truncate(short, 100), "short");
        let body = "a".repeat(200);
        let text = format!("{body}END-MARKER");
        let out = truncate(&text, 100);
        assert!(out.len() <= 100, "len {} > 100", out.len());
        assert!(out.contains('…'));
        assert!(out.starts_with("aaa"));
        assert!(out.ends_with("END-MARKER"));
    }

    // ─── upstream block ───────────────────────────────────────────────────

    #[test]
    fn upstream_block_keeps_index_when_budget_exhausted() {
        let mut sources = Vec::new();
        for i in 0..20 {
            sources.push(UpstreamSource {
                task_id: format!("tsk_{i}"),
                task_name: format!("task {i}"),
                run_id: format!("trn_{i}"),
                status: RunStatus::Completed,
                finished_at: Some(1_000),
                result_summary: Some("x".repeat(500)),
            });
        }
        let block = compose_upstream_block(&sources);
        // Every source keeps its index line even though 1200/20=60 < 500.
        for i in 0..20 {
            assert!(
                block.contains(&format!("upstream tsk_{i}")),
                "missing index for {i}"
            );
            assert!(
                block.contains(&format!("trn_{i}")),
                "missing run id for {i}"
            );
        }
        assert!(block.contains("future task output"));
    }
}
