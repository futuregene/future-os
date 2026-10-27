//! Deterministic kernel for `future-tasks`.
//!
//! Everything here is a pure function of the inputs: no I/O, no clock, no
//! agent. `next_due` computes schedule anchors, `prompt_change_revisions`
//! decides what a prompt edit must record, and `truncate` is the shared
//! byte-budget truncator.

use crate::types::Task;
use chrono::{Datelike, NaiveDate};
use chrono_tz::Tz;
use serde::Deserialize;

/// The status a stored row carried when it was a *suggestion*. Nothing writes it
/// any more (the prompt-suggestion pass is gone), but a database from before the
/// removal can still hold such rows and the version writer has to see them for
/// what they are: not a version of the prompt.
pub const LEGACY_PROPOSED_STATUS: &str = "proposed";
/// Stored `result_summary` cap (head…tail).
pub const RESULT_SUMMARY_CHARS: usize = 2_000;
/// Cap on the answer saved alongside a run (`task_runs.result_text`). The
/// summary above is what lists and downstream runs read; this is the whole
/// answer, kept so a run stays readable after its conversation is gone —
/// head…tail, and large enough for a report rather than a paragraph.
pub const RUN_OUTPUT_CHARS: usize = 16_000;
/// Cap on the one-line "where did the output go" note (`task_runs.finish_note`).
pub const FINISH_NOTE_CHARS: usize = 400;

/// The ledger's `finish_reason` for a run that ended with `answer`.
///
/// Whitespace-only counts as empty: a terminal that printed two newlines is the
/// same silence as one that printed nothing, and treating it as an answer is
/// exactly how an idle sweep passed as a finished one.
pub fn finish_reason_for_answer(answer: &str) -> &'static str {
    if answer.trim().is_empty() {
        crate::types::finish_reason::EMPTY_ANSWER
    } else {
        crate::types::finish_reason::ANSWER
    }
}

/// The ledger's `finish_reason` for a failed run, from the failure text the
/// prompt pipeline produces.
///
/// Those failures carry a stable `[CODE]` prefix (`[RUN_CANCELLED] …`) so the
/// cause survives the trip through a `String` error; the ledger must not
/// collapse "the agent was restarted under it" and "the provider refused the
/// request" into one word. An unrecognized error is `error` — never `answer`.
pub fn finish_reason_for_error(error: &str) -> &'static str {
    use crate::types::finish_reason as fr;
    let code = error
        .strip_prefix('[')
        .and_then(|rest| rest.split(']').next())
        .unwrap_or("");
    match code {
        "RUN_CANCELLED" | "RUN_QUEUED" => fr::CANCELLED,
        "INTERRUPTED_BY_RESTART" => fr::INTERRUPTED,
        "RESPONSE_TIMEOUT" => fr::TIMEOUT,
        "UPSTREAM_DISCONNECTED"
        | "PROVIDER_CANCELLED"
        | "MODEL_RESPONSE_ERROR"
        | "MODEL_CONTENT_FILTER"
        | "MODEL_PAUSED"
        | "OUTPUT_LIMIT"
        | "AGENT_FAILED" => fr::PROVIDER_ERROR,
        // `RESPONSE_UNCONFIRMED` is the agent's "ended without confirming
        // completion" — the run was cut off, not answered.
        "RESPONSE_UNCONFIRMED" => fr::INTERRUPTED,
        _ => fr::ERROR,
    }
}

/// The one-line note left beside a run that produced nothing usable.
///
/// A `session_retention = delete` task throws its conversation away after every
/// run, so this line is the whole of what the next reader gets: enough to know
/// whether the run got anywhere (did it call tools? did it stream any text?)
/// without keeping a transcript.
pub fn finish_note(answer_chars: usize, last_tool: Option<&str>) -> String {
    let tool = match last_tool {
        Some(name) if !name.trim().is_empty() => format!("last tool call: {name}"),
        _ => "last tool call: none".to_string(),
    };
    truncate(
        &format!("answer: {answer_chars} chars · {tool}"),
        FINISH_NOTE_CHARS,
    )
}

/// Truncate to `max` bytes, keeping the head (context) and tail (conclusion)
/// with a single `…` marker. Byte-budget safe for any UTF-8 content.
///
/// A `max` too small to hold the marker and at least one tail byte takes the
/// head alone: the subtraction below would otherwise underflow, and a budget
/// that cannot fit `…` cannot fit a tail either.
pub fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    const MARK: &str = "…";
    // The marker and at least one byte on each side of it must fit; below that
    // there is no room for a tail at all, so take the head alone.
    if max <= MARK.len() + 1 {
        return text[..floor_char_boundary(text, max)].to_string();
    }
    // Split what is left after the marker: 3/4 head (context), 1/4 tail
    // (conclusion). Deriving both from the remaining budget keeps
    // head + marker + tail == max with no underflow for any max.
    let budget = max - MARK.len();
    let head_bytes = budget * 3 / 4;
    let tail_bytes = budget - head_bytes;
    let head = &text[..floor_char_boundary(text, head_bytes)];
    let tail_start = floor_char_boundary(text, text.len() - tail_bytes);
    let tail = &text[tail_start..];
    format!("{head}{MARK}{tail}")
}

/// Back up to the nearest char boundary at or before `idx` (never past the end).
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
    Once {
        date: String,
        time: String,
    },
    Interval {
        every_minutes: i64,
        /// Where the cadence counts from. Absent (or 0) means "from now": the
        /// form that writes this schedule does not stamp a start, and a required
        /// field made every interval task unparseable — it was created, listed
        /// with no next slot, and never ran.
        #[serde(default)]
        anchor: i64,
    },
    Daily {
        time: String,
    },
    Weekly {
        days: Vec<String>,
        time: String,
    },
    Monthly {
        day: i64,
        time: String,
    },
    /// Once a year on `month`/`day`. A day that does not exist in that month
    /// clamps to the month's last day, the rule monthly mode already uses.
    Yearly {
        month: u32,
        day: i64,
        time: String,
    },
}

/// Parse `trigger_json` into a schedule mode (only for `trigger_kind=schedule`).
pub fn parse_schedule(trigger_json: &serde_json::Value) -> Option<ScheduleMode> {
    serde_json::from_value(trigger_json.clone()).ok()
}

