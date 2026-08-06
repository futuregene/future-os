//! Markdown backfill (G-3) — reconstruct idempotent append-only events from
//! an ACTIVE_GOAL_STATE.md workbench, mirroring the reference
//! `backfill_todo_events_from_markdown` (`event_sourced_state.py`): todo
//! records carry `source_ref` / `source_section` / `source_line` provenance
//! and backfill ids `backfill-<suffix>-<digest>` so re-running the backfill
//! is idempotent. Read-only import: one-shot, never a two-way sync.
//!
//! Privacy (G-4): a `public_safe` backfill redacts todo text / evidence that
//! looks like private state; `local_private` preserves the workbench text.

use anyhow::{Context, Result};
use serde::Serialize;

use crate::projection::privacy::{redact, PrivacyLevel};
use crate::state::{Priority, TaskClass, Todo, TodoStatus};
use crate::store::{content_digest, Event};

pub const MARKDOWN_BACKFILL_PRODUCER: &str = "loopx.markdown_backfill";
pub const BACKFILL_SOURCE_REF_DEFAULT: &str = "ACTIVE_GOAL_STATE.md";

/// One todo record parsed from the markdown workbench (LoopX
/// `_markdown_todo_records`).
#[derive(Debug, Clone)]
pub struct MarkdownTodoRecord {
    pub role: String,
    pub source_section: String,
    pub source_line: u64,
    pub planner_order: u32,
    pub todo_id: Option<String>,
    pub status: String,
    pub text: String,
    pub task_class: Option<String>,
    pub action_kind: Option<String>,
    pub claimed_by: Option<String>,
    pub no_followup: bool,
    pub evidence: Option<String>,
    pub completed_at: Option<String>,
    pub updated_at: Option<String>,
    pub note: Option<String>,
    pub monitor_target: Option<String>,
    pub monitor_policy: Option<String>,
    pub cadence: Option<String>,
    pub goal_bound: bool,
    pub global_gate: bool,
}

/// URL-decode the subset reference uses in anchors (%20 → ' ', %2B → '+').
fn url_decode(value: &str) -> String {
    value.replace("%20", " ").replace("%2B", "+")
}

fn heading_role(heading: &str) -> Option<&'static str> {
    let normalized = heading
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    if normalized.starts_with("user todo") || normalized.contains("owner review") {
        return Some("user");
    }
    if normalized.starts_with("agent todo") {
        return Some("agent");
    }
    None
}

fn status_from_marker(marker: &str) -> String {
    match marker.trim() {
        "x" | "X" => "done".to_string(),
        "-" => "deferred".to_string(),
        _ => "open".to_string(),
    }
}

/// Parse `key=value` pairs from a metadata line. Handles both the full
/// `<!-- future-loop:todo ... -->` anchor and bare `key=value` continuations.
fn parse_metadata(line: &str) -> Vec<(String, String)> {
    let text = line
        .trim()
        .trim_start_matches("<!--")
        .trim_end_matches("-->")
        .trim();
    let mut out = vec![];
    for token in text.split_whitespace() {
        if let Some((key, value)) = token.split_once('=') {
            out.push((key.trim().to_string(), value.trim().to_string()));
        }
    }
    out
}