/// Compute the next due epoch-ms strictly after `after_ms` for a schedule.
/// Returns `None` for `manual`, `once` already past, or an unparseable trigger.
///
/// The wall-clock times in `trigger_json` are in the task's own zone (see
/// [`crate::timezone::task_timezone`]), not the host's: a task set to `09:00` in
/// Shanghai fires at 09:00 Shanghai even on a machine whose own clock sits
/// elsewhere.
pub fn next_due(task: &Task, after_ms: i64) -> Option<i64> {
    if !task.enabled || task.deleted_at.is_some() {
        return None;
    }
    match task.trigger_kind {
        crate::types::TriggerKind::Manual => None,
        crate::types::TriggerKind::Schedule => {
            let mode = parse_schedule(&task.trigger_json)?;
            next_due_for_mode(&mode, after_ms, crate::timezone::task_timezone(task))
        }
    }
}

fn next_due_for_mode(mode: &ScheduleMode, after_ms: i64, tz: Tz) -> Option<i64> {
    use chrono::{DateTime, Duration, TimeZone};
    let after = DateTime::from_timestamp_millis(after_ms)?.with_timezone(&tz);
    match mode {
        ScheduleMode::Once { date, time } => {
            let dt = parse_date_time(date, time)?;
            let due = tz.from_local_datetime(&dt).earliest()?;
            (due.timestamp_millis() > after_ms).then(|| due.timestamp_millis())
        }
        ScheduleMode::Interval {
            every_minutes,
            anchor,
        } => {
            let step = Duration::minutes(*every_minutes)
                .num_milliseconds()
                .max(60_000);
            // `anchor <= after_ms` covers both "the anchor passed" and "there is
            // no anchor": with 0 the next slot is one step from now, which is
            // what "every N minutes" means when the cadence was just created.
            let anchor = if *anchor > 0 { *anchor } else { after_ms };
            let mut due = anchor;
            if due <= after_ms {
                due = anchor + ((after_ms - anchor) / step + 1) * step;
            }
            Some(due)
        }
        ScheduleMode::Daily { time } => {
            let t = parse_time(time)?;
            let due = next_calendar_time(&after, t, tz)?;
            (due.timestamp_millis() > after_ms).then(|| due.timestamp_millis())
        }
        ScheduleMode::Weekly { days, time } => {
            let t = parse_time(time)?;
            let day_nums: Vec<u32> = days.iter().filter_map(|d| weekday_num(d)).collect();
            if day_nums.is_empty() {
                return None;
            }
            (0..=7)
                .filter_map(|delta| weekly_candidate(after_ms, &day_nums, t, delta, tz))
                .filter(|cand| cand.timestamp_millis() > after_ms)
                .min()
                .map(|d| d.timestamp_millis())
        }
        ScheduleMode::Monthly { day, time } => {
            let t = parse_time(time)?;
            (0..=12)
                .filter_map(|delta| monthly_candidate(after_ms, *day, t, delta, tz))
                .filter(|cand| cand.timestamp_millis() > after_ms)
                .min()
                .map(|d| d.timestamp_millis())
        }
        ScheduleMode::Yearly { month, day, time } => {
            let t = parse_time(time)?;
            // Three years of candidates: the one this year (if still ahead), the
            // next, and one more so a clamped leap day cannot fall through.
            (0..=2)
                .filter_map(|delta| yearly_candidate(after_ms, *month, *day, t, delta, tz))
                .filter(|cand| cand.timestamp_millis() > after_ms)
                .min()
                .map(|d| d.timestamp_millis())
        }
    }
}

/// The `delta`-th weekday candidate for a weekly schedule, or `None` when that
/// day is not selected (or does not exist in the task's zone, e.g. a
/// spring-forward gap).
fn weekly_candidate(
    after_ms: i64,
    day_nums: &[u32],
    time: chrono::NaiveTime,
    delta: i64,
    tz: Tz,
) -> Option<chrono::DateTime<Tz>> {
    use chrono::{Duration, TimeZone};
    let after = chrono::DateTime::from_timestamp_millis(after_ms)?.with_timezone(&tz);
    let date = (after + Duration::days(delta)).date_naive();
    let matches = day_nums.contains(&date.weekday().num_days_from_monday());
    matches
        .then(|| tz.from_local_datetime(&date.and_time(time)).earliest())
        .flatten()
}

/// The `delta`-th monthly candidate, clamped to the month's last day when the
/// requested day does not exist in it (day 31 in February).
fn monthly_candidate(
    after_ms: i64,
    day: i64,
    time: chrono::NaiveTime,
    delta: i64,
    tz: Tz,
) -> Option<chrono::DateTime<Tz>> {
    use chrono::{DateTime, TimeZone};
    let after = DateTime::from_timestamp_millis(after_ms)?.with_timezone(&tz);
    let (y, m) = add_months(after.year(), after.month(), delta);
    let dom = day.min(days_in_month(y, m) as i64) as u32;
    let date = NaiveDate::from_ymd_opt(y, m, dom)?;
    tz.from_local_datetime(&date.and_time(time)).earliest()
}

/// The `delta`-th `month`/`day` at `time`, relative to the year `after_ms` is in
/// — in the task's own zone, since a year starts at a different instant
/// everywhere.
///
/// The month is checked before [`days_in_month`], which indexes the *next*
/// month and would panic on a 0 or 13 that a hand-edited `trigger_json` can
/// carry. A day below 1 is rejected here and a day past the month's end clamps,
/// matching monthly mode.
fn yearly_candidate(
    after_ms: i64,
    month: u32,
    day: i64,
    time: chrono::NaiveTime,
    delta: i64,
    tz: Tz,
) -> Option<chrono::DateTime<Tz>> {
    use chrono::{DateTime, TimeZone};
    if !(1..=12).contains(&month) || day < 1 {
        return None;
    }
    let after = DateTime::from_timestamp_millis(after_ms)?.with_timezone(&tz);
    let year = after.year() + delta as i32;
    let dom = day.min(days_in_month(year, month) as i64) as u32;
    let date = NaiveDate::from_ymd_opt(year, month, dom)?;
    tz.from_local_datetime(&date.and_time(time)).earliest()
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
    after: &chrono::DateTime<Tz>,
    time: chrono::NaiveTime,
    tz: Tz,
) -> Option<chrono::DateTime<Tz>> {
    use chrono::{Duration, TimeZone};
    let days = if after.time() >= time { 1 } else { 0 };
    let date = after.date_naive() + Duration::days(days);
    tz.from_local_datetime(&date.and_time(time)).earliest()
}

// ─── session retention ────────────────────────────────────────────────────

/// Whether the conversation a run used should be deleted once the run settles.
///
/// Pure, and the single place the combination is decided: the setting only
/// means something for a task that opens a conversation per run, so
/// `existing` + `delete` (which a hand-edited database or an older client could
/// produce) reads as `keep` rather than deleting the conversation the next run
/// is supposed to continue.
pub fn deletes_run_conversation(task: &Task) -> bool {
    task.session_retention == crate::types::SessionRetention::Delete
        && task.session_policy == crate::types::SessionPolicy::New
}

/// The revision rows a prompt change must leave behind, in insert order.
///
/// Returns an empty list when the prompt is unchanged, so callers can treat a
/// no-op edit as "nothing to record" without checking first.
///
/// The **outgoing** prompt is recorded whenever the history does not already
/// carry a row for the version being replaced. Without it the chain has a hole
/// at the start: a task's first prompt is the one nobody has a copy of, so
/// `prompt revert` could walk back through every change except the one that
/// actually got you out of trouble. Tasks written before this bookkeeping
/// existed are repaired the same way — the first edit backfills their v1.
///
/// `reason` is the caller's text, not the helper's: it shows up verbatim in both
/// UIs, so inventing English prose here would put an untranslated sentence in a
/// Chinese panel. Pass `None` when there is nothing to say beyond the source.
///
/// The contract: the caller applies the result, and takes the active version
/// and prompt from the last row. Callers must **not** advance
/// `task.prompt_version` themselves.
pub fn prompt_change_revisions(
    task: &Task,
    history: &[crate::types::PromptRevision],
    new_prompt: &str,
    source: &str,
    reason: Option<&str>,
    at_ms: i64,
) -> Vec<crate::types::PromptRevision> {
    if new_prompt == task.prompt {
        return Vec::new();
    }
    let mut rows = Vec::new();
    // Only a real version counts as "this version is already recorded". A
    // database written before the suggestion pass was removed can still hold a
    // `proposed` row, and a suggestion was never a version of the prompt, so it
    // must not make the writer skip keeping the version it would replace.
    let recorded = history
        .iter()
        .any(|r| r.version == task.prompt_version && r.status != LEGACY_PROPOSED_STATUS);
    if !recorded {
        rows.push(crate::types::PromptRevision {
            id: crate::types::new_revision_id(),
            task_id: task.id.clone(),
            version: task.prompt_version,
            prompt: task.prompt.clone(),
            // Who wrote it is not recorded anywhere, so the row says what is
            // certain instead of guessing: this version was replaced.
            source: crate::types::REVISION_SOURCE_SUPERSEDED.to_string(),
            status: crate::types::REVISION_STATUS_SUPERSEDED.to_string(),
            reason: None,
            confidence: None,
            source_run_id: None,
            created_at: at_ms,
        });
    }
    rows.push(crate::types::PromptRevision {
        id: crate::types::new_revision_id(),
        task_id: task.id.clone(),
        version: task.prompt_version + 1,
        prompt: new_prompt.to_string(),
        source: source.to_string(),
        status: crate::types::REVISION_STATUS_ACTIVE.to_string(),
        reason: reason.map(str::to_string),
        confidence: None,
        source_run_id: None,
        created_at: at_ms,
    });
    rows
}

/// `RunKind` as the envelope spells it (lowercase, like every other label).
fn run_kind_label(kind: crate::types::RunKind) -> &'static str {
    use crate::types::RunKind;
    match kind {
        RunKind::Main => "main",
        RunKind::Manual => "manual",
        RunKind::Chain => "chain",
    }
}

/// Epoch ms as an RFC 3339 stamp in the task's own zone; empty when
/// unrepresentable.
///
/// The offset is part of the value (`…+08:00`), so a reader in another zone can
/// still recover the instant; and reading it in the task's zone is what makes
/// `09:00` mean the same thing here as in the schedule it came from.
fn local_time(ms: i64, tz: Tz) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.with_timezone(&tz).to_rfc3339())
        .unwrap_or_default()
}

/// How this run's trigger reads in the envelope: `manual`, or the normalized
/// schedule as JSON (the same keys `trigger_json` stores, so a reader can go
/// from the envelope back to `future task show`).
fn trigger_label(task: &Task) -> String {
    match task.trigger_kind {
        crate::types::TriggerKind::Manual => "manual".to_string(),
        crate::types::TriggerKind::Schedule => task.trigger_json.to_string(),
    }
}

/// What the run actually is: where it works, what kind of conversation it
/// opens, whether it continues one, the zone its clock is read in, and the
/// model it spends on.
///
/// A `chat` task may carry no working directory at all (its conversation runs
/// in the conversation's own temporary workspace), so the path is omitted
/// rather than printed empty.
fn run_settings_line(task: &Task) -> String {
    let mut parts = Vec::new();
    if !task.cwd.trim().is_empty() {
        parts.push(format!("cwd={}", task.cwd.trim()));
    }
    // The zone is resolved, not `(default)`: a run that reports `18:00` should
    // be able to say which clock produced it, especially when it is not the
    // reader's own.
    parts.push(format!(
        "timezone={}",
        crate::timezone::task_timezone(task).name()
    ));
    parts.push(format!(
        "conversation={}",
        match task.conversation_mode {
            crate::types::ConversationMode::Chat => "chat",
            crate::types::ConversationMode::Workspace => "workspace",
        }
    ));
    parts.push(
        match task.session_policy {
            // The policy decides what the agent can assume about its context,
            // so it is stated with its consequence, not just its name.
            crate::types::SessionPolicy::New => "session=new (fresh conversation per run)",
            crate::types::SessionPolicy::Existing => {
                "session=existing (one conversation kept across runs, compacted before this one)"
            }
        }
        .to_string(),
    );
    parts.push(format!(
        "model={}",
        task.model_id.as_deref().unwrap_or("(app default)")
    ));
    parts.push(format!(
        "thinking={}",
        task.thinking_level.as_deref().unwrap_or("(app default)")
    ));
    parts.join(" | ")
}