/// Parse the markdown workbench into todo records (LoopX
/// `_markdown_todo_records`): section headings set the role; `- [marker]`
/// lines start records; indented metadata lines attach anchor fields.
pub fn parse_markdown_todos(state_text: &str) -> Vec<MarkdownTodoRecord> {
    let mut records: Vec<MarkdownTodoRecord> = vec![];
    let mut role: Option<String> = None;
    let mut source_section: Option<String> = None;
    // Index of the record currently receiving metadata/continuation updates
    // (reference appends the record dict at creation and mutates it in place).
    let mut current: Option<usize> = None;
    let mut role_indexes = std::collections::HashMap::<String, u32>::new();

    for (line_number, line) in state_text.lines().enumerate() {
        let line_number = (line_number + 1) as u64;
        if let Some(heading) = line.strip_prefix("##") {
            let heading = heading.trim();
            source_section = Some(heading.to_string());
            role = heading_role(heading).map(|r| r.to_string());
            current = None;
            continue;
        }
        let (Some(role_name), Some(section)) = (&role, &source_section) else {
            continue;
        };
        // `- [ ] text` / `- [x] text` / `- [-] text`
        if let Some(rest) = line.trim_start().strip_prefix("- [") {
            let Some((marker, text)) = rest.split_once(']') else {
                continue;
            };
            let text = text.trim_start().trim_start_matches(' ').to_string();
            if text.is_empty() {
                continue;
            }
            let index = role_indexes.entry(role_name.clone()).or_insert(0);
            *index += 1;
            let record = MarkdownTodoRecord {
                role: role_name.clone(),
                source_section: section.clone(),
                source_line: line_number,
                planner_order: *index,
                todo_id: None,
                status: status_from_marker(marker),
                text,
                task_class: None,
                action_kind: None,
                claimed_by: None,
                no_followup: false,
                evidence: None,
                completed_at: None,
                updated_at: None,
                note: None,
                monitor_target: None,
                monitor_policy: None,
                cadence: None,
                goal_bound: false,
                global_gate: false,
            };
            records.push(record);
            current = Some(records.len() - 1);
            continue;
        }
        let Some(record) = current.and_then(|i| records.get_mut(i)) else {
            continue;
        };
        // Metadata / continuation lines must be indented (anchor or key=value).
        if !line.starts_with(' ') && !line.starts_with('\t') {
            current = None;
            continue;
        }
        let metadata = parse_metadata(line);
        if metadata.is_empty() {
            // Continuation line — append to the todo text.
            let continuation = line.trim();
            if !continuation.is_empty() {
                record.text = format!("{} {}", record.text, continuation);
            }
            continue;
        }
        for (key, value) in metadata {
            let value = url_decode(&value);
            match key.as_str() {
                "todo_id" => record.todo_id = Some(value),
                "status" => record.status = value,
                "task_class" => record.task_class = Some(value),
                "action_kind" => record.action_kind = Some(value),
                "claimed_by" => record.claimed_by = Some(value),
                "no_followup" => record.no_followup = value == "true",
                "evidence" => record.evidence = Some(value),
                "completed_at" => record.completed_at = Some(value),
                "updated_at" => record.updated_at = Some(value),
                "note" => record.note = Some(value),
                "monitor_target" => record.monitor_target = Some(value),
                "monitor_policy" => record.monitor_policy = Some(value),
                "cadence" => record.cadence = Some(value),
                "goal_bound" => record.goal_bound = value == "true",
                "global_gate" => record.global_gate = value == "true",
                _ => {}
            }
        }
    }
    records
}

/// Backfill event id (reference `_backfill_event_id`): sha-style digest over
/// `goal|todo|suffix`, prefixed `backfill-<suffix>-`.
pub fn backfill_event_id(goal_id: &str, todo_id: &str, suffix: &str) -> String {
    let digest = content_digest(format!("{goal_id}|{todo_id}|{suffix}").as_bytes());
    format!("backfill-{suffix}-{}", &digest[..16])
}

/// A generated backfill event + its provenance (for append_with_meta).
#[derive(Debug, Clone, Serialize)]
pub struct BackfillEvent {
    pub event: Event,
    pub event_id: String,
    pub source_ref: String,
    pub source_section: String,
    pub source_line: u64,
    pub privacy: PrivacyLevel,
}

/// The outcome of a backfill run.
#[derive(Debug, Clone, Serialize)]
pub struct BackfillOutcome {
    pub goal_id: String,
    pub todo_count: usize,
    pub event_count: usize,
    pub events: Vec<BackfillEvent>,
}

fn todo_id_for(record: &MarkdownTodoRecord, goal_id: &str) -> String {
    if let Some(id) = record.todo_id.as_deref().filter(|id| !id.is_empty()) {
        return id.to_string();
    }
    let digest = content_digest(
        format!(
            "{goal_id}|{}|{}|{}",
            record.role, record.planner_order, record.text
        )
        .as_bytes(),
    );
    format!("todo-{}", &digest[..16])
}

fn task_class_for(record: &MarkdownTodoRecord) -> TaskClass {
    let class = record
        .task_class
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match class.as_str() {
        "user_gate" => TaskClass::UserGate,
        "user_action" => TaskClass::UserAction,
        "continuous_monitor" | "monitor" => TaskClass::Monitor,
        "blocker" => TaskClass::Blocker,
        _ => {
            if record.role == "user" {
                TaskClass::UserGate
            } else {
                TaskClass::Advancement
            }
        }
    }
}

fn priority_from_text(text: &str) -> Priority {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.starts_with("[P0]") {
        Priority::P0
    } else if compact.starts_with("[P2]") {
        Priority::P2
    } else {
        Priority::P1
    }
}

fn parse_anchor_epoch(value: &str) -> Option<u64> {
    crate::scheduler::state::parse_epoch(value)
}

fn build_todo(record: &MarkdownTodoRecord, goal_id: &str, privacy: PrivacyLevel) -> Todo {
    let id = todo_id_for(record, goal_id);
    let text = redact(&record.text, privacy);
    let class = task_class_for(record);
    let mut todo = Todo::advancement(&id, &text);
    todo.class = class;
    todo.title = text.clone();
    todo.priority = priority_from_text(&text);
    if let Some(ak) = &record.action_kind {
        todo.action_kind = Some(ak.clone());
    }
    if let Some(owner) = &record.claimed_by {
        todo.claimed_by = Some(owner.clone());
    }
    if let Some(evidence) = &record.evidence {
        todo.evidence = Some(redact(evidence, privacy));
    }
    if let Some(note) = &record.note {
        todo.note = Some(redact(note, privacy));
    }
    todo.no_follow_up = record.no_followup;
    todo.goal_bound = record.goal_bound;
    todo.global_gate = record.global_gate;
    if let Some(target) = &record.monitor_target {
        todo.monitor_target = Some(target.clone());
    }
    if let Some(policy) = &record.monitor_policy {
        todo.monitor_policy = Some(policy.clone());
    }
    if let Some(cadence) = &record.cadence {
        todo.monitor_cadence = Some(cadence.clone());
    }
    if let Some(ts) = record.completed_at.as_deref().and_then(parse_anchor_epoch) {
        todo.completed_at = Some(ts);
    }
    if let Some(ts) = record.updated_at.as_deref().and_then(parse_anchor_epoch) {
        todo.updated_at = ts;
    }
    match record.status.as_str() {
        "done" => {
            todo.status = TodoStatus::Done;
            if todo.completed_at.is_none() {
                todo.completed_at = Some(crate::state::now_epoch());
            }
        }
        "deferred" => todo.status = TodoStatus::Deferred,
        "blocked" => todo.status = TodoStatus::Blocked,
        _ => {}
    }
    todo
}

/// Convert markdown workbench todos into idempotent backfill events (LoopX
/// `backfill_todo_events_from_markdown`): TodoAdded (+ TodoClaimed when a
/// claim is recorded, + TodoCompleted when done). `privacy` selects the
/// grading lens for text/evidence redaction. Never mutates the source.
pub fn backfill_todo_events(
    state_text: &str,
    goal_id: &str,
    privacy: PrivacyLevel,
) -> Result<BackfillOutcome> {
    if goal_id.trim().is_empty() {
        anyhow::bail!("goal_id is required");
    }
    let records = parse_markdown_todos(state_text);
    if records.is_empty() {
        anyhow::bail!("no todo records found in markdown (expected `- [ ]` lines under ## Agent Todo / ## User Todo headings)");
    }
    let mut events: Vec<BackfillEvent> = vec![];
    let mut todo_count = 0usize;
    for record in &records {
        let todo_id = todo_id_for(record, goal_id);
        let todo = build_todo(record, goal_id, privacy);
        todo_count += 1;

        let anchor_ts = record
            .updated_at
            .as_deref()
            .and_then(parse_anchor_epoch)
            .unwrap_or(crate::state::now_epoch());

        events.push(BackfillEvent {
            event_id: backfill_event_id(goal_id, &todo_id, "add"),
            source_ref: BACKFILL_SOURCE_REF_DEFAULT.to_string(),
            source_section: record.source_section.clone(),
            source_line: record.source_line,
            privacy,
            event: Event::TodoAdded {
                goal_id: goal_id.to_string(),
                todo,
                ts: anchor_ts,
            },
        });
        if let Some(owner) = record.claimed_by.clone() {
            events.push(BackfillEvent {
                event_id: backfill_event_id(goal_id, &todo_id, "claim"),
                source_ref: BACKFILL_SOURCE_REF_DEFAULT.to_string(),
                source_section: record.source_section.clone(),
                source_line: record.source_line,
                privacy,
                event: Event::TodoClaimed {
                    goal_id: goal_id.to_string(),
                    todo_id: todo_id.clone(),
                    agent_id: owner,
                    lease_expires_at: crate::state::now_epoch() + 45 * 60,
                    ts: anchor_ts,
                },
            });
        }
        if record.status == "done" {
            events.push(BackfillEvent {
                event_id: backfill_event_id(goal_id, &todo_id, "complete"),
                source_ref: BACKFILL_SOURCE_REF_DEFAULT.to_string(),
                source_section: record.source_section.clone(),
                source_line: record.source_line,
                privacy,
                event: Event::TodoCompleted {
                    goal_id: goal_id.to_string(),
                    todo_id: todo_id.clone(),
                    no_follow_up: record.no_followup,
                    successor_ids: vec![],
                    evidence: record.evidence.as_deref().map(|e| redact(e, privacy)),
                    ts: record
                        .completed_at
                        .as_deref()
                        .and_then(parse_anchor_epoch)
                        .unwrap_or(crate::state::now_epoch()),
                },
            });
        }
    }
    Ok(BackfillOutcome {
        goal_id: goal_id.to_string(),
        todo_count,
        event_count: events.len(),
        events,
    })
}