/// Compose the run envelope header: schema banner + identity + this run's
/// context, loop-style (`── schema ──` then `key: value` lines) rather than the
/// single self-closing `<task … />` tag the first version emitted.
///
/// It is context, not instruction: the task's own prompt follows it, under the
/// [`INSTRUCTION_BANNER`], and every run closes with [`COMPLETION_CONTRACT`]
/// (see [`compose_run_prompt`]).
pub fn compose_envelope_header(
    task: &Task,
    kind: crate::types::RunKind,
    due_ms: Option<i64>,
    occurrence: Option<i64>,
) -> String {
    let mut out = format!("── {} ──\n", crate::types::TASK_ENVELOPE_SCHEMA_VERSION);
    out.push_str(&format!(
        "task: {} | id: {} | prompt-version: {}\n",
        task.name, task.id, task.prompt_version
    ));
    let tz = crate::timezone::task_timezone(task);
    let mut run = format!("run: kind={}", run_kind_label(kind));
    if let Some(due) = due_ms
        .map(|due| local_time(due, tz))
        .filter(|due| !due.is_empty())
    {
        run.push_str(&format!(" | due={due}"));
    }
    if let Some(n) = occurrence {
        run.push_str(&format!(" | occurrence={n}"));
    }
    out.push_str(&run);
    out.push('\n');
    out.push_str(&format!("trigger: {}\n", trigger_label(task)));
    out.push_str(&format!("run settings: {}\n", run_settings_line(task)));
    out.push('\n');
    out
}

/// Banner between the context blocks and the task's own prompt.
pub const INSTRUCTION_BANNER: &str = "Instruction:\n";

/// The footer every run ends with.
///
/// A task run is unattended and its final message *is* the run's result (the
/// ledger stores it as `result_summary`), so the envelope says both outright:
/// nobody can answer a question mid-run, and the closing message should carry
/// what a reader of the ledger needs.
pub const COMPLETION_CONTRACT: &str = "\n\nCompletion contract:\n\
- This run is unattended: nobody answers mid-run. Make a reasonable choice, record it, and carry on — do not end by asking a question.\n\
- Close with a short report: it becomes this run's summary. State what you did, the artifacts and paths, what is new versus earlier runs, and what is still uncertain.\n";

/// The whole prompt for one run: envelope + the task's prompt + the completion
/// contract.
///
/// The pieces are public on their own (the header has its own tests and
/// reasons), but a host should compose through this so the order and the
/// separators exist in exactly one place.
///
/// `started_at_ms` anchors every run that has no scheduled occurrence. Only
/// `RunKind::Main` has a `due_at`; a manual, test or legacy chain run has none,
/// and a task prompt derives its target date from the envelope — so without the
/// fallback those runs would have to guess from the wall clock, which is exactly
/// what the prompts tell the agent not to do. For those kinds the run's own
/// start *is* the occurrence.
pub fn compose_run_prompt(
    task: &Task,
    kind: crate::types::RunKind,
    due_ms: Option<i64>,
    occurrence: Option<i64>,
    started_at_ms: i64,
) -> String {
    let mut out = compose_envelope_header(task, kind, due_ms.or(Some(started_at_ms)), occurrence);
    out.push_str(INSTRUCTION_BANNER);
    out.push_str(&task.prompt);
    out.push_str(COMPLETION_CONTRACT);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{SessionPolicy, SessionRetention, Task, TriggerKind};
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
            timezone: None,
            session_policy: SessionPolicy::New,
            conversation_mode: crate::types::ConversationMode::Workspace,
            thread_id: None,
            trigger_kind: kind,
            trigger_json: trigger,
            next_due_at: None,
            pending_request_at: None,
            pending_origin: None,
            pending_actor: None,
            session_retention: SessionRetention::Keep,
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

    /// The same wall-clock reading in an explicit zone.
    fn ms_in(tz: Tz, y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        tz.with_ymd_and_hms(y, mo, d, h, mi, 0)
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

    /// Yearly fires once a year on its month/day, and rolls to next year once
    /// the slot has passed.
    #[test]
    fn yearly_fires_once_a_year() {
        let t = task(
            serde_json::json!({"mode":"yearly","month":12,"day":31,"time":"22:00"}),
            TriggerKind::Schedule,
        );
        // From mid-year → this year's slot.
        assert_eq!(
            next_due(&t, ms_of(2026, 6, 1, 0, 0)),
            Some(ms_of(2026, 12, 31, 22, 0))
        );
        // Just before it → still this year's.
        assert_eq!(
            next_due(&t, ms_of(2026, 12, 31, 21, 59)),
            Some(ms_of(2026, 12, 31, 22, 0))
        );
        // Exactly on the slot → next year's, so the run does not repeat.
        assert_eq!(
            next_due(&t, ms_of(2026, 12, 31, 22, 0)),
            Some(ms_of(2027, 12, 31, 22, 0))
        );
    }

    /// A yearly day the month cannot hold clamps to the month's last day — the
    /// rule monthly mode already uses — so 29 February runs on the 28th in a
    /// common year and back on the 29th in a leap year.
    #[test]
    fn yearly_clamps_a_leap_day() {
        let t = task(
            serde_json::json!({"mode":"yearly","month":2,"day":29,"time":"09:00"}),
            TriggerKind::Schedule,
        );
        // 2026 is not a leap year.
        assert_eq!(
            next_due(&t, ms_of(2026, 1, 1, 0, 0)),
            Some(ms_of(2026, 2, 28, 9, 0))
        );
        // 2028 is.
        assert_eq!(
            next_due(&t, ms_of(2027, 6, 1, 0, 0)),
            Some(ms_of(2028, 2, 29, 9, 0))
        );
    }

    /// The earliest upcoming occurrence wins, whether that is this year or next.
    #[test]
    fn yearly_picks_the_earliest_upcoming_occurrence() {
        let t = task(
            serde_json::json!({"mode":"yearly","month":3,"day":31,"time":"08:00"}),
            TriggerKind::Schedule,
        );
        assert_eq!(
            next_due(&t, ms_of(2026, 3, 1, 0, 0)),
            Some(ms_of(2026, 3, 31, 8, 0))
        );
        assert_eq!(
            next_due(&t, ms_of(2026, 4, 1, 0, 0)),
            Some(ms_of(2027, 3, 31, 8, 0))
        );
    }

    /// Impossible yearly fields are refused rather than panicking the tick. The
    /// month is what matters: `days_in_month` indexes the month *after* it, so a
    /// 0 or 13 would reach `from_ymd_opt(..).unwrap()` and take the tick down.
    #[test]
    fn yearly_refuses_impossible_fields() {
        for (trigger, why) in [
            (
                serde_json::json!({"mode":"yearly","month":0,"day":1,"time":"09:00"}),
                "month 0",
            ),
            (
                serde_json::json!({"mode":"yearly","month":13,"day":1,"time":"09:00"}),
                "month 13",
            ),
            (
                serde_json::json!({"mode":"yearly","month":6,"day":0,"time":"09:00"}),
                "day 0",
            ),
            (
                serde_json::json!({"mode":"yearly","month":6,"day":-3,"time":"09:00"}),
                "negative day",
            ),
            (
                serde_json::json!({"mode":"yearly","month":6,"day":1,"time":"nope"}),
                "unparseable time",
            ),
            (
                serde_json::json!({"mode":"yearly","month":6,"day":1,"time":""}),
                "empty time",
            ),
        ] {
            let t = task(trigger, TriggerKind::Schedule);
            assert_eq!(next_due(&t, ms_of(2026, 6, 1, 0, 0)), None, "{why}");
        }
    }

    /// The mode tag is what `trigger_json` carries, so it has to round-trip.
    #[test]
    fn yearly_parses_from_its_stored_trigger() {
        let parsed = parse_schedule(&serde_json::json!({
            "mode": "yearly", "month": 12, "day": 31, "time": "22:00"
        }));
        assert_eq!(
            parsed,
            Some(ScheduleMode::Yearly {
                month: 12,
                day: 31,
                time: "22:00".to_string(),
            })
        );
        // A missing field is not a yearly trigger.
        assert_eq!(
            parse_schedule(&serde_json::json!({"mode":"yearly","day":31,"time":"22:00"})),
            None
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

    #[test]
    fn a_disabled_or_deleted_task_never_schedules() {
        let base = task(
            serde_json::json!({"mode":"daily","time":"09:00"}),
            TriggerKind::Schedule,
        );
        let after = ms_of(2026, 1, 1, 0, 0);
        assert!(next_due(&base, after).is_some());

        let mut disabled = base.clone();
        disabled.enabled = false;
        assert_eq!(next_due(&disabled, after), None);

        let mut deleted = base.clone();
        deleted.deleted_at = Some(1);
        assert_eq!(next_due(&deleted, after), None);
    }

    #[test]
    fn weekly_accepts_weekend_days_and_rejects_unknown_names() {
        let after = ms_of(2026, 1, 5, 0, 0); // Monday
        let weekend = task(
            serde_json::json!({"mode":"weekly","days":["sat","sun"],"time":"10:00"}),
            TriggerKind::Schedule,
        );
        // Saturday of that week.
        assert_eq!(next_due(&weekend, after), Some(ms_of(2026, 1, 10, 10, 0)));

        // Every name unknown → no candidate at all, rather than "every day".
        let bogus = task(
            serde_json::json!({"mode":"weekly","days":["noday"],"time":"10:00"}),
            TriggerKind::Schedule,
        );
        assert_eq!(next_due(&bogus, after), None);
    }

    #[test]
    fn an_out_of_range_monthly_day_has_no_candidate() {
        let after = ms_of(2026, 1, 5, 0, 0);
        // Day 0 (and anything below 1) is not a date, so every month is skipped.
        let zero = task(
            serde_json::json!({"mode":"monthly","day":0,"time":"09:00"}),
            TriggerKind::Schedule,
        );
        assert_eq!(next_due(&zero, after), None);
    }

    #[test]
    fn an_unparsable_date_or_time_schedules_nothing() {
        let after = ms_of(2026, 1, 1, 0, 0);
        for trigger in [
            serde_json::json!({"mode":"once","date":"not-a-date","time":"09:00"}),
            serde_json::json!({"mode":"once","date":"2026-12-24","time":"9am"}),
            serde_json::json!({"mode":"daily","time":"25:00"}),
            serde_json::json!({"mode":"monthly","day":5,"time":"nope"}),
            serde_json::json!({"mode":"weekly","days":["mon"],"time":""}),
            serde_json::json!({"mode":"nonsense"}),
        ] {
            let t = task(trigger.clone(), TriggerKind::Schedule);
            assert_eq!(next_due(&t, after), None, "{trigger}");
        }
    }

    // ─── time zone ────────────────────────────────────────────────────────

    /// The same schedule fires at its wall-clock time in the task's zone, not
    /// the host's: `09:00` is 01:00 UTC in Shanghai and 13:00 UTC in New York.
    #[test]
    fn a_daily_schedule_fires_on_the_tasks_own_clock() {
        let shanghai = ms_in(Tz::Asia__Shanghai, 2026, 1, 5, 8, 0);
        let mut t = task(
            serde_json::json!({"mode":"daily","time":"09:00"}),
            TriggerKind::Schedule,
        );
        t.timezone = Some("Asia/Shanghai".into());
        assert_eq!(
            next_due(&t, shanghai),
            Some(ms_in(Tz::Asia__Shanghai, 2026, 1, 5, 9, 0)),
            "09:00 Shanghai is 01:00 UTC"
        );
        assert_eq!(
            next_due(&t, shanghai),
            Some(ms_in(Tz::UTC, 2026, 1, 5, 1, 0))
        );

        // The same instant in another zone: the task still waits for 09:00 its
        // own time, six hours after New York's 09:00.
        t.timezone = Some("America/New_York".into());
        assert_eq!(
            next_due(&t, shanghai),
            Some(ms_in(Tz::America__New_York, 2026, 1, 5, 9, 0)),
            "the zone on the task decides, not the host's"
        );
        assert_eq!(
            next_due(&t, shanghai),
            Some(ms_in(Tz::UTC, 2026, 1, 5, 14, 0))
        );
    }

    /// The zone's own DST rules apply. A daily 09:00 in New York moves with the
    /// clock change, which a fixed offset could never express.
    #[test]
    fn a_zone_with_dst_follows_its_own_offset() {
        let mut t = task(
            serde_json::json!({"mode":"daily","time":"09:00"}),
            TriggerKind::Schedule,
        );
        t.timezone = Some("America/New_York".into());
        // 2026-03-08 is the US spring-forward day: 09:00 is EDT (UTC-4).
        let winter = ms_in(Tz::America__New_York, 2026, 3, 7, 8, 0);
        let due = next_due(&t, winter).unwrap();
        assert_eq!(
            due,
            ms_in(Tz::UTC, 2026, 3, 7, 14, 0),
            "the day before the change is EST (UTC-5)"
        );
        let summer = ms_in(Tz::America__New_York, 2026, 3, 9, 8, 0);
        assert_eq!(
            next_due(&t, summer),
            Some(ms_in(Tz::UTC, 2026, 3, 9, 13, 0)),
            "after the change it is EDT (UTC-4)"
        );
    }

    /// An explicit zone whose spring-forward gap swallows the scheduled time
    /// yields no candidate that day (the same rule the host's zone always had),
    /// rather than a shifted run.
    #[test]
    fn a_time_that_does_not_exist_in_the_zone_is_skipped() {
        let mut t = task(
            serde_json::json!({"mode":"daily","time":"02:30"}),
            TriggerKind::Schedule,
        );
        t.timezone = Some("America/New_York".into());
        // 2026-03-08 02:00–02:59 does not exist in New York.
        let before = ms_in(Tz::America__New_York, 2026, 3, 8, 1, 0);
        assert_eq!(next_due(&t, before), None);
    }

    /// The envelope's `due=` is read on the task's clock, with its offset — a
    /// manual run still gets an anchor, and a zone other than the host's must
    /// not be rendered as if it were the host's.
    #[test]
    fn the_envelope_renders_due_on_the_tasks_clock() {
        let mut t = task(serde_json::json!({}), TriggerKind::Manual);
        t.timezone = Some("Asia/Shanghai".into());
        let due = ms_in(Tz::UTC, 2026, 10, 7, 1, 0);
        let header = compose_run_prompt(&t, crate::types::RunKind::Main, Some(due), None, due);
        assert!(header.contains("timezone=Asia/Shanghai"), "{header}");
        assert!(
            header.contains("due=2026-10-07T09:00:00+08:00"),
            "09:00 Shanghai, not 01:00 UTC: {header}"
        );

        // A task with no stored zone still names the one it resolved to.
        let bare = task(serde_json::json!({}), TriggerKind::Manual);
        let resolved = crate::timezone::task_timezone(&bare).name().to_string();
        assert!(
            compose_envelope_header(&bare, crate::types::RunKind::Manual, None, None)
                .contains(&format!("timezone={resolved}")),
            "the resolved zone is stated"
        );
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

    #[test]
    fn truncate_never_splits_a_multibyte_character() {
        // Every boundary lands inside a 2-byte character, so the floor steps
        // must walk back to a real char boundary and the result must still be
        // valid UTF-8 within budget.
        let text = "é".repeat(200);
        for max in [1, 2, 3, 5, 7, 9, 11] {
            let out = truncate(&text, max);
            assert!(out.len() <= max, "max {max}: len {}", out.len());
            // A 1-byte budget cannot hold a 2-byte character: empty, never a
            // half-character (the panic this test exists to prevent).
            assert_eq!(out.is_empty(), max < 2, "max {max}: {out:?}");
        }
        // A budget too small for the marker takes the head alone.
        assert_eq!(truncate(&"é".repeat(10), 2), "é".to_string());
        assert_eq!(truncate(&"é".repeat(10), 3), "é".to_string());
        assert_eq!(truncate(&"ab".repeat(10), 1), "a".to_string());
    }

    // ─── envelope header ──────────────────────────────────────────────────

    #[test]
    fn the_envelope_header_carries_the_schema_and_optional_context() {
        let mut t = task(
            serde_json::json!({"mode":"daily","time":"09:00"}),
            TriggerKind::Schedule,
        );
        t.name = "daily report".into();
        t.id = "tsk_1".into();
        t.prompt_version = 4;
        t.cwd = "/tmp/repo".into();
        t.model_id = Some("future/deepseek-flash".into());
        t.thinking_level = Some("high".into());

        let bare = compose_envelope_header(&t, crate::types::RunKind::Main, None, None);
        assert!(
            bare.starts_with(&format!("── {} ──\n", crate::TASK_ENVELOPE_SCHEMA_VERSION)),
            "{bare}"
        );
        assert!(
            bare.contains("task: daily report | id: tsk_1 | prompt-version: 4"),
            "{bare}"
        );
        assert!(bare.contains("run: kind=main"), "{bare}");
        assert!(
            bare.contains("trigger: {\"mode\":\"daily\",\"time\":\"09:00\"}"),
            "{bare}"
        );
        assert!(
            bare.contains("run settings: cwd=/tmp/repo | timezone=",),
            "the run names the clock it reads: {bare}"
        );
        assert!(
            bare.contains(
                "conversation=workspace | session=new (fresh conversation per run) | model=future/deepseek-flash | thinking=high"
            ),
            "{bare}"
        );
        assert!(
            !bare.contains("due="),
            "a run with no due time omits it: {bare}"
        );
        assert!(!bare.contains("occurrence="), "{bare}");
        assert!(bare.ends_with("\n\n"), "blocks are separated: {bare}");
        // The header is context only: the instruction and the closing contract
        // are added by `compose_run_prompt`.
        assert!(!bare.contains(INSTRUCTION_BANNER), "{bare}");
        assert!(!bare.contains("Completion contract"), "{bare}");

        let full = compose_envelope_header(
            &t,
            crate::types::RunKind::Chain,
            Some(1_760_000_000_000),
            Some(7),
        );
        assert!(full.contains("run: kind=chain | due=2025-10-09T"), "{full}");
        assert!(full.contains(" | occurrence=7"), "{full}");

        // A manual task says so rather than printing an empty trigger object.
        let manual = compose_envelope_header(
            &task(serde_json::json!({}), TriggerKind::Manual),
            crate::types::RunKind::Manual,
            None,
            None,
        );
        assert!(manual.contains("trigger: manual"), "{manual}");

        // An unrepresentable epoch lands in the same "omit" branch as None.
        let absurd = compose_envelope_header(&t, crate::types::RunKind::Main, Some(i64::MAX), None);
        assert!(!absurd.contains("due="), "{absurd}");
    }

    #[test]
    fn a_chat_task_without_a_directory_carries_no_cwd_and_states_its_reuse() {
        let mut t = task(serde_json::json!({}), TriggerKind::Manual);
        t.cwd = String::new();
        t.conversation_mode = crate::types::ConversationMode::Chat;
        t.session_policy = SessionPolicy::Existing;
        t.model_id = None;
        t.thinking_level = None;
        let header = compose_envelope_header(&t, crate::types::RunKind::Manual, None, None);
        assert!(
            !header.contains("cwd="),
            "no empty path is printed: {header}"
        );
        assert!(header.contains("conversation=chat"), "{header}");
        assert!(header.contains("session=existing"), "{header}");
        assert!(
            header.contains("model=(app default) | thinking=(app default)"),
            "a task without a model says what it falls back to: {header}"
        );
    }

    #[test]
    fn the_run_prompt_wraps_the_task_prompt_in_instruction_and_contract() {
        let mut t = task(serde_json::json!({}), TriggerKind::Manual);
        t.prompt = "summarise the week".into();

        let plain = compose_run_prompt(&t, crate::types::RunKind::Manual, None, None, 0);
        assert!(
            plain.contains("Instruction:\nsummarise the week"),
            "{plain}"
        );
        assert!(plain.contains("Completion contract:"), "{plain}");
        assert!(
            plain.contains("nobody answers mid-run"),
            "the run is unattended: {plain}"
        );
        assert!(
            plain.contains("it becomes this run's summary"),
            "the closing message is the ledger's summary: {plain}"
        );
        // The contract closes the prompt: the instruction is above it, never
        // buried under a footer the agent might read first.
        assert!(plain.ends_with('\n'), "{plain}");
        assert!(
            plain.find("summarise the week").unwrap() < plain.find("Completion contract").unwrap(),
            "{plain}"
        );
    }

    /// An interval as the task form writes it — no `anchor` — still schedules.
    ///
    /// The form sends `{mode: "interval", every_minutes}`, so a required anchor
    /// made `parse_schedule` return `None`: the task was stored with no next
    /// slot and never fired, while "run once on a date" (whose fields the form
    /// does send) worked.
    #[test]
    fn an_interval_without_an_anchor_still_schedules() {
        let trigger = serde_json::json!({"mode": "interval", "every_minutes": 30});
        let mode = parse_schedule(&trigger).expect("the form's own payload parses");
        assert_eq!(
            mode,
            ScheduleMode::Interval {
                every_minutes: 30,
                anchor: 0
            }
        );

        let mut task = task(trigger, TriggerKind::Schedule);
        let now = 1_700_000_000_000_i64;
        let due = next_due(&task, now).expect("an interval has a next slot");
        assert_eq!(
            due,
            now + 30 * 60_000,
            "the first slot is one step from now"
        );

        // An explicit anchor keeps its own cadence.
        task.trigger_json = serde_json::json!({
            "mode": "interval",
            "every_minutes": 30,
            "anchor": now,
        });
        assert_eq!(next_due(&task, now + 1).unwrap(), now + 30 * 60_000);
        assert_eq!(
            next_due(&task, now + 31 * 60_000).unwrap(),
            now + 60 * 60_000
        );
    }

    #[test]
    fn every_run_kind_gets_a_date_anchor_in_the_envelope() {
        let t = task(serde_json::json!({}), TriggerKind::Manual);
        // A scheduled occurrence is the anchor when there is one.
        let scheduled = compose_run_prompt(
            &t,
            crate::types::RunKind::Main,
            Some(1_760_000_000_000),
            None,
            1_760_000_100_000,
        );
        assert!(scheduled.contains("due=20"), "{scheduled}");

        // A run with no scheduled occurrence of its own (manual, or a legacy
        // chain row) is anchored by its start time, so a prompt that derives its
        // target date from the envelope never has to guess from the wall clock.
        for kind in [crate::types::RunKind::Manual, crate::types::RunKind::Chain] {
            let run = compose_run_prompt(&t, kind, None, None, 1_760_000_000_000);
            assert!(
                run.contains("due=20"),
                "{kind:?} run has no date anchor: {run}"
            );
        }

        // The occurrence still wins when both are present (a late catch-up run
        // reports the slot it belongs to, not when it happened to fire).
        let late = compose_run_prompt(
            &t,
            crate::types::RunKind::Main,
            Some(1_760_000_000_000),
            None,
            1_760_090_000_000,
        );
        assert_eq!(
            due_field(&late),
            due_field(&scheduled),
            "a late run reports its slot, not its firing time"
        );
        assert_ne!(
            due_field(&late),
            due_field(&compose_run_prompt(
                &t,
                crate::types::RunKind::Manual,
                None,
                None,
                1_760_090_000_000,
            )),
            "the fallback really uses the start time"
        );
    }

    /// The `due=` value from an envelope header's `run:` line.
    fn due_field(prompt: &str) -> String {
        prompt
            .lines()
            .find_map(|line| line.strip_prefix("run: "))
            .and_then(|line| {
                line.split('|')
                    .find_map(|part| part.trim().strip_prefix("due="))
            })
            .unwrap_or_default()
            .to_string()
    }

    // ─── prompt versions ──────────────────────────────────────────────────

    fn revision(version: i64, prompt: &str) -> crate::types::PromptRevision {
        crate::types::PromptRevision {
            id: format!("rev_{version}"),
            task_id: "tsk_test".into(),
            version,
            prompt: prompt.into(),
            source: "user".into(),
            status: "active".into(),
            reason: None,
            confidence: None,
            source_run_id: None,
            created_at: 0,
        }
    }

    /// An unchanged prompt records nothing, so a caller can treat a no-op edit
    /// as "nothing to write" without checking first.
    #[test]
    fn an_unchanged_prompt_records_no_version() {
        let t = task(serde_json::json!({}), TriggerKind::Manual);
        assert!(prompt_change_revisions(&t, &[], "p", "user", Some("why"), 1).is_empty());
    }

    /// The first change records both the new prompt and the one it replaced.
    /// Without the second row the task's original prompt is gone, and
    /// `prompt revert` could never get back to where the task started.
    #[test]
    fn the_first_change_keeps_the_prompt_it_replaced() {
        let t = task(serde_json::json!({}), TriggerKind::Manual);
        let rows = prompt_change_revisions(&t, &[], "the new one", "user", Some("why"), 7);
        assert_eq!(rows.len(), 2, "the outgoing prompt is kept once");

        assert_eq!(rows[0].version, 1);
        assert_eq!(rows[0].prompt, "p", "the text the task started with");
        assert_eq!(rows[0].source, "superseded");
        assert_eq!(rows[0].status, "superseded");
        assert_eq!(
            rows[0].reason, None,
            "the helper does not invent reason prose: it would show up untranslated"
        );
        assert_eq!(rows[0].task_id, "tsk_test");

        assert_eq!(rows[1].version, 2);
        assert_eq!(rows[1].prompt, "the new one");
        assert_eq!(rows[1].source, "user");
        assert_eq!(rows[1].status, "active");
        assert_eq!(rows[1].reason.as_deref(), Some("why"));
        assert_eq!(rows[1].created_at, 7);
        assert!(
            rows.iter().all(|r| r.id != rows[0].id || r.version == 1),
            "each row gets its own id"
        );
        assert_ne!(rows[0].id, rows[1].id);
    }

    /// Once the history carries the version being replaced, only the new prompt
    /// is recorded — otherwise every edit would add another copy of the past.
    #[test]
    fn a_change_onto_a_recorded_version_only_adds_the_new_one() {
        let mut t = task(serde_json::json!({}), TriggerKind::Manual);
        t.prompt_version = 2;
        t.prompt = "second".into();
        let history = vec![revision(1, "p"), revision(2, "second")];
        let rows = prompt_change_revisions(&t, &history, "third", "user", Some("shorter"), 9);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].version, 3);
        assert_eq!(rows[0].prompt, "third");
        assert_eq!(rows[0].source, "user");
    }

    /// A task written before this bookkeeping existed has a gap at v1; the
    /// first edit backfills it, so the repair needs no migration.
    #[test]
    fn a_legacy_task_gets_its_missing_version_backfilled() {
        let mut t = task(serde_json::json!({}), TriggerKind::Manual);
        // v3 is current, but only v2 is in the history (v1 was never stored).
        t.prompt_version = 3;
        t.prompt = "third".into();
        let history = vec![revision(2, "second")];
        let rows = prompt_change_revisions(&t, &history, "fourth", "user", Some("why"), 11);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].version, 3, "the version being replaced");
        assert_eq!(rows[0].prompt, "third");
        assert_eq!(rows[1].version, 4);
    }

    /// Deleting the conversation is only meaningful for a task that opens one
    /// per run. The combination a hand-edited database or an older client could
    /// produce (reuse + delete) must not delete the conversation the next run is
    /// supposed to continue.
    #[test]
    fn only_a_per_run_conversation_is_deleted_after_the_run() {
        let mut t = task(serde_json::json!({}), TriggerKind::Manual);

        t.session_policy = SessionPolicy::New;
        t.session_retention = SessionRetention::Keep;
        assert!(!deletes_run_conversation(&t), "the default keeps it");

        t.session_retention = SessionRetention::Delete;
        assert!(deletes_run_conversation(&t));

        t.session_policy = SessionPolicy::Existing;
        assert!(
            !deletes_run_conversation(&t),
            "a reused conversation is what the next run continues"
        );
    }

    /// A version row that is not `active`/`superseded` (a suggestion recorded by
    /// a build from before the pass was removed) is not a version of the prompt:
    /// it must not make the writer think the current version is already recorded.
    #[test]
    fn a_stale_suggestion_row_does_not_hide_the_version_it_sits_on() {
        let mut t = task(serde_json::json!({}), TriggerKind::Manual);
        t.prompt_version = 2;
        t.prompt = "second".into();
        let mut suggested = revision(2, "a suggestion about v2");
        suggested.status = "proposed".to_string();
        suggested.source = crate::types::REVISION_SOURCE_REFLECTION.to_string();
        let rows = prompt_change_revisions(&t, &[suggested], "third", "user", None, 5);
        assert_eq!(
            rows.len(),
            2,
            "history keeps the replaced version: {rows:?}"
        );
        assert_eq!(rows[0].version, 2);
        assert_eq!(rows[0].prompt, "second");
        assert_eq!(rows[0].status, crate::types::REVISION_STATUS_SUPERSEDED);
    }
}