/// The markdown workbench text for a goal (project-local state layout under
/// the project's `.future/loop/goals/<id>/`).
pub fn active_state_markdown(cwd: &str, goal_id: &str) -> Result<String> {
    let path = std::path::Path::new(cwd)
        .join(".future")
        .join("loop")
        .join("goals")
        .join(goal_id)
        .join("ACTIVE_GOAL_STATE.md");
    std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "---\nstatus: active\n---\n\n# Active Goal State\n\n\
        ## Agent Todo\n\n\
        - [ ] [P1] Run the check\n  <!-- future-loop:todo todo_id=todo_abc123 status=open action_kind=shell updated_at=2026-08-05T12:00:00+00:00 -->\n\
        - [x] Ship the artifact\n  <!-- future-loop:todo todo_id=todo_def456 status=done no_followup=true evidence=done%20well completed_at=2026-08-05T13:00:00+00:00 updated_at=2026-08-05T13:00:00+00:00 -->\n\n\
        ## User Todo / Owner Review Reading Queue\n\n\
        - [ ] Decide the scope\n  <!-- future-loop:todo todo_id=todo_ghi789 status=open task_class=user_gate updated_at=2026-08-05T12:30:00+00:00 -->\n";

    #[test]
    fn parses_records_with_roles_and_anchors() {
        let records = parse_markdown_todos(SAMPLE);
        assert_eq!(records.len(), 3);
        assert_eq!(records[0].role, "agent");
        assert_eq!(records[0].todo_id.as_deref(), Some("todo_abc123"));
        assert_eq!(records[0].status, "open");
        assert_eq!(records[0].source_section, "Agent Todo");
        assert_eq!(records[1].role, "agent");
        assert_eq!(records[1].status, "done");
        assert!(records[1].no_followup);
        assert_eq!(records[1].evidence.as_deref(), Some("done well"));
        assert_eq!(records[2].role, "user");
        assert_eq!(records[2].task_class.as_deref(), Some("user_gate"));
    }

    #[test]
    fn backfill_events_are_idempotent_and_carry_provenance() {
        let outcome = backfill_todo_events(SAMPLE, "g1", PrivacyLevel::LocalPrivate).unwrap();
        assert_eq!(outcome.todo_count, 3);
        // 3 adds + 1 complete (no claims) — ids are deterministic.
        assert_eq!(outcome.event_count, 4);
        let add = &outcome.events[0];
        assert!(add.event_id.starts_with("backfill-add-"));
        assert_eq!(add.source_ref, "ACTIVE_GOAL_STATE.md");
        assert_eq!(add.source_section, "Agent Todo");
        assert_eq!(add.source_line, 9);
        match &add.event {
            Event::TodoAdded { todo, .. } => {
                assert_eq!(todo.id, "todo_abc123");
                assert_eq!(todo.class, TaskClass::Advancement);
                assert_eq!(todo.priority, Priority::P1);
            }
            _ => panic!("expected TodoAdded"),
        }
        // The done todo produces a TodoCompleted with URL-decoded evidence.
        let complete = outcome
            .events
            .iter()
            .find(|e| e.event_id.starts_with("backfill-complete-"))
            .unwrap();
        match &complete.event {
            Event::TodoCompleted { evidence, .. } => {
                assert_eq!(evidence.as_deref(), Some("done well"));
            }
            _ => panic!("expected TodoCompleted"),
        }
        // Re-running yields identical ids (idempotent re-append).
        let again = backfill_todo_events(SAMPLE, "g1", PrivacyLevel::LocalPrivate).unwrap();
        for (a, b) in outcome.events.iter().zip(again.events.iter()) {
            assert_eq!(a.event_id, b.event_id);
        }
    }

    #[test]
    fn public_safe_backfill_redacts_private_text() {
        let md = "## Agent Todo\n\n- [ ] touch /Users/geilige/secret\n  <!-- future-loop:todo todo_id=todo_x status=open -->\n";
        let public = backfill_todo_events(md, "g1", PrivacyLevel::PublicSafe).unwrap();
        match &public.events[0].event {
            Event::TodoAdded { todo, .. } => {
                assert!(todo.text.contains("[redacted-private-state]"));
            }
            _ => panic!("expected TodoAdded"),
        }
        let local = backfill_todo_events(md, "g1", PrivacyLevel::LocalPrivate).unwrap();
        match &local.events[0].event {
            Event::TodoAdded { todo, .. } => {
                assert!(todo.text.contains("/Users/geilige"));
            }
            _ => panic!("expected TodoAdded"),
        }
    }

    #[test]
    fn backfill_rejects_empty_workbench() {
        assert!(backfill_todo_events("no todos here", "g1", PrivacyLevel::LocalPrivate).is_err());
    }
}