#[cfg(test)]
mod finish_reason_tests {
    use super::*;
    use crate::types::finish_reason as fr;

    /// A blank answer is not an answer. This is the case that used to reach the
    /// ledger as `completed` with an empty summary: the agent ended cleanly, so
    /// the run looked like a success with nothing in it.
    #[test]
    fn a_blank_answer_is_empty_not_an_answer() {
        assert_eq!(finish_reason_for_answer(""), fr::EMPTY_ANSWER);
        assert_eq!(finish_reason_for_answer("  \n\t\n "), fr::EMPTY_ANSWER);
        assert_eq!(finish_reason_for_answer("\u{a0}"), fr::EMPTY_ANSWER);
    }

    #[test]
    fn real_text_is_an_answer() {
        assert_eq!(finish_reason_for_answer("done"), fr::ANSWER);
        assert_eq!(finish_reason_for_answer(" 报告在 /tmp/x.md "), fr::ANSWER);
    }

    /// Each way a run can end carries its own token, so the ledger says *why*
    /// rather than only that it was not an answer.
    #[test]
    fn every_failure_code_maps_to_its_own_reason() {
        assert_eq!(
            finish_reason_for_error("[RUN_CANCELLED] run was cancelled"),
            fr::CANCELLED
        );
        assert_eq!(
            finish_reason_for_error("[INTERRUPTED_BY_RESTART] the Agent restarted"),
            fr::INTERRUPTED
        );
        assert_eq!(
            finish_reason_for_error("[RESPONSE_TIMEOUT] response timed out"),
            fr::TIMEOUT
        );
        assert_eq!(
            finish_reason_for_error("[MODEL_RESPONSE_ERROR] invalid model response"),
            fr::PROVIDER_ERROR
        );
        // An unknown failure is never mistaken for a success.
        assert_eq!(finish_reason_for_error("something else"), fr::ERROR);
        assert_eq!(finish_reason_for_error(""), fr::ERROR);
    }

    #[test]
    fn the_note_names_the_answer_size_and_the_last_tool() {
        assert_eq!(
            finish_note(0, None),
            "answer: 0 chars · last tool call: none"
        );
        assert_eq!(
            finish_note(1234, Some("shell")),
            "answer: 1234 chars · last tool call: shell"
        );
        // An empty tool name is "none", not a blank after the colon.
        assert_eq!(
            finish_note(0, Some("  ")),
            "answer: 0 chars · last tool call: none"
        );
    }
}
