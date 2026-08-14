//! Explore capability (LoopX: explore — bounded exploration with replay/trace
//! hygiene). Wave 2 deepening: the 44-line keyword shell becomes a structured
//! exploration pipeline, porting the hypothesis-tracking + explore-graph
//! subdomains of the reference `capabilities/explore/result_log.py` as a
//! deterministic rule version:
//!
//! - **hypothesis modeling**: canonical, public-safe hypothesis node events
//!   (title compaction, stable derived ids, boundary declaration);
//! - **verification tracking**: a hypothesis node's verification state is
//!   derived from its node status, its attached findings, and the incident
//!   `supports` / `refutes` edges — never claimed by prose;
//! - **explore graph**: append-only node / edge / finding events folded into
//!   a bounded graph view (nodes / edges / statuses / stuck / frontier /
//!   mermaid), with status+tag filters and ancestor context;
//! - **propose**: routes payloads and claims into a FINITE set of typed
//!   proposals (successor / monitor / gate / no-follow-up). A capability
//!   never writes state itself: it proposes; the kernel decides.
//!
//! Out of scope (deliberately): the harness/replay/counterfactual runtimes,
//! external sink delivery, and todo-branch planning from the reference —
//! those stay LoopX-side; this module ships only the result-log subdomain.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use serde_json::Value;

use super::monitor_todo;
use super::successor_todo;
use super::Capability;
use super::TypedProposal;

// ── schema vocabulary (reference result_log.py constants) ─────────────────

pub const EXPLORE_RESULT_EVENT_SCHEMA_VERSION: &str = "loopx_explore_result_event_v0";
pub const EXPLORE_RESULT_PROJECTION_VERSION: &str = "loopx_explore_result_projection_v0";
pub const EXPLORE_HYPOTHESIS_VERIFICATION_VERSION: &str =
    "loopx_explore_hypothesis_verification_v0";

pub const EVENT_KIND_NODE: &str = "node";
pub const EVENT_KIND_EDGE: &str = "edge";
pub const EVENT_KIND_FINDING: &str = "finding";
pub const EXPLORE_EVENT_KINDS: [&str; 3] = [EVENT_KIND_NODE, EVENT_KIND_EDGE, EVENT_KIND_FINDING];

pub const NODE_KIND_QUESTION: &str = "question";
pub const NODE_KIND_AREA: &str = "area";
pub const NODE_KIND_HYPOTHESIS: &str = "hypothesis";
pub const NODE_KIND_EXPERIMENT: &str = "experiment";
pub const NODE_KIND_ARTIFACT: &str = "artifact";
pub const NODE_KINDS: [&str; 5] = [
    NODE_KIND_QUESTION,
    NODE_KIND_AREA,
    NODE_KIND_HYPOTHESIS,
    NODE_KIND_EXPERIMENT,
    NODE_KIND_ARTIFACT,
];

pub const NODE_STATUS_OPEN: &str = "open";
pub const NODE_STATUS_EXPLORING: &str = "exploring";
pub const NODE_STATUS_BLOCKED: &str = "blocked";
pub const NODE_STATUS_RESOLVED: &str = "resolved";
pub const NODE_STATUS_DEAD_END: &str = "dead_end";
pub const NODE_STATUSES: [&str; 5] = [
    NODE_STATUS_OPEN,
    NODE_STATUS_EXPLORING,
    NODE_STATUS_BLOCKED,
    NODE_STATUS_RESOLVED,
    NODE_STATUS_DEAD_END,
];

pub const EDGE_TYPE_SUBTOPIC_OF: &str = "subtopic_of";
pub const EDGE_TYPES: [&str; 6] = [
    EDGE_TYPE_SUBTOPIC_OF,
    "depends_on",
    "answers",
    "supports",
    "refutes",
    "leads_to",
];

pub const FINDING_STATUS_TENTATIVE: &str = "tentative";
pub const FINDING_STATUS_CONFIRMED: &str = "confirmed";
pub const FINDING_STATUS_REFUTED: &str = "refuted";
pub const FINDING_STATUSES: [&str; 3] = [
    FINDING_STATUS_TENTATIVE,
    FINDING_STATUS_CONFIRMED,
    FINDING_STATUS_REFUTED,
];

/// Hypothesis verification states (derived, never authored).
pub const VERIFICATION_UNVERIFIED: &str = "unverified";
pub const VERIFICATION_TESTING: &str = "testing";
pub const VERIFICATION_SUPPORTED: &str = "supported";
pub const VERIFICATION_REFUTED: &str = "refuted";
pub const VERIFICATION_BLOCKED: &str = "blocked";

pub const TITLE_LIMIT: usize = 200;
pub const SUMMARY_LIMIT: usize = 1200;
pub const REF_LIMIT: usize = 240;
pub const MAX_EVIDENCE_REFS: usize = 16;
pub const MAX_TAGS: usize = 8;
pub const DEFAULT_FINDING_LIMIT: usize = 200;
pub const DEFAULT_MERMAID_NODE_LIMIT: usize = 60;
pub const DEFAULT_TREE_DEPTH_LIMIT: usize = 6;

/// Cadence for re-observing a graph with in-flight exploration (minute
/// string + seconds; consumed by the monitor todo).
pub const REOBSERVE_CADENCE_MINUTES: u64 = 15;
pub const REOBSERVE_CADENCE: &str = "15m";

const FORBIDDEN_TEXT_MARKERS: [&str; 12] = [
    "/Users/",
    "/root/",
    "/home/",
    "/private/",
    "\\Users\\",
    "\\root\\",
    "\\home\\",
    "\\private\\",
    ".local/private",
    "authorization:",
    "api_key",
    "password",
];

/// The public-safety boundary every event must declare verbatim (reference
/// PUBLIC_BOUNDARY). Forged flags are rejected at validation.
fn public_boundary() -> Value {
    serde_json::json!({
        "raw_task_text_recorded": false,
        "raw_logs_recorded": false,
        "raw_trajectory_recorded": false,
        "raw_session_transcript_recorded": false,
        "credential_values_recorded": false,
        "absolute_paths_recorded": false,
    })
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// `C:\Users` / `C:/Users`-style drive paths (reference `_WINDOWS_ABS_PATH`).
fn contains_windows_abs_path(text: &str) -> bool {
    let bytes = text.as_bytes();
    for (index, _) in text.char_indices() {
        let drive = text[index..]
            .chars()
            .next()
            .expect("char_indices yields a char boundary");
        if !drive.is_ascii_alphabetic() {
            continue;
        }
        if index > 0 {
            let previous = bytes[index - 1];
            if previous.is_ascii_alphanumeric() || matches!(previous, b'_' | b'.' | b'-') {
                continue;
            }
        }
        let Some(colon) = text[index..].chars().nth(1) else {
            continue;
        };
        if colon != ':' {
            continue;
        }
        let Some(separator) = text[index..].chars().nth(2) else {
            continue;
        };
        if matches!(separator, '/' | '\\') {
            let after = text[index..].chars().nth(3);
            if !matches!(after, Some('/') | Some('\\')) {
                return true;
            }
        }
    }
    false
}

/// Collapse whitespace, reject private/credential-like material, truncate
/// (reference `_compact_text`).
fn compact_text(value: &str, limit: usize, field: &str) -> Result<String, String> {
    let text = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let lowered = text.to_lowercase();
    if lowered.contains("file://") || contains_windows_abs_path(&text) {
        return Err(format!(
            "{field} contains private or credential-like material"
        ));
    }
    for marker in FORBIDDEN_TEXT_MARKERS {
        if lowered.contains(&marker.to_lowercase()) {
            return Err(format!(
                "{field} contains private or credential-like material"
            ));
        }
    }
    if text.len() <= limit {
        return Ok(text);
    }
    Ok(format!("{}...", text[..limit.saturating_sub(3)].trim_end()))
}

/// A public relative ref or opaque id — never a local path (reference
/// `_safe_public_ref`).
fn safe_public_ref(value: &str, field: &str) -> Result<String, String> {
    let text = compact_text(value, REF_LIMIT, field)?;
    if text.is_empty() {
        return Err(format!("{field} is empty"));
    }
    if text.starts_with(['~', '/', '\\'])
        || text.to_lowercase().starts_with("file://")
        || contains_windows_abs_path(&text)
        || text.split(['/', '\\']).any(|part| part == "..")
    {
        return Err(format!(
            "{field} must be a public relative ref or opaque id, not a local path"
        ));
    }
    Ok(text)
}

fn safe_public_refs(
    values: &[String],
    field: &str,
    max_items: usize,
) -> Result<Vec<String>, String> {
    values
        .iter()
        .take(max_items)
        .enumerate()
        .map(|(index, value)| safe_public_ref(value, &format!("{field}[{index}]")))
        .collect()
}

/// `^[A-Za-z][A-Za-z0-9_.:-]{0,95}$` (reference `_safe_result_id`).
fn safe_result_id(value: &str, field: &str) -> Result<String, String> {
    let text = value.trim();
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return Err(format!(
            "{field} must match ^[A-Za-z][A-Za-z0-9_.:-]{{0,95}}$"
        ));
    };
    let valid = first.is_ascii_alphabetic()
        && text.len() <= 96
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'));
    if !valid {
        return Err(format!(
            "{field} must match ^[A-Za-z][A-Za-z0-9_.:-]{{0,95}}$"
        ));
    }
    Ok(text.to_string())
}

/// A single path segment (reference `_safe_goal_id`).
fn safe_goal_id(value: &str) -> Result<String, String> {
    let text = value.trim();
    if text.is_empty() || text.contains(['/', '\\']) || matches!(text, "." | "..") {
        return Err("goal_id must be a single path segment".to_string());
    }
    Ok(text.to_string())
}

/// Confidence in [0, 1], rounded to 3 decimals (reference `_safe_confidence`).
fn safe_confidence(value: Option<f64>) -> Result<Option<f64>, String> {
    let Some(number) = value else { return Ok(None) };
    if !number.is_finite() || !(0.0..=1.0).contains(&number) {
        return Err("confidence must be between 0 and 1".to_string());
    }
    Ok(Some((number * 1000.0).round() / 1000.0))
}

/// Stable sha256-based id over the payload minus `event_id` (reference
/// `_event_id`).
fn event_id(payload: &Value) -> String {
    use sha2::Digest;
    let mut stable = payload.clone();
    if let Some(map) = stable.as_object_mut() {
        map.remove("event_id");
    }
    let encoded = serde_json::to_string(&stable).unwrap_or_default();
    let digest = sha2::Sha256::digest(encoded.as_bytes());
    digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()[..16]
        .to_string()
}

fn derived_result_id(prefix: &str, parts: &[&str]) -> String {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(parts.join("|").as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("{prefix}_{}", &hex[..12])
}

fn choice(value: &str, choices: &[&str], default: &str, field: &str) -> Result<String, String> {
    let text = if value.trim().is_empty() {
        default
    } else {
        value.trim()
    }
    .to_lowercase();
    if !choices.contains(&text.as_str()) {
        return Err(format!(
            "unsupported {field} {value:?}; choose {}",
            choices.join(", ")
        ));
    }
    Ok(text)
}

// ── event builders (reference build_explore_{node,edge,finding}_event) ─────

/// Build one canonical hypothesis/exploration node event (reference
/// `build_explore_node_event`).
#[allow(clippy::too_many_arguments)]
pub fn build_explore_node_event(
    goal_id: &str,
    title: &str,
    node_id: Option<&str>,
    node_kind: Option<&str>,
    status: Option<&str>,
    summary: Option<&str>,
    blocked_reason: Option<&str>,
    parent_id: Option<&str>,
    agent_id: Option<&str>,
    run_id: Option<&str>,
    evidence_refs: &[String],
    tags: &[String],
    supersedes: Option<&str>,
    recorded_at: Option<&str>,
) -> Result<Value, String> {
    let safe_goal_id = safe_goal_id(goal_id)?;
    let safe_title = compact_text(title, TITLE_LIMIT, "title")?;
    if safe_title.is_empty() {
        return Err("title is required".to_string());
    }
    let resolved_status = choice(
        status.unwrap_or_default(),
        &NODE_STATUSES,
        NODE_STATUS_OPEN,
        "status",
    )?;
    let safe_blocked_reason = compact_text(
        blocked_reason.unwrap_or_default(),
        SUMMARY_LIMIT,
        "blocked_reason",
    )?;
    if resolved_status == NODE_STATUS_BLOCKED && safe_blocked_reason.is_empty() {
        return Err("blocked nodes must state a blocked_reason".to_string());
    }
    let mut event = serde_json::json!({
        "schema_version": EXPLORE_RESULT_EVENT_SCHEMA_VERSION,
        "goal_id": safe_goal_id.clone(),
        "event_kind": EVENT_KIND_NODE,
        "recorded_at": recorded_at.filter(|s| !s.trim().is_empty()).map(str::to_string).unwrap_or_else(now_iso),
        "title": safe_title.clone(),
        "boundary": public_boundary(),
    });
    let map = event.as_object_mut().expect("node event is an object");
    if let Some(summary) = summary.filter(|s| !s.trim().is_empty()) {
        map.insert(
            "summary".to_string(),
            Value::String(compact_text(summary, SUMMARY_LIMIT, "summary")?),
        );
    }
    if let Some(agent_id) = agent_id.filter(|s| !s.trim().is_empty()) {
        map.insert(
            "agent_id".to_string(),
            Value::String(compact_text(agent_id, 80, "agent_id")?),
        );
    }
    if let Some(run_id) = run_id.filter(|s| !s.trim().is_empty()) {
        map.insert(
            "run_id".to_string(),
            Value::String(safe_public_ref(run_id, "run_id")?),
        );
    }
    let refs = safe_public_refs(evidence_refs, "evidence_refs", MAX_EVIDENCE_REFS)?;
    if !refs.is_empty() {
        map.insert(
            "evidence_refs".to_string(),
            Value::Array(refs.into_iter().map(Value::String).collect()),
        );
    }
    let mut safe_tags: Vec<String> = tags
        .iter()
        .take(MAX_TAGS)
        .enumerate()
        .map(|(index, tag)| compact_text(tag, 48, &format!("tags[{index}]")))
        .collect::<Result<_, _>>()?;
    safe_tags.retain(|tag| !tag.is_empty());
    if !safe_tags.is_empty() {
        map.insert(
            "tags".to_string(),
            Value::Array(safe_tags.into_iter().map(Value::String).collect()),
        );
    }
    if let Some(supersedes) = supersedes.filter(|s| !s.trim().is_empty()) {
        map.insert(
            "supersedes".to_string(),
            Value::String(safe_result_id(supersedes, "supersedes")?),
        );
    }
    map.insert(
        "result_id".to_string(),
        Value::String(match node_id.filter(|s| !s.trim().is_empty()) {
            Some(id) => safe_result_id(id, "node_id")?,
            None => derived_result_id("node", &[&safe_goal_id, &safe_title.to_lowercase()]),
        }),
    );
    map.insert(
        "node_kind".to_string(),
        Value::String(choice(
            node_kind.unwrap_or_default(),
            &NODE_KINDS,
            NODE_KIND_AREA,
            "node_kind",
        )?),
    );
    map.insert("status".to_string(), Value::String(resolved_status));
    if !safe_blocked_reason.is_empty() {
        map.insert(
            "blocked_reason".to_string(),
            Value::String(safe_blocked_reason),
        );
    }
    if let Some(parent_id) = parent_id.filter(|s| !s.trim().is_empty()) {
        map.insert(
            "parent_id".to_string(),
            Value::String(safe_result_id(parent_id, "parent_id")?),
        );
    }
    let digest = event_id(&Value::Object(map.clone()));
    map.insert("event_id".to_string(), Value::String(digest));
    Ok(event)
}

/// Build one typed edge event (reference `build_explore_edge_event`).
#[allow(clippy::too_many_arguments)]
pub fn build_explore_edge_event(
    goal_id: &str,
    from_node: &str,
    to_node: &str,
    edge_type: &str,
    summary: Option<&str>,
    confidence: Option<f64>,
    agent_id: Option<&str>,
    run_id: Option<&str>,
    recorded_at: Option<&str>,
) -> Result<Value, String> {
    let safe_goal_id = safe_goal_id(goal_id)?;
    let safe_from = safe_result_id(from_node, "from_node")?;
    let safe_to = safe_result_id(to_node, "to_node")?;
    if safe_from == safe_to {
        return Err("edge must connect two different nodes".to_string());
    }
    let resolved_type = choice(edge_type, &EDGE_TYPES, "", "edge_type")?;
    let mut event = serde_json::json!({
        "schema_version": EXPLORE_RESULT_EVENT_SCHEMA_VERSION,
        "goal_id": safe_goal_id,
        "event_kind": EVENT_KIND_EDGE,
        "recorded_at": recorded_at.filter(|s| !s.trim().is_empty()).map(str::to_string).unwrap_or_else(now_iso),
        "title": format!("{safe_from} -{resolved_type}-> {safe_to}"),
        "boundary": public_boundary(),
    });
    let map = event.as_object_mut().expect("edge event is an object");
    if let Some(summary) = summary.filter(|s| !s.trim().is_empty()) {
        map.insert(
            "summary".to_string(),
            Value::String(compact_text(summary, SUMMARY_LIMIT, "summary")?),
        );
    }
    if let Some(agent_id) = agent_id.filter(|s| !s.trim().is_empty()) {
        map.insert(
            "agent_id".to_string(),
            Value::String(compact_text(agent_id, 80, "agent_id")?),
        );
    }
    if let Some(run_id) = run_id.filter(|s| !s.trim().is_empty()) {
        map.insert(
            "run_id".to_string(),
            Value::String(safe_public_ref(run_id, "run_id")?),
        );
    }
    map.insert(
        "result_id".to_string(),
        Value::String(derived_result_id(
            "edge",
            &[&safe_goal_id, &safe_from, &resolved_type, &safe_to],
        )),
    );
    map.insert("from_node".to_string(), Value::String(safe_from));
    map.insert("to_node".to_string(), Value::String(safe_to));
    map.insert("edge_type".to_string(), Value::String(resolved_type));
    if let Some(confidence) = safe_confidence(confidence)? {
        map.insert("confidence".to_string(), Value::from(confidence));
    }
    let digest = event_id(&Value::Object(map.clone()));
    map.insert("event_id".to_string(), Value::String(digest));
    Ok(event)
}

/// Build one finding event attached to an optional node (reference
/// `build_explore_finding_event`).
#[allow(clippy::too_many_arguments)]
pub fn build_explore_finding_event(
    goal_id: &str,
    title: &str,
    finding_id: Option<&str>,
    node_id: Option<&str>,
    status: Option<&str>,
    summary: Option<&str>,
    confidence: Option<f64>,
    agent_id: Option<&str>,
    run_id: Option<&str>,
    evidence_refs: &[String],
    tags: &[String],
    supersedes: Option<&str>,
    recorded_at: Option<&str>,
) -> Result<Value, String> {
    let safe_goal_id = safe_goal_id(goal_id)?;
    let safe_title = compact_text(title, TITLE_LIMIT, "title")?;
    if safe_title.is_empty() {
        return Err("title is required".to_string());
    }
    let mut event = serde_json::json!({
        "schema_version": EXPLORE_RESULT_EVENT_SCHEMA_VERSION,
        "goal_id": safe_goal_id,
        "event_kind": EVENT_KIND_FINDING,
        "recorded_at": recorded_at.filter(|s| !s.trim().is_empty()).map(str::to_string).unwrap_or_else(now_iso),
        "title": safe_title.clone(),
        "boundary": public_boundary(),
    });
    let map = event.as_object_mut().expect("finding event is an object");
    if let Some(summary) = summary.filter(|s| !s.trim().is_empty()) {
        map.insert(
            "summary".to_string(),
            Value::String(compact_text(summary, SUMMARY_LIMIT, "summary")?),
        );
    }
    if let Some(agent_id) = agent_id.filter(|s| !s.trim().is_empty()) {
        map.insert(
            "agent_id".to_string(),
            Value::String(compact_text(agent_id, 80, "agent_id")?),
        );
    }
    if let Some(run_id) = run_id.filter(|s| !s.trim().is_empty()) {
        map.insert(
            "run_id".to_string(),
            Value::String(safe_public_ref(run_id, "run_id")?),
        );
    }
    let refs = safe_public_refs(evidence_refs, "evidence_refs", MAX_EVIDENCE_REFS)?;
    if !refs.is_empty() {
        map.insert(
            "evidence_refs".to_string(),
            Value::Array(refs.into_iter().map(Value::String).collect()),
        );
    }
    let mut safe_tags: Vec<String> = tags
        .iter()
        .take(MAX_TAGS)
        .enumerate()
        .map(|(index, tag)| compact_text(tag, 48, &format!("tags[{index}]")))
        .collect::<Result<_, _>>()?;
    safe_tags.retain(|tag| !tag.is_empty());
    if !safe_tags.is_empty() {
        map.insert(
            "tags".to_string(),
            Value::Array(safe_tags.into_iter().map(Value::String).collect()),
        );
    }
    if let Some(supersedes) = supersedes.filter(|s| !s.trim().is_empty()) {
        map.insert(
            "supersedes".to_string(),
            Value::String(safe_result_id(supersedes, "supersedes")?),
        );
    }
    map.insert(
        "result_id".to_string(),
        Value::String(match finding_id.filter(|s| !s.trim().is_empty()) {
            Some(id) => safe_result_id(id, "finding_id")?,
            None => derived_result_id("finding", &[&safe_goal_id, &safe_title.to_lowercase()]),
        }),
    );
    if let Some(node_id) = node_id.filter(|s| !s.trim().is_empty()) {
        map.insert(
            "node_id".to_string(),
            Value::String(safe_result_id(node_id, "node_id")?),
        );
    }
    map.insert(
        "status".to_string(),
        Value::String(choice(
            status.unwrap_or_default(),
            &FINDING_STATUSES,
            FINDING_STATUS_TENTATIVE,
            "status",
        )?),
    );
    if let Some(confidence) = safe_confidence(confidence)? {
        map.insert("confidence".to_string(), Value::from(confidence));
    }
    let digest = event_id(&Value::Object(map.clone()));
    map.insert("event_id".to_string(), Value::String(digest));
    Ok(event)
}

/// Return one canonical public-safe event or fail closed (reference
/// `validate_explore_result_event`): the builders are the schema authority,
/// so rebuilding catches unknown fields, invalid ids, unsafe text, forged
/// boundary flags, and stale event ids.
pub fn validate_explore_result_event(
    event: &Value,
    expected_goal_id: Option<&str>,
) -> Result<Value, String> {
    let payload = event
        .as_object()
        .ok_or("explore result event must be an object")?;
    if payload.get("schema_version").and_then(Value::as_str)
        != Some(EXPLORE_RESULT_EVENT_SCHEMA_VERSION)
    {
        return Err(format!(
            "explore result event must use schema {EXPLORE_RESULT_EVENT_SCHEMA_VERSION}"
        ));
    }
    let event_kind = payload
        .get("event_kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !EXPLORE_EVENT_KINDS.contains(&event_kind) {
        return Err(format!(
            "unsupported explore result event kind {event_kind:?}"
        ));
    }
    let goal_id = safe_goal_id(
        payload
            .get("goal_id")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    )?;
    if let Some(expected) = expected_goal_id {
        if goal_id != safe_goal_id(expected)? {
            return Err("explore result event belongs to a different goal".to_string());
        }
    }
    if payload.get("boundary") != Some(&public_boundary()) {
        return Err("explore result event must declare the public-safe boundary".to_string());
    }
    let text = |key: &str| payload.get(key).and_then(Value::as_str);
    let text_list = |key: &str| -> Vec<String> {
        payload
            .get(key)
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    let rebuilt = match event_kind {
        EVENT_KIND_NODE => build_explore_node_event(
            &goal_id,
            text("title").unwrap_or_default(),
            text("result_id"),
            text("node_kind"),
            text("status"),
            text("summary"),
            text("blocked_reason"),
            text("parent_id"),
            text("agent_id"),
            text("run_id"),
            &text_list("evidence_refs"),
            &text_list("tags"),
            text("supersedes"),
            text("recorded_at"),
        )?,
        EVENT_KIND_EDGE => build_explore_edge_event(
            &goal_id,
            text("from_node").unwrap_or_default(),
            text("to_node").unwrap_or_default(),
            text("edge_type").unwrap_or_default(),
            text("summary"),
            payload.get("confidence").and_then(Value::as_f64),
            text("agent_id"),
            text("run_id"),
            text("recorded_at"),
        )?,
        _ => build_explore_finding_event(
            &goal_id,
            text("title").unwrap_or_default(),
            text("result_id"),
            text("node_id"),
            text("status"),
            text("summary"),
            payload.get("confidence").and_then(Value::as_f64),
            text("agent_id"),
            text("run_id"),
            &text_list("evidence_refs"),
            &text_list("tags"),
            text("supersedes"),
            text("recorded_at"),
        )?,
    };
    if event != &rebuilt {
        return Err("explore result event is not canonical or contains unknown fields".to_string());
    }
    Ok(rebuilt)
}

// ── graph view + projection (reference result_log.py folding) ─────────────

/// Fold events by result_id: last event wins; first/last timestamps and the
/// update count are derived (reference `_fold_by_result_id`).
fn fold_by_result_id(events: &[Value], event_kind: &str) -> Vec<Value> {
    let mut folded: BTreeMap<String, Value> = BTreeMap::new();
    for event in events {
        if event.get("event_kind").and_then(Value::as_str) != Some(event_kind) {
            continue;
        }
        let Some(result_id) = event.get("result_id").and_then(Value::as_str) else {
            continue;
        };
        let recorded_at = event
            .get("recorded_at")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let prior = folded.get(result_id);
        let mut view = event.clone();
        let map = view.as_object_mut().expect("folded event is an object");
        map.insert(
            "first_recorded_at".to_string(),
            Value::String(
                prior
                    .and_then(|p| p.get("first_recorded_at"))
                    .and_then(Value::as_str)
                    .unwrap_or(recorded_at)
                    .to_string(),
            ),
        );
        map.insert(
            "last_updated_at".to_string(),
            Value::String(recorded_at.to_string()),
        );
        let update_count = prior
            .and_then(|p| p.get("update_count"))
            .and_then(Value::as_u64)
            .unwrap_or(0)
            + 1;
        map.insert("update_count".to_string(), Value::from(update_count));
        folded.insert(result_id.to_string(), view);
    }
    folded.into_values().collect()
}

fn node_view(event: &Value, finding_count: usize) -> Value {
    let text = |key: &str| {
        event
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let list = |key: &str| -> Vec<String> {
        event
            .get(key)
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    serde_json::json!({
        "node_id": text("result_id"),
        "title": text("title"),
        "node_kind": if text("node_kind").is_empty() { NODE_KIND_AREA.to_string() } else { text("node_kind") },
        "status": if text("status").is_empty() { NODE_STATUS_OPEN.to_string() } else { text("status") },
        "summary": text("summary"),
        "blocked_reason": text("blocked_reason"),
        "parent_id": text("parent_id"),
        "agent_id": text("agent_id"),
        "evidence_refs": list("evidence_refs"),
        "tags": list("tags"),
        "supersedes": text("supersedes"),
        "finding_count": finding_count,
        "first_recorded_at": text("first_recorded_at"),
        "last_updated_at": text("last_updated_at"),
        "update_count": event.get("update_count").and_then(Value::as_u64).unwrap_or(1),
    })
}

fn edge_view(event: &Value) -> Value {
    let text = |key: &str| {
        event
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    serde_json::json!({
        "edge_id": text("result_id"),
        "from_node": text("from_node"),
        "to_node": text("to_node"),
        "edge_type": text("edge_type"),
        "summary": text("summary"),
        "confidence": event.get("confidence").cloned(),
        "last_updated_at": text("last_updated_at"),
    })
}

fn finding_view(event: &Value) -> Value {
    let text = |key: &str| {
        event
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let list = |key: &str| -> Vec<String> {
        event
            .get(key)
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    serde_json::json!({
        "finding_id": text("result_id"),
        "finding": text("title"),
        "summary": text("summary"),
        "status": if text("status").is_empty() { FINDING_STATUS_TENTATIVE.to_string() } else { text("status") },
        "confidence": event.get("confidence").cloned(),
        "node_id": text("node_id"),
        "agent_id": text("agent_id"),
        "evidence_refs": list("evidence_refs"),
        "tags": list("tags"),
        "supersedes": text("supersedes"),
        "first_recorded_at": text("first_recorded_at"),
        "last_updated_at": text("last_updated_at"),
        "update_count": event.get("update_count").and_then(Value::as_u64).unwrap_or(1),
    })
}

/// Parent → child `supports` display edges derived from `parent_id`
/// (reference `_materialized_parent_edges`). They do not alter the
/// `subtopic_of` tree parser.
fn materialized_parent_edges(nodes: &[Value], edges: &[Value]) -> Vec<Value> {
    use sha2::Digest;
    let known: BTreeSet<String> = nodes
        .iter()
        .filter_map(|node| {
            node.get("node_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    let existing_pairs: BTreeSet<(String, String)> = edges
        .iter()
        .filter_map(|edge| {
            let from = edge.get("from_node").and_then(Value::as_str)?;
            let to = edge.get("to_node").and_then(Value::as_str)?;
            Some(if from <= to {
                (from.to_string(), to.to_string())
            } else {
                (to.to_string(), from.to_string())
            })
        })
        .collect();
    let mut derived: Vec<Value> = Vec::new();
    for node in nodes {
        let child = node
            .get("node_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let parent = node
            .get("parent_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if child.is_empty()
            || parent.is_empty()
            || child == parent
            || !known.contains(parent)
            || existing_pairs.contains(&(parent.to_string(), child.to_string()))
            || existing_pairs.contains(&(child.to_string(), parent.to_string()))
        {
            continue;
        }
        let digest = sha2::Sha256::digest(format!("{parent}->{child}").as_bytes());
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        derived.push(serde_json::json!({
            "edge_id": format!("parent_{}", &hex[..12]),
            "from_node": parent,
            "to_node": child,
            "edge_type": "supports",
            "summary": "Parent topic contains this exploration node.",
            "confidence": 1.0,
            "last_updated_at": node.get("last_updated_at").and_then(Value::as_str).unwrap_or_default(),
            "materialized_from": "node_parent_id",
        }));
    }
    derived
}

/// node → parent map from `subtopic_of` edges + `parent_id` links (reference
/// `_parent_map`).
fn parent_map(nodes: &[Value], edges: &[Value]) -> BTreeMap<String, String> {
    let known: BTreeSet<String> = nodes
        .iter()
        .filter_map(|node| {
            node.get("node_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    let mut parents: BTreeMap<String, String> = BTreeMap::new();
    for edge in edges {
        if edge.get("edge_type").and_then(Value::as_str) != Some(EDGE_TYPE_SUBTOPIC_OF) {
            continue;
        }
        let child = edge
            .get("from_node")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let parent = edge
            .get("to_node")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if known.contains(child) && known.contains(parent) {
            parents.insert(child.to_string(), parent.to_string());
        }
    }
    for node in nodes {
        let parent = node
            .get("parent_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let node_id = node
            .get("node_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !parent.is_empty() && known.contains(parent) {
            parents.insert(node_id.to_string(), parent.to_string());
        }
    }
    parents
}

/// Compact status → parent tree (reference `_build_tree`).
fn build_tree(
    nodes: &[Value],
    parents: &BTreeMap<String, String>,
    depth_limit: usize,
) -> Vec<Value> {
    let node_by_id: BTreeMap<String, &Value> = nodes
        .iter()
        .filter_map(|node| {
            let id = node.get("node_id").and_then(Value::as_str)?;
            Some((id.to_string(), node))
        })
        .collect();
    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut roots: Vec<String> = Vec::new();
    for node_id in node_by_id.keys() {
        match parents.get(node_id) {
            Some(parent) if parent != node_id && node_by_id.contains_key(parent) => {
                children
                    .entry(parent.clone())
                    .or_default()
                    .push(node_id.clone());
            }
            _ => roots.push(node_id.clone()),
        }
    }
    fn branch(
        node_id: &str,
        node_by_id: &BTreeMap<String, &Value>,
        children: &BTreeMap<String, Vec<String>>,
        depth: usize,
        depth_limit: usize,
        seen: &BTreeSet<String>,
    ) -> Value {
        let node = node_by_id[node_id];
        let mut view = serde_json::json!({
            "node_id": node_id,
            "title": node.get("title").and_then(Value::as_str).unwrap_or_default(),
            "status": node.get("status").and_then(Value::as_str).unwrap_or(NODE_STATUS_OPEN),
            "children": [],
        });
        if depth < depth_limit {
            let mut seen = seen.clone();
            seen.insert(node_id.to_string());
            let kids = children
                .get(node_id)
                .map(|kids| {
                    kids.iter()
                        .filter(|kid| !seen.contains(*kid))
                        .map(|kid| branch(kid, node_by_id, children, depth + 1, depth_limit, &seen))
                        .collect()
                })
                .unwrap_or_default();
            view["children"] = Value::Array(kids);
        }
        view
    }
    roots
        .iter()
        .map(|root| {
            let mut root_seen = BTreeSet::new();
            root_seen.insert(root.clone());
            branch(root, &node_by_id, &children, 1, depth_limit, &root_seen)
        })
        .collect()
}

fn mermaid_label(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| if "\"[]{}<>`|".contains(c) { '\'' } else { c })
        .collect();
    let cleaned = cleaned.chars().take(60).collect::<String>();
    if cleaned.trim().is_empty() {
        "untitled".to_string()
    } else {
        cleaned.trim().to_string()
    }
}

fn mermaid_id(node_id: &str) -> String {
    node_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Render the exploration topology as Mermaid flowchart source (reference
/// `build_explore_mermaid`).
pub fn build_explore_mermaid(nodes: &[Value], edges: &[Value], node_limit: usize) -> String {
    let mut lines = vec!["flowchart TD".to_string()];
    let shown = nodes.iter().take(node_limit);
    let shown_ids: BTreeSet<String> = shown
        .clone()
        .filter_map(|node| {
            node.get("node_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    let status_class = |status: &str| match status {
        NODE_STATUS_EXPLORING => "exploring",
        NODE_STATUS_BLOCKED => "blocked",
        NODE_STATUS_RESOLVED => "resolved",
        NODE_STATUS_DEAD_END => "deadend",
        _ => "open",
    };
    let status_marker = |status: &str| match status {
        NODE_STATUS_BLOCKED => " (BLOCKED)",
        NODE_STATUS_RESOLVED => " (done)",
        NODE_STATUS_DEAD_END => " (dead end)",
        _ => "",
    };
    for node in shown {
        let node_id = mermaid_id(
            node.get("node_id")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        );
        let status = node
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or(NODE_STATUS_OPEN);
        let title = node
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let label = mermaid_label(&format!("{title}{}", status_marker(status)));
        lines.push(format!(
            "    {node_id}[\"{label}\"]:::{}",
            status_class(status)
        ));
    }
    for edge in edges {
        let from_node = edge
            .get("from_node")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let to_node = edge
            .get("to_node")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !shown_ids.contains(from_node) || !shown_ids.contains(to_node) {
            continue;
        }
        let label = mermaid_label(
            edge.get("edge_type")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        );
        lines.push(format!(
            "    {} -->|{label}| {}",
            mermaid_id(from_node),
            mermaid_id(to_node)
        ));
    }
    if nodes.len() > node_limit {
        lines.push(format!(
            "    %% {} more nodes omitted",
            nodes.len() - node_limit
        ));
    }
    lines.extend([
        "    classDef open fill:#f5f5f5,stroke:#9e9e9e".to_string(),
        "    classDef exploring fill:#e3f2fd,stroke:#1e88e5".to_string(),
        "    classDef blocked fill:#ffebee,stroke:#e53935,stroke-width:2px".to_string(),
        "    classDef resolved fill:#e8f5e9,stroke:#43a047".to_string(),
        "    classDef deadend fill:#eeeeee,stroke:#9e9e9e,stroke-dasharray: 4 4".to_string(),
    ]);
    lines.join("\n")
}

/// Build a focused graph view without mutating the full projection
/// (reference `build_explore_graph_view`). Status and tag filters combine
/// with AND semantics; tag matching is exact and OR within the requested
/// set; ancestors are included by default.
pub fn build_explore_graph_view(
    nodes: &[Value],
    edges: &[Value],
    statuses: &[&str],
    tags: &[&str],
    include_ancestors: bool,
    node_limit: usize,
) -> Result<Value, String> {
    let mut requested_statuses: BTreeSet<String> = BTreeSet::new();
    for status in statuses {
        requested_statuses.insert(choice(status, &NODE_STATUSES, "", "status")?);
    }
    let requested_tags: BTreeSet<String> = tags
        .iter()
        .map(|tag| tag.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();

    let node_list: Vec<Value> = nodes.to_vec();
    let edge_list: Vec<Value> = edges.to_vec();
    let node_by_id: BTreeMap<String, &Value> = node_list
        .iter()
        .filter_map(|node| {
            let id = node.get("node_id").and_then(Value::as_str)?;
            Some((id.to_string(), node))
        })
        .collect();

    let matches = |node: &Value| -> bool {
        if !requested_statuses.is_empty() {
            let status = node
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !requested_statuses.contains(status) {
                return false;
            }
        }
        if !requested_tags.is_empty() {
            let node_tags: BTreeSet<String> = node
                .get("tags")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            if node_tags.is_disjoint(&requested_tags) {
                return false;
            }
        }
        true
    };

    let filtering = !requested_statuses.is_empty() || !requested_tags.is_empty();
    let matched_ids: BTreeSet<String> = if filtering {
        node_by_id
            .iter()
            .filter(|(_, node)| matches(node))
            .map(|(id, _)| id.clone())
            .collect()
    } else {
        node_by_id.keys().cloned().collect()
    };
    let mut selected_ids = matched_ids.clone();
    if filtering && include_ancestors {
        let parents = parent_map(&node_list, &edge_list);
        for node_id in matched_ids.iter() {
            let mut seen = BTreeSet::from([node_id.clone()]);
            let mut parent = parents.get(node_id);
            while let Some(p) = parent {
                if seen.contains(p) || !node_by_id.contains_key(p) {
                    break;
                }
                selected_ids.insert(p.clone());
                seen.insert(p.clone());
                parent = parents.get(p);
            }
        }
    }
    let selected_nodes: Vec<Value> = node_list
        .iter()
        .filter(|node| {
            node.get("node_id")
                .and_then(Value::as_str)
                .is_some_and(|id| selected_ids.contains(id))
        })
        .cloned()
        .collect();
    let selected_edges: Vec<Value> = edge_list
        .iter()
        .filter(|edge| {
            let from = edge
                .get("from_node")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let to = edge
                .get("to_node")
                .and_then(Value::as_str)
                .unwrap_or_default();
            selected_ids.contains(from) && selected_ids.contains(to)
        })
        .cloned()
        .collect();
    Ok(serde_json::json!({
        "nodes": selected_nodes,
        "edges": selected_edges,
        "mermaid": build_explore_mermaid(&selected_nodes, &selected_edges, node_limit.max(1)),
        "graph_counts": {
            "node_count": selected_nodes.len(),
            "edge_count": selected_edges.len(),
            "matched_node_count": matched_ids.len(),
            "context_node_count": selected_ids.len() - matched_ids.len(),
        },
        "filter": {
            "active": filtering,
            "statuses": requested_statuses.iter().cloned().collect::<Vec<_>>(),
            "tags": requested_tags.iter().cloned().collect::<Vec<_>>(),
            "include_ancestors": include_ancestors,
            "semantics": "status_and_any_tag",
        },
    }))
}

/// Fold result events into the bounded projection display sinks render
/// (reference `build_explore_result_projection`).
pub fn build_explore_result_projection(
    events: &[Value],
    goal_id: &str,
    finding_limit: usize,
    mermaid_node_limit: usize,
) -> Result<Value, String> {
    let safe_goal_id = safe_goal_id(goal_id)?;
    let scoped: Vec<Value> = events
        .iter()
        .filter(|event| event.get("goal_id").and_then(Value::as_str) == Some(safe_goal_id.as_str()))
        .cloned()
        .collect();

    let folded_findings = fold_by_result_id(&scoped, EVENT_KIND_FINDING);
    let mut finding_counts: BTreeMap<String, usize> = BTreeMap::new();
    for finding in &folded_findings {
        if let Some(node_id) = finding.get("node_id").and_then(Value::as_str) {
            if !node_id.is_empty() {
                *finding_counts.entry(node_id.to_string()).or_default() += 1;
            }
        }
    }

    let mut nodes: Vec<Value> = fold_by_result_id(&scoped, EVENT_KIND_NODE)
        .iter()
        .map(|event| {
            node_view(
                event,
                finding_counts
                    .get(
                        event
                            .get("result_id")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                    )
                    .copied()
                    .unwrap_or(0),
            )
        })
        .collect();
    nodes.sort_by_key(|node| {
        node.get("first_recorded_at")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    });
    let mut edges: Vec<Value> = fold_by_result_id(&scoped, EVENT_KIND_EDGE)
        .iter()
        .map(edge_view)
        .collect();
    edges.sort_by_key(|edge| {
        edge.get("edge_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    });
    edges.extend(materialized_parent_edges(&nodes, &edges));
    let mut findings: Vec<Value> = folded_findings.iter().map(finding_view).collect();
    findings.sort_by(|a, b| {
        b.get("last_updated_at")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .cmp(
                a.get("last_updated_at")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            )
    });

    let mut nodes_by_status: BTreeMap<String, usize> = BTreeMap::new();
    for status in NODE_STATUSES {
        nodes_by_status.insert(status.to_string(), 0);
    }
    for node in &nodes {
        let status = node
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default();
        *nodes_by_status.entry(status.to_string()).or_default() += 1;
    }
    let mut findings_by_status: BTreeMap<String, usize> = BTreeMap::new();
    for status in FINDING_STATUSES {
        findings_by_status.insert(status.to_string(), 0);
    }
    for finding in &findings {
        let status = finding
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default();
        *findings_by_status.entry(status.to_string()).or_default() += 1;
    }

    let parents = parent_map(&nodes, &edges);
    let stuck: Vec<Value> = nodes
        .iter()
        .filter(|node| node.get("status").and_then(Value::as_str) == Some(NODE_STATUS_BLOCKED))
        .cloned()
        .collect();
    let frontier: Vec<Value> = nodes
        .iter()
        .filter(|node| node.get("status").and_then(Value::as_str) == Some(NODE_STATUS_EXPLORING))
        .cloned()
        .collect();
    Ok(serde_json::json!({
        "ok": true,
        "schema_version": EXPLORE_RESULT_PROJECTION_VERSION,
        "goal_id": safe_goal_id,
        "generated_at": now_iso(),
        "source_event_count": scoped.len(),
        "counts": {
            "node_count": nodes.len(),
            "edge_count": edges.len(),
            "finding_count": findings.len(),
            "nodes_by_status": nodes_by_status,
            "findings_by_status": findings_by_status,
        },
        "nodes": nodes,
        "edges": edges,
        "findings": if finding_limit > 0 { findings.into_iter().take(finding_limit).collect::<Vec<_>>() } else { Vec::new() },
        "stuck": stuck,
        "frontier": frontier,
        "tree": build_tree(&nodes, &parents, DEFAULT_TREE_DEPTH_LIMIT),
        "mermaid": build_explore_mermaid(&nodes, &edges, mermaid_node_limit.max(1)),
        "boundary": public_boundary(),
    }))
}

// ── hypothesis verification tracking ──────────────────────────────────────

/// Derive one hypothesis node's verification state from its node status, its
/// attached findings, and the incident `supports` / `refutes` edges
/// (evidence counts). The state is computed, never authored — prose claims
/// do not move it.
///
/// Ladder (first match wins):
/// 1. node status `blocked` → `blocked`;
/// 2. any refuted finding or incident `refutes` edge → `refuted`;
/// 3. any confirmed finding or incident `supports` edge → `supported`;
/// 4. node status `exploring`, any tentative finding, or an incident
///    `answers` / `leads_to` / `depends_on` edge → `testing`;
/// 5. otherwise → `unverified`.
///
/// `node` is a projection node view; `findings` / `edges` are projection
/// views. Incidents count in both directions (edge direction semantics stay
/// domain-owned).
pub fn build_hypothesis_verification(node: &Value, findings: &[Value], edges: &[Value]) -> Value {
    let node_id = node
        .get("node_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let node_status = node
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or(NODE_STATUS_OPEN);

    let mut confirmed = 0usize;
    let mut refuted = 0usize;
    let mut tentative = 0usize;
    let mut max_confidence: Option<f64> = None;
    for finding in findings {
        if finding.get("node_id").and_then(Value::as_str) != Some(node_id) {
            continue;
        }
        if let Some(confidence) = finding.get("confidence").and_then(Value::as_f64) {
            max_confidence = Some(max_confidence.map_or(confidence, |m| m.max(confidence)));
        }
        match finding.get("status").and_then(Value::as_str) {
            Some(FINDING_STATUS_CONFIRMED) => confirmed += 1,
            Some(FINDING_STATUS_REFUTED) => refuted += 1,
            Some(FINDING_STATUS_TENTATIVE) => tentative += 1,
            _ => {}
        }
    }
    let mut supports = 0usize;
    let mut refutes = 0usize;
    let mut activity = 0usize;
    for edge in edges {
        let from = edge
            .get("from_node")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let to = edge
            .get("to_node")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if from != node_id && to != node_id {
            continue;
        }
        match edge.get("edge_type").and_then(Value::as_str) {
            Some("supports") => supports += 1,
            Some("refutes") => refutes += 1,
            Some("answers" | "leads_to" | "depends_on") => activity += 1,
            _ => {}
        }
    }

    let state = if node_status == NODE_STATUS_BLOCKED {
        VERIFICATION_BLOCKED
    } else if refuted > 0 || refutes > 0 {
        VERIFICATION_REFUTED
    } else if confirmed > 0 || supports > 0 {
        VERIFICATION_SUPPORTED
    } else if node_status == NODE_STATUS_EXPLORING || tentative > 0 || activity > 0 {
        VERIFICATION_TESTING
    } else {
        VERIFICATION_UNVERIFIED
    };

    let mut hazards: Vec<&str> = Vec::new();
    if node_status == NODE_STATUS_DEAD_END {
        hazards.push("hypothesis_dead_end");
    }
    if node_status == NODE_STATUS_RESOLVED
        && confirmed == 0
        && refuted == 0
        && tentative == 0
        && supports == 0
        && refutes == 0
    {
        hazards.push("resolved_without_evidence");
    }
    if matches!(state, VERIFICATION_SUPPORTED | VERIFICATION_REFUTED)
        && matches!(node_status, NODE_STATUS_OPEN | NODE_STATUS_EXPLORING)
    {
        hazards.push("verdict_not_recorded_on_node");
    }

    let recommended_action = match state {
        VERIFICATION_BLOCKED => "unblock the hypothesis node (supply the missing input) or close it as dead_end with a blocked_reason",
        VERIFICATION_REFUTED => "record the refutation as a finding and close the hypothesis node (resolved or dead_end); pivot to the next open hypothesis",
        VERIFICATION_SUPPORTED => "record the supporting evidence and close the hypothesis node as resolved",
        VERIFICATION_TESTING => "advance the running experiment to a finding event, then re-evaluate the verification state",
        _ => "design one cheap exploration experiment and record its outcome as a finding event (confirmed/refuted)",
    };

    serde_json::json!({
        "schema_version": EXPLORE_HYPOTHESIS_VERIFICATION_VERSION,
        "hypothesis_id": node_id,
        "hypothesis": node.get("title").and_then(Value::as_str).unwrap_or_default(),
        "node_status": node_status,
        "verification_state": state,
        "evidence": {
            "confirmed_findings": confirmed,
            "refuted_findings": refuted,
            "tentative_findings": tentative,
            "supports_edges": supports,
            "refutes_edges": refutes,
        },
        "confidence": max_confidence.map(|c| (c * 1000.0).round() / 1000.0),
        "hazards": hazards,
        "recommended_action": recommended_action,
    })
}

/// Verification records for every hypothesis node in a projection.
pub fn hypothesis_verifications(projection: &Value) -> Vec<Value> {
    let nodes = projection
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let findings = projection
        .get("findings")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let edges = projection
        .get("edges")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    nodes
        .iter()
        .filter(|node| node.get("node_kind").and_then(Value::as_str) == Some(NODE_KIND_HYPOTHESIS))
        .map(|node| build_hypothesis_verification(node, &findings, &edges))
        .collect()
}

// ── capability ────────────────────────────────────────────────────────────

pub struct ExploreCapability;

impl ExploreCapability {
    /// A JSON explore payload, when present: `{"goal_id", "events"}` or
    /// `{"hypothesis"}`.
    fn payload(input: &str) -> Option<Value> {
        let value: Value = serde_json::from_str(input.trim()).ok()?;
        value.as_object()?;
        Some(value)
    }

    /// Free text carries explore intent when it names a hypothesis or
    /// research question (markers mirror the reference operator vocabulary).
    fn has_explore_marker(text: &str) -> bool {
        let lowered = text.to_lowercase();
        lowered.contains("hypothesis")
            || lowered.contains("假设")
            || lowered.contains("探索")
            || lowered.contains("research question")
            || lowered.contains("研究问题")
            || text.trim_end().ends_with('?')
    }

    /// Strip a leading `hypothesis:` / `假设：` marker for a cleaner claim.
    fn strip_claim_prefix(claim: &str) -> String {
        let text = claim.trim();
        for prefix in ["hypothesis", "假设", "hypotheses"] {
            if let Some(rest) = text.strip_prefix(prefix) {
                if rest.starts_with([':', '：']) {
                    return rest.trim_start_matches([':', '：', ' ']).to_string();
                }
                continue;
            }
            // ASCII prefixes are single-byte but may still split a
            // multi-byte tail — only split at a char boundary.
            if prefix.is_ascii()
                && text.len() > prefix.len()
                && text.is_char_boundary(prefix.len())
                && text[..prefix.len()].eq_ignore_ascii_case(prefix)
                && text[prefix.len()..].starts_with([':', '：'])
            {
                return text[prefix.len()..]
                    .trim_start_matches([':', '：', ' '])
                    .to_string();
            }
        }
        text.to_string()
    }
}

impl Capability for ExploreCapability {
    fn name(&self) -> &'static str {
        "explore"
    }
    fn describe(&self) -> &'static str {
        "model a hypothesis, track its verification state through findings and evidence edges, and propose the next bounded exploration probe from the explore graph"
    }

    fn propose(&self, input: &str) -> Vec<TypedProposal> {
        let text = input.trim();
        if text.is_empty() {
            return vec![TypedProposal::no_followup("empty input for explore")];
        }

        // Payload path: a hypothesis claim or an events observation.
        if let Some(payload) = Self::payload(text) {
            if let Some(claim) = payload.get("hypothesis").and_then(Value::as_str) {
                return Self::hypothesis_proposals(claim);
            }
            let Some(goal_id) = payload.get("goal_id").and_then(Value::as_str) else {
                return vec![TypedProposal::gate(
                    "Provide an explore payload: {\"goal_id\": …, \"events\": […]} for a graph observation, or {\"hypothesis\": \"…\"} for a claim.",
                    "explore payload requires goal_id with events, or hypothesis",
                )];
            };
            let Some(events) = payload.get("events").and_then(Value::as_array) else {
                return vec![TypedProposal::gate(
                    "Provide an explore payload: {\"goal_id\": …, \"events\": […]} for a graph observation, or {\"hypothesis\": \"…\"} for a claim.",
                    "explore payload requires an events array, or hypothesis",
                )];
            };
            return Self::graph_proposals(goal_id, events);
        }

        // Free-text path: marker text models a hypothesis; anything else
        // asks for clarification before acting (missing signal).
        if Self::has_explore_marker(text) {
            Self::hypothesis_proposals(text)
        } else {
            vec![TypedProposal::successor(
                successor_todo(
                    "clarify",
                    "Clarify the request before acting (missing explore signal).",
                ),
                "rule: no explore marker matched",
            )]
        }
    }
}

impl ExploreCapability {
    /// Model a free-text claim as a canonical hypothesis node event and
    /// propose the first experiment.
    fn hypothesis_proposals(claim: &str) -> Vec<TypedProposal> {
        let claim = Self::strip_claim_prefix(claim);
        match build_explore_node_event(
            "(proposal)",
            &claim,
            None,
            Some(NODE_KIND_HYPOTHESIS),
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            &[],
            None,
            None,
        ) {
            Err(err) => vec![TypedProposal::gate(
                &format!("Fix the hypothesis claim before modeling it: {err}"),
                "hypothesis claim rejected by the event contract",
            )],
            Ok(event) => {
                let node_id = event
                    .get("result_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let mut todo = successor_todo(
                    "explore",
                    &format!(
                        "Design one cheap exploration experiment to test the hypothesis `{claim}` (node {node_id}); record the outcome as a finding event attached to the node (confirmed/refuted), then re-evaluate the verification state."
                    ),
                );
                todo.action_kind = Some("run_exploration_probe".to_string());
                todo.required_capability = Some("explore".to_string());
                todo.capability_binding_ref = Some("explore".to_string());
                vec![TypedProposal::successor(
                    todo,
                    &format!("hypothesis modeled as node {node_id}; verification unverified — propose the first experiment"),
                )]
            }
        }
    }

    /// Observe an events payload: validate every event, fold the projection,
    /// derive hypothesis verifications, and propose at most one typed next
    /// step (blocked → experiment → refutation closeout → frontier monitor →
    /// open node → settled).
    fn graph_proposals(goal_id: &str, events: &[Value]) -> Vec<TypedProposal> {
        if events.is_empty() {
            return vec![TypedProposal::gate(
                "Provide at least one explore result event before proposing (the events array is empty).",
                "explore observation has no events",
            )];
        }
        let mut validated: Vec<Value> = Vec::new();
        for (index, event) in events.iter().enumerate() {
            match validate_explore_result_event(event, Some(goal_id)) {
                Ok(canonical) => validated.push(canonical),
                Err(err) => {
                    return vec![TypedProposal::gate(
                        &format!("Explore event {index} rejected: {err}. Fix the payload before re-observing."),
                        "explore payload rejected by the event contract",
                    )];
                }
            }
        }
        // `build_explore_result_projection` can only fail on an unsafe
        // goal_id, and every event above already passed
        // `validate_explore_result_event(.., Some(goal_id))`, which checks the
        // same invariant — so this cannot fail here.
        let projection = build_explore_result_projection(
            &validated,
            goal_id,
            DEFAULT_FINDING_LIMIT,
            DEFAULT_MERMAID_NODE_LIMIT,
        )
        .expect("goal_id was validated as a single path segment upstream");
        let nodes: Vec<Value> = projection
            .get("nodes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let verifications = hypothesis_verifications(&projection);

        // 1. Blocked nodes first: the graph cannot advance while stuck.
        if let Some(blocked) = nodes
            .iter()
            .find(|node| node.get("status").and_then(Value::as_str) == Some(NODE_STATUS_BLOCKED))
        {
            let title = blocked
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let reason = blocked
                .get("blocked_reason")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let node_id = blocked
                .get("node_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let mut todo = successor_todo(
                "explore",
                &format!(
                    "Resolve the exploration block on `{title}` (node {node_id}): {reason}. Supply the missing input or close the node as dead_end, then re-observe the graph."
                ),
            );
            todo.action_kind = Some("resolve_exploration_block".to_string());
            todo.required_capability = Some("explore".to_string());
            todo.capability_binding_ref = Some("explore".to_string());
            return vec![TypedProposal::successor(
                todo,
                "graph has a blocked exploration node — unblock it before any new probe",
            )];
        }

        // 2. Unverified hypotheses get the next experiment design.
        if let Some(verification) = verifications.iter().find(|v| {
            v.get("verification_state").and_then(Value::as_str) == Some(VERIFICATION_UNVERIFIED)
        }) {
            let hypothesis = verification
                .get("hypothesis")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let node_id = verification
                .get("hypothesis_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let mut todo = successor_todo(
                "explore",
                &format!(
                    "Design one cheap exploration experiment to test the hypothesis `{hypothesis}` (node {node_id}); record the outcome as a finding event attached to the node (confirmed/refuted), then re-observe the graph."
                ),
            );
            todo.action_kind = Some("run_exploration_probe".to_string());
            todo.required_capability = Some("explore".to_string());
            todo.capability_binding_ref = Some("explore".to_string());
            return vec![TypedProposal::successor(
                todo,
                "an unverified hypothesis remains — propose its first experiment",
            )];
        }

        // 3. Testing hypotheses advance their running experiment to a finding.
        if let Some(verification) = verifications.iter().find(|v| {
            v.get("verification_state").and_then(Value::as_str) == Some(VERIFICATION_TESTING)
        }) {
            let hypothesis = verification
                .get("hypothesis")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let node_id = verification
                .get("hypothesis_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let mut todo = successor_todo(
                "explore",
                &format!(
                    "Advance the running experiment for hypothesis `{hypothesis}` (node {node_id}) to a finding event; then re-evaluate the verification state."
                ),
            );
            todo.action_kind = Some("advance_exploration_experiment".to_string());
            todo.required_capability = Some("explore".to_string());
            todo.capability_binding_ref = Some("explore".to_string());
            return vec![TypedProposal::successor(
                todo,
                "a hypothesis is under test — advance the experiment to a finding",
            )];
        }

        // 4. Refuted hypotheses close out and pivot.
        if let Some(verification) = verifications.iter().find(|v| {
            v.get("verification_state").and_then(Value::as_str) == Some(VERIFICATION_REFUTED)
        }) {
            let hypothesis = verification
                .get("hypothesis")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let node_id = verification
                .get("hypothesis_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let mut todo = successor_todo(
                "explore",
                &format!(
                    "Record the refutation of hypothesis `{hypothesis}` (node {node_id}) and close the node (resolved/dead_end); pivot to the next open hypothesis."
                ),
            );
            todo.action_kind = Some("close_refuted_hypothesis".to_string());
            todo.required_capability = Some("explore".to_string());
            todo.capability_binding_ref = Some("explore".to_string());
            return vec![TypedProposal::successor(
                todo,
                "a hypothesis is refuted — record the closeout and pivot",
            )];
        }

        // 5. In-flight exploration: periodic re-observation, no new work.
        if nodes
            .iter()
            .any(|node| node.get("status").and_then(Value::as_str) == Some(NODE_STATUS_EXPLORING))
        {
            let mut todo = monitor_todo(
                "explore",
                &format!(
                    "Re-observe the exploration graph for goal `{goal_id}`; the exploring frontier is claimed — propose nothing new unless the graph changed."
                ),
                REOBSERVE_CADENCE_MINUTES * 60,
            );
            todo.monitor_target = Some(format!("explore-graph:{goal_id}"));
            todo.monitor_policy =
                Some("read_only_observation_then_no_spend_if_unchanged".to_string());
            todo.monitor_cadence = Some(REOBSERVE_CADENCE.to_string());
            return vec![TypedProposal::monitor(
                todo,
                "exploration frontier is in flight — periodic graph re-observation",
            )];
        }

        // 6. Open nodes still need the next transition.
        if let Some(open) = nodes
            .iter()
            .find(|node| node.get("status").and_then(Value::as_str) == Some(NODE_STATUS_OPEN))
        {
            let title = open
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let node_id = open
                .get("node_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let mut todo = successor_todo(
                "explore",
                &format!(
                    "Advance the open exploration node `{title}` (node {node_id}): pick the next question or experiment transition and record it as an event."
                ),
            );
            todo.action_kind = Some("advance_exploration_node".to_string());
            todo.required_capability = Some("explore".to_string());
            todo.capability_binding_ref = Some("explore".to_string());
            return vec![TypedProposal::successor(
                todo,
                "open exploration nodes remain — advance the next transition",
            )];
        }

        vec![TypedProposal::no_followup(
            "exploration graph settled — no blocked, unverified, or open nodes remain",
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::ProposalKind;

    fn at(day: u32) -> String {
        format!("2026-08-{day:02}T08:00:00Z")
    }

    fn node(
        id: &str,
        kind: &str,
        status: &str,
        title: &str,
        recorded_at: &str,
        blocked_reason: Option<&str>,
    ) -> Value {
        build_explore_node_event(
            "g",
            title,
            Some(id),
            Some(kind),
            Some(status),
            None,
            blocked_reason,
            None,
            None,
            None,
            &[],
            &[],
            None,
            Some(recorded_at),
        )
        .expect("node event builds")
    }

    fn finding(
        id: &str,
        node_id: &str,
        status: &str,
        title: &str,
        confidence: Option<f64>,
        recorded_at: &str,
    ) -> Value {
        build_explore_finding_event(
            "g",
            title,
            Some(id),
            Some(node_id),
            Some(status),
            None,
            confidence,
            None,
            None,
            &[],
            &[],
            None,
            Some(recorded_at),
        )
        .expect("finding event builds")
    }

    fn edge(
        from_node: &str,
        edge_type: &str,
        to_node: &str,
        confidence: Option<f64>,
        recorded_at: &str,
    ) -> Value {
        build_explore_edge_event(
            "g",
            from_node,
            to_node,
            edge_type,
            None,
            confidence,
            None,
            None,
            Some(recorded_at),
        )
        .expect("edge event builds")
    }

    // ── safety + ids ──────────────────────────────────────────────────────

    #[test]
    fn compact_text_rejects_private_material() {
        assert!(compact_text("api_key: x", 200, "title").is_err());
        assert!(compact_text("see /Users/me/file", 200, "title").is_err());
        assert!(compact_text("password here", 200, "title").is_err());
        assert!(compact_text("file:///etc/passwd", 200, "title").is_err());
        assert!(compact_text(r"C:\Users\me\x", 200, "title").is_err());
        assert!(compact_text("plain claim", 200, "title").is_ok());
        // Truncation appends the ellipsis.
        let truncated = compact_text("a".repeat(300).as_str(), 200, "title").unwrap();
        assert_eq!(truncated.len(), 200);
        assert!(truncated.ends_with("..."));
    }

    #[test]
    fn result_id_pattern_and_goal_id() {
        assert!(safe_result_id("h_1", "node_id").is_ok());
        assert!(safe_result_id("a.b:c-d", "node_id").is_ok());
        assert!(safe_result_id("1bad", "node_id").is_err());
        assert!(safe_result_id("has space", "node_id").is_err());
        assert!(safe_result_id(&"a".repeat(97), "node_id").is_err());
        assert!(safe_goal_id("goal-a").is_ok());
        assert!(safe_goal_id("a/b").is_err());
        assert!(safe_goal_id("..").is_err());
        assert!(safe_confidence(Some(0.1234)) == Ok(Some(0.123)));
        assert!(safe_confidence(Some(1.5)).is_err());
        assert!(safe_confidence(None) == Ok(None));
    }

    // ── node / edge / finding events ───────────────────────────────────────

    #[test]
    fn node_event_defaults_and_derived_id() {
        let event = build_explore_node_event(
            "g",
            "Claims are testable",
            None,
            Some(NODE_KIND_HYPOTHESIS),
            None,
            Some("summary here"),
            None,
            None,
            Some("agent-1"),
            Some("run-x"),
            &[],
            &[],
            None,
            Some("2026-08-01T00:00:00Z"),
        )
        .unwrap();
        assert_eq!(event["status"], NODE_STATUS_OPEN);
        assert_eq!(event["node_kind"], NODE_KIND_HYPOTHESIS);
        assert_eq!(
            event["result_id"],
            derived_result_id("node", &["g", "claims are testable"])
        );
        assert_eq!(event["goal_id"], "g");
        assert!(event["event_id"].as_str().unwrap().len() == 16);
        assert_eq!(event["boundary"], public_boundary());
        // Explicit node id is preserved verbatim.
        let explicit = build_explore_node_event(
            "g",
            "t",
            Some("h_7"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            &[],
            None,
            Some("2026-08-01T00:00:00Z"),
        )
        .unwrap();
        assert_eq!(explicit["result_id"], "h_7");
        // Blocked nodes must state a reason.
        assert!(build_explore_node_event(
            "g",
            "t",
            None,
            None,
            Some(NODE_STATUS_BLOCKED),
            None,
            None,
            None,
            None,
            None,
            &[],
            &[],
            None,
            None,
        )
        .is_err());
        // Unknown status / kind fail closed.
        assert!(build_explore_node_event(
            "g",
            "t",
            None,
            Some("ghost-kind"),
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            &[],
            None,
            None,
        )
        .is_err());
        assert!(build_explore_node_event(
            "g",
            "t",
            None,
            None,
            Some("ghost-status"),
            None,
            None,
            None,
            None,
            None,
            &[],
            &[],
            None,
            None,
        )
        .is_err());
    }

    #[test]
    fn edge_event_rules() {
        let ok = build_explore_edge_event(
            "g",
            "h_1",
            "e_2",
            "answers",
            None,
            Some(0.8),
            None,
            None,
            Some("2026-08-01T00:00:00Z"),
        )
        .unwrap();
        assert_eq!(ok["from_node"], "h_1");
        assert_eq!(ok["edge_type"], "answers");
        assert_eq!(ok["confidence"], 0.8);
        assert!(build_explore_edge_event(
            "g", "h_1", "h_1", "supports", None, None, None, None, None,
        )
        .is_err());
        assert!(build_explore_edge_event(
            "g",
            "h_1",
            "h_2",
            "ghost-edge",
            None,
            None,
            None,
            None,
            None,
        )
        .is_err());
    }

    #[test]
    fn finding_event_defaults_and_attachment() {
        let event = build_explore_finding_event(
            "g",
            "the probe passed",
            None,
            Some("h_1"),
            None,
            None,
            Some(0.95),
            None,
            None,
            &[],
            &[],
            None,
            Some("2026-08-01T00:00:00Z"),
        )
        .unwrap();
        assert_eq!(event["status"], FINDING_STATUS_TENTATIVE);
        assert_eq!(event["node_id"], "h_1");
        assert_eq!(
            event["result_id"],
            derived_result_id("finding", &["g", "the probe passed"])
        );
        assert_eq!(event["confidence"], 0.95);
    }

    // ── validation ────────────────────────────────────────────────────────

    #[test]
    fn validation_roundtrip_and_rejections() {
        let canonical = node(
            "h_1",
            NODE_KIND_HYPOTHESIS,
            NODE_STATUS_OPEN,
            "claim",
            "2026-08-01T00:00:00Z",
            None,
        );
        assert_eq!(
            validate_explore_result_event(&canonical, Some("g")).unwrap(),
            canonical
        );
        // Unknown fields fail closed.
        let mut forged = canonical.clone();
        forged["secret"] = Value::String("x".into());
        assert!(validate_explore_result_event(&forged, Some("g")).is_err());
        // Forged boundary flags fail closed.
        let mut forged_boundary = canonical.clone();
        forged_boundary["boundary"]["raw_logs_recorded"] = Value::Bool(true);
        assert!(validate_explore_result_event(&forged_boundary, Some("g")).is_err());
        // Wrong schema / kind / goal fail closed.
        let mut wrong_schema = canonical.clone();
        wrong_schema["schema_version"] = Value::String("v1".into());
        assert!(validate_explore_result_event(&wrong_schema, Some("g")).is_err());
        let mut wrong_goal = canonical.clone();
        wrong_goal["goal_id"] = Value::String("other".into());
        assert!(validate_explore_result_event(&wrong_goal, Some("g")).is_err());
        let mut wrong_kind = canonical.clone();
        wrong_kind["event_kind"] = Value::String("ghost".into());
        assert!(validate_explore_result_event(&wrong_kind, Some("g")).is_err());
    }

    // ── projection + graph view ────────────────────────────────────────────

    #[test]
    fn projection_folds_counts_and_surfaces() {
        let events = vec![
            node(
                "h_1",
                NODE_KIND_HYPOTHESIS,
                NODE_STATUS_RESOLVED,
                "fast path",
                "2026-08-01T00:00:00Z",
                None,
            ),
            node(
                "e_1",
                NODE_KIND_EXPERIMENT,
                NODE_STATUS_EXPLORING,
                "probe",
                "2026-08-01T00:00:00Z",
                None,
            ),
            node(
                "b_1",
                NODE_KIND_AREA,
                NODE_STATUS_BLOCKED,
                "blocked area",
                "2026-08-01T00:00:00Z",
                Some("missing API access"),
            ),
            edge("e_1", "answers", "h_1", Some(0.9), "2026-08-01T00:00:00Z"),
            finding(
                "f_1",
                "h_1",
                FINDING_STATUS_CONFIRMED,
                "it holds",
                Some(0.8),
                "2026-08-01T00:00:00Z",
            ),
            // A second update of the same hypothesis node: folds, does not duplicate.
            node(
                "h_1",
                NODE_KIND_HYPOTHESIS,
                NODE_STATUS_RESOLVED,
                "fast path",
                "2026-08-02T00:00:00Z",
                None,
            ),
        ];
        let projection = build_explore_result_projection(
            &events,
            "g",
            DEFAULT_FINDING_LIMIT,
            DEFAULT_MERMAID_NODE_LIMIT,
        )
        .unwrap();
        assert_eq!(projection["ok"], true);
        assert_eq!(projection["goal_id"], "g");
        assert_eq!(projection["source_event_count"], 6);
        assert_eq!(projection["counts"]["node_count"], 3);
        assert_eq!(projection["counts"]["edge_count"], 1); // the answers edge; no parent links here
        assert_eq!(projection["counts"]["finding_count"], 1);
        let nodes = projection["nodes"].as_array().unwrap();
        let h1 = nodes.iter().find(|n| n["node_id"] == "h_1").unwrap();
        assert_eq!(h1["update_count"], 2);
        assert_eq!(h1["first_recorded_at"], "2026-08-01T00:00:00Z");
        assert_eq!(h1["last_updated_at"], "2026-08-02T00:00:00Z");
        assert_eq!(h1["finding_count"], 1);
        assert_eq!(projection["stuck"].as_array().unwrap().len(), 1);
        assert_eq!(projection["frontier"].as_array().unwrap().len(), 1);
        let mermaid = projection["mermaid"].as_str().unwrap();
        assert!(mermaid.starts_with("flowchart TD"));
        assert!(mermaid.contains("(BLOCKED)"));
        assert!(mermaid.contains(":::blocked"));
        assert!(mermaid.contains(":::exploring"));
    }

    #[test]
    fn projection_materializes_parent_edges_and_tree() {
        let events = vec![
            node(
                "area_1",
                NODE_KIND_AREA,
                NODE_STATUS_OPEN,
                "root",
                "2026-08-01T00:00:00Z",
                None,
            ),
            node(
                "h_1",
                NODE_KIND_HYPOTHESIS,
                NODE_STATUS_OPEN,
                "claim",
                "2026-08-01T00:00:00Z",
                None,
            ),
            // h_1's second update carries parent_id → a derived supports edge.
            {
                let mut updated = node(
                    "h_1",
                    NODE_KIND_HYPOTHESIS,
                    NODE_STATUS_OPEN,
                    "claim",
                    "2026-08-02T00:00:00Z",
                    None,
                );
                updated["parent_id"] = Value::String("area_1".into());
                updated["event_id"] = Value::String(event_id(&updated));
                updated
            },
        ];
        let projection = build_explore_result_projection(
            &events,
            "g",
            DEFAULT_FINDING_LIMIT,
            DEFAULT_MERMAID_NODE_LIMIT,
        )
        .unwrap();
        let edges = projection["edges"].as_array().unwrap();
        let derived = edges
            .iter()
            .find(|e| e.get("materialized_from").and_then(Value::as_str) == Some("node_parent_id"))
            .unwrap();
        assert_eq!(derived["from_node"], "area_1");
        assert_eq!(derived["to_node"], "h_1");
        assert_eq!(derived["edge_type"], "supports");
        assert_eq!(derived["confidence"], 1.0);
        let tree = projection["tree"].as_array().unwrap();
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0]["node_id"], "area_1");
        assert_eq!(tree[0]["children"][0]["node_id"], "h_1");
    }

    #[test]
    fn graph_view_filters_with_ancestor_context() {
        let events = vec![
            node(
                "area_1",
                NODE_KIND_AREA,
                NODE_STATUS_OPEN,
                "root area",
                &at(1),
                None,
            ),
            node(
                "h_1",
                NODE_KIND_HYPOTHESIS,
                NODE_STATUS_RESOLVED,
                "supported claim",
                &at(1),
                None,
            ),
            node(
                "h_2",
                NODE_KIND_HYPOTHESIS,
                NODE_STATUS_OPEN,
                "open claim",
                &at(1),
                None,
            ),
            edge("h_1", EDGE_TYPE_SUBTOPIC_OF, "area_1", None, &at(1)),
        ];
        let (nodes, _, edges) = views(events);
        let view = build_explore_graph_view(&nodes, &edges, &[NODE_STATUS_RESOLVED], &[], true, 60)
            .unwrap();
        assert_eq!(view["graph_counts"]["matched_node_count"], 1);
        assert_eq!(view["graph_counts"]["context_node_count"], 1);
        let ids: Vec<&str> = view["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|n| n["node_id"].as_str())
            .collect();
        assert!(ids.contains(&"h_1") && ids.contains(&"area_1"));
        // AND semantics: status + tag.
        let none = build_explore_graph_view(
            &nodes,
            &edges,
            &[NODE_STATUS_RESOLVED],
            &["ghost-tag"],
            true,
            60,
        )
        .unwrap();
        assert_eq!(none["graph_counts"]["node_count"], 0);
        // Unknown status fails closed.
        assert!(build_explore_graph_view(&nodes, &edges, &["ghost"], &[], true, 60).is_err());
    }

    // ── hypothesis verification tracking ───────────────────────────────────

    /// Projection views (node/finding/edge) for a list of events.
    fn views(events: Vec<Value>) -> (Vec<Value>, Vec<Value>, Vec<Value>) {
        let projection = build_explore_result_projection(
            &events,
            "g",
            DEFAULT_FINDING_LIMIT,
            DEFAULT_MERMAID_NODE_LIMIT,
        )
        .unwrap();
        let nodes = projection["nodes"].as_array().unwrap().clone();
        let findings = projection["findings"].as_array().unwrap().clone();
        let edges = projection["edges"].as_array().unwrap().clone();
        (nodes, findings, edges)
    }

    fn verify(node: &Value, findings: &[Value], edges: &[Value]) -> Value {
        build_hypothesis_verification(node, findings, edges)
    }

    #[test]
    fn verification_state_ladder() {
        let (nodes, findings, edges) = views(vec![
            node(
                "h_1",
                NODE_KIND_HYPOTHESIS,
                NODE_STATUS_OPEN,
                "claim",
                &at(1),
                None,
            ),
            node(
                "h_2",
                NODE_KIND_HYPOTHESIS,
                NODE_STATUS_EXPLORING,
                "claim",
                &at(1),
                None,
            ),
            node(
                "h_3",
                NODE_KIND_HYPOTHESIS,
                NODE_STATUS_DEAD_END,
                "claim",
                &at(1),
                None,
            ),
            node(
                "h_4",
                NODE_KIND_HYPOTHESIS,
                NODE_STATUS_RESOLVED,
                "claim",
                &at(1),
                None,
            ),
            node(
                "b_1",
                NODE_KIND_HYPOTHESIS,
                NODE_STATUS_BLOCKED,
                "claim",
                &at(1),
                Some("no API"),
            ),
            finding(
                "f_1",
                "h_1",
                FINDING_STATUS_TENTATIVE,
                "probe running",
                None,
                &at(2),
            ),
            finding(
                "f_2",
                "h_1",
                FINDING_STATUS_CONFIRMED,
                "it holds",
                Some(0.8),
                &at(2),
            ),
            finding(
                "f_3",
                "h_1",
                FINDING_STATUS_REFUTED,
                "it fails",
                None,
                &at(2),
            ),
            edge("h_1", "supports", "e_1", Some(0.9), &at(2)),
            edge("e_2", "refutes", "h_1", None, &at(2)),
        ]);
        let by_id = |id: &str| {
            nodes
                .iter()
                .find(|n| n["node_id"] == id)
                .unwrap_or_else(|| panic!("missing node {id}"))
        };
        let open = by_id("h_1");

        // All three findings + both evidence edges attach to h_1: refuted
        // wins over supported (the ladder checks refutation first).
        let record = verify(open, &findings, &edges);
        assert_eq!(record["verification_state"], VERIFICATION_REFUTED);
        assert_eq!(record["evidence"]["confirmed_findings"], 1);
        assert_eq!(record["evidence"]["refuted_findings"], 1);
        assert_eq!(record["evidence"]["tentative_findings"], 1);
        assert_eq!(record["evidence"]["supports_edges"], 1);
        assert_eq!(record["evidence"]["refutes_edges"], 1);
        assert!(record["hazards"]
            .as_array()
            .unwrap()
            .iter()
            .any(|h| h == "verdict_not_recorded_on_node"));

        // No evidence at all → unverified.
        let fresh = by_id("h_4");
        let record = verify(fresh, &[], &[]);
        assert_eq!(record["verification_state"], VERIFICATION_UNVERIFIED);
        assert!(record["recommended_action"]
            .as_str()
            .unwrap()
            .contains("cheap exploration experiment"));

        // Blocked wins first.
        let record = verify(by_id("b_1"), &[], &[]);
        assert_eq!(record["verification_state"], VERIFICATION_BLOCKED);

        // Exploring status alone → testing.
        let record = verify(by_id("h_2"), &[], &[]);
        assert_eq!(record["verification_state"], VERIFICATION_TESTING);

        // A lone tentative finding → testing.
        let (nodes, findings, _) = views(vec![
            node(
                "h_1",
                NODE_KIND_HYPOTHESIS,
                NODE_STATUS_OPEN,
                "claim",
                &at(1),
                None,
            ),
            finding(
                "f_1",
                "h_1",
                FINDING_STATUS_TENTATIVE,
                "probe running",
                None,
                &at(2),
            ),
        ]);
        let record = verify(
            nodes.iter().find(|n| n["node_id"] == "h_1").unwrap(),
            &findings,
            &[],
        );
        assert_eq!(record["verification_state"], VERIFICATION_TESTING);

        // A lone confirmed finding → supported; confidence carries over.
        let (nodes, findings, _) = views(vec![
            node(
                "h_1",
                NODE_KIND_HYPOTHESIS,
                NODE_STATUS_RESOLVED,
                "claim",
                &at(1),
                None,
            ),
            finding(
                "f_2",
                "h_1",
                FINDING_STATUS_CONFIRMED,
                "it holds",
                Some(0.8),
                &at(2),
            ),
        ]);
        let record = verify(
            nodes.iter().find(|n| n["node_id"] == "h_1").unwrap(),
            &findings,
            &[],
        );
        assert_eq!(record["verification_state"], VERIFICATION_SUPPORTED);
        assert_eq!(record["evidence"]["confirmed_findings"], 1);
        assert_eq!(record["confidence"], 0.8);
        assert!(!record["hazards"]
            .as_array()
            .unwrap()
            .iter()
            .any(|h| h == "verdict_not_recorded_on_node"));

        // supports / refutes edges count in both directions.
        let (nodes, _, edges) = views(vec![
            node(
                "h_1",
                NODE_KIND_HYPOTHESIS,
                NODE_STATUS_OPEN,
                "claim",
                &at(1),
                None,
            ),
            edge("h_1", "supports", "e_1", Some(0.9), &at(2)),
        ]);
        let record = verify(
            nodes.iter().find(|n| n["node_id"] == "h_1").unwrap(),
            &[],
            &edges,
        );
        assert_eq!(record["verification_state"], VERIFICATION_SUPPORTED);
        let (nodes, _, edges) = views(vec![
            node(
                "h_1",
                NODE_KIND_HYPOTHESIS,
                NODE_STATUS_OPEN,
                "claim",
                &at(1),
                None,
            ),
            edge("e_2", "refutes", "h_1", None, &at(2)),
        ]);
        let record = verify(
            nodes.iter().find(|n| n["node_id"] == "h_1").unwrap(),
            &[],
            &edges,
        );
        assert_eq!(record["verification_state"], VERIFICATION_REFUTED);

        // Dead-end hazards surface; resolved without evidence is a hazard.
        let record = verify(by_id("h_3"), &[], &[]);
        assert!(record["hazards"]
            .as_array()
            .unwrap()
            .iter()
            .any(|h| h == "hypothesis_dead_end"));
        let record = verify(by_id("h_4"), &[], &[]);
        assert_eq!(record["verification_state"], VERIFICATION_UNVERIFIED);
        assert!(record["hazards"]
            .as_array()
            .unwrap()
            .iter()
            .any(|h| h == "resolved_without_evidence"));
    }

    // ── propose ────────────────────────────────────────────────────────────

    #[test]
    fn propose_empty_and_non_marker_text() {
        let cap = ExploreCapability;
        let proposals = cap.propose("   ");
        assert_eq!(proposals[0].kind, ProposalKind::NoFollowUp);
        // No marker → clarify (missing signal).
        let proposals = cap.propose("decide whether to ship the report");
        assert_eq!(proposals[0].kind, ProposalKind::SuccessorTodo);
        assert!(proposals[0].todo.as_ref().unwrap().text.contains("Clarify"));
    }

    #[test]
    fn propose_models_a_free_text_hypothesis() {
        let cap = ExploreCapability;
        for input in [
            "hypothesis: bigger prompts help",
            "假设：更大的提示更好",
            "探索 better search",
            "research question? why does it fail",
        ] {
            let proposals = cap.propose(input);
            assert_eq!(proposals.len(), 1, "{input:?}");
            assert_eq!(proposals[0].kind, ProposalKind::SuccessorTodo);
            let todo = proposals[0].todo.as_ref().unwrap();
            assert_eq!(todo.action_kind.as_deref(), Some("run_exploration_probe"));
            assert_eq!(todo.required_capability.as_deref(), Some("explore"));
            assert_eq!(todo.capability_binding_ref.as_deref(), Some("explore"));
            assert!(todo.text.contains("hypothesis"), "{input:?}");
            assert!(proposals[0].reason.contains("unverified"), "{input:?}");
        }
        // The `hypothesis:` prefix is stripped from the modeled claim.
        let proposals = cap.propose("hypothesis: bigger prompts help");
        assert!(
            !proposals[0]
                .todo
                .as_ref()
                .unwrap()
                .text
                .contains("hypothesis:"),
            "{proposals:?}"
        );
        // Unsafe claims gate instead of proposing work.
        let proposals = cap.propose("hypothesis: the secret is api_key=abc");
        assert_eq!(proposals[0].kind, ProposalKind::Gate);
    }

    #[test]
    fn propose_json_hypothesis_and_bad_payloads() {
        let cap = ExploreCapability;
        let proposals = cap.propose(r#"{"hypothesis": "wider context wins"}"#);
        assert_eq!(proposals[0].kind, ProposalKind::SuccessorTodo);
        assert_eq!(
            proposals[0].todo.as_ref().unwrap().action_kind.as_deref(),
            Some("run_exploration_probe")
        );
        // Missing goal_id / events → gate.
        let proposals = cap.propose(r#"{"events": []}"#);
        assert_eq!(proposals[0].kind, ProposalKind::Gate);
        let proposals = cap.propose(r#"{"goal_id": "g"}"#);
        assert_eq!(proposals[0].kind, ProposalKind::Gate);
        // Empty events → gate.
        let proposals = cap.propose(r#"{"goal_id": "g", "events": []}"#);
        assert_eq!(proposals[0].kind, ProposalKind::Gate);
    }

    #[test]
    fn propose_graph_ladder() {
        let cap = ExploreCapability;
        let hypothesis = node(
            "h_1",
            NODE_KIND_HYPOTHESIS,
            NODE_STATUS_OPEN,
            "open claim",
            &at(1),
            None,
        );
        let experiment = node(
            "e_1",
            NODE_KIND_EXPERIMENT,
            NODE_STATUS_OPEN,
            "probe",
            &at(1),
            None,
        );

        // Unverified hypothesis → experiment design.
        let input = serde_json::json!({"goal_id": "g", "events": [hypothesis]}).to_string();
        let proposals = cap.propose(&input);
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].kind, ProposalKind::SuccessorTodo);
        assert_eq!(
            proposals[0].todo.as_ref().unwrap().action_kind.as_deref(),
            Some("run_exploration_probe")
        );

        // A refuted hypothesis → closeout + pivot.
        let refuted_finding = finding(
            "f_1",
            "h_1",
            FINDING_STATUS_REFUTED,
            "it fails",
            None,
            &at(2),
        );
        let input = serde_json::json!({"goal_id": "g", "events": [hypothesis, refuted_finding]})
            .to_string();
        let proposals = cap.propose(&input);
        assert_eq!(
            proposals[0].todo.as_ref().unwrap().action_kind.as_deref(),
            Some("close_refuted_hypothesis")
        );

        // A blocked node wins over everything.
        let blocked = node(
            "b_1",
            NODE_KIND_AREA,
            NODE_STATUS_BLOCKED,
            "stuck area",
            &at(1),
            Some("missing access"),
        );
        let input = serde_json::json!({"goal_id": "g", "events": [hypothesis.clone(), blocked]})
            .to_string();
        let proposals = cap.propose(&input);
        assert_eq!(
            proposals[0].todo.as_ref().unwrap().action_kind.as_deref(),
            Some("resolve_exploration_block")
        );

        // An exploring frontier with no open hypotheses → monitor.
        let exploring = node(
            "e_2",
            NODE_KIND_EXPERIMENT,
            NODE_STATUS_EXPLORING,
            "running probe",
            &at(1),
            None,
        );
        let supported_h = node(
            "h_2",
            NODE_KIND_HYPOTHESIS,
            NODE_STATUS_RESOLVED,
            "settled",
            &at(1),
            None,
        );
        let confirmed = finding(
            "f_2",
            "h_2",
            FINDING_STATUS_CONFIRMED,
            "holds",
            None,
            &at(2),
        );
        let input =
            serde_json::json!({"goal_id": "g", "events": [supported_h, confirmed, exploring]})
                .to_string();
        let proposals = cap.propose(&input);
        assert_eq!(proposals[0].kind, ProposalKind::Monitor);
        let todo = proposals[0].todo.as_ref().unwrap();
        assert_eq!(todo.monitor_target.as_deref(), Some("explore-graph:g"));
        assert_eq!(todo.monitor_cadence.as_deref(), Some(REOBSERVE_CADENCE));
        assert_eq!(todo.action_kind, None);

        // Settled graph → no follow-up.
        let input =
            serde_json::json!({"goal_id": "g", "events": [supported_h, confirmed]}).to_string();
        let proposals = cap.propose(&input);
        assert_eq!(proposals[0].kind, ProposalKind::NoFollowUp);
        assert!(proposals[0].reason.contains("settled"));

        // Open non-hypothesis node with settled hypotheses → advance node.
        let input =
            serde_json::json!({"goal_id": "g", "events": [supported_h, confirmed, experiment]})
                .to_string();
        let proposals = cap.propose(&input);
        assert_eq!(
            proposals[0].todo.as_ref().unwrap().action_kind.as_deref(),
            Some("advance_exploration_node")
        );

        // Invalid event → gate.
        let mut forged = hypothesis.clone();
        forged["goal_id"] = Value::String("other".into());
        let input = serde_json::json!({"goal_id": "g", "events": [forged]}).to_string();
        let proposals = cap.propose(&input);
        assert_eq!(proposals[0].kind, ProposalKind::Gate);
    }

    // ── residual-branch coverage (cov100) ──────────────────────────────────

    #[test]
    fn windows_abs_path_edge_cases() {
        assert!(contains_windows_abs_path(r"C:\Users\me"));
        assert!(contains_windows_abs_path(r"C:/Users/me"));
        // colon without a separator (nth(2) is None) → not a drive path
        assert!(!contains_windows_abs_path("a:"));
        // double separator → not a drive path
        assert!(!contains_windows_abs_path("a://x"));
        // non-separator after the colon → not a drive path
        assert!(!contains_windows_abs_path("a:x"));
        assert!(!contains_windows_abs_path("plain words"));
    }

    #[test]
    fn safe_refs_reject_empty_and_path_like_values() {
        assert!(safe_public_ref("", "ref").is_err());
        assert!(safe_public_ref("/abs/path", "ref").is_err());
        assert!(safe_public_ref("~/home", "ref").is_err());
        assert!(safe_public_ref("..", "ref").is_err());
        assert!(safe_public_ref("file:///x", "ref").is_err());
        assert!(safe_result_id("", "id").is_err());
        assert!(safe_public_refs(&["".to_string()], "refs", 10).is_err());
    }

    #[test]
    fn node_event_optional_fields_and_errors() {
        let event = build_explore_node_event(
            "g",
            "title",
            Some("h_1"),
            Some("hypothesis"),
            Some("open"),
            Some("summary text"),
            Some("blocked reason"),
            Some("parent_1"),
            Some("agent-1"),
            Some("run-1"),
            &["ref_1".to_string()],
            &["tag_1".to_string()],
            Some("old_id"),
            Some("2026-08-01T00:00:00Z"),
        )
        .unwrap();
        assert_eq!(event["summary"], "summary text");
        assert_eq!(event["agent_id"], "agent-1");
        assert_eq!(event["run_id"], "run-1");
        assert_eq!(event["evidence_refs"], serde_json::json!(["ref_1"]));
        assert_eq!(event["tags"], serde_json::json!(["tag_1"]));
        assert_eq!(event["supersedes"], "old_id");
        assert_eq!(event["parent_id"], "parent_1");
        assert_eq!(event["blocked_reason"], "blocked reason");
        // empty title
        assert!(build_explore_node_event(
            "g",
            "   ",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            &[],
            None,
            None,
        )
        .is_err());
        // blocked_reason carrying private material
        assert!(build_explore_node_event(
            "g",
            "t",
            None,
            None,
            Some("open"),
            None,
            Some("api_key: x"),
            None,
            None,
            None,
            &[],
            &[],
            None,
            None,
        )
        .is_err());
    }

    #[test]
    fn edge_event_optional_fields() {
        let event = build_explore_edge_event(
            "g",
            "h_1",
            "h_2",
            "supports",
            Some("summary"),
            Some(0.8),
            Some("agent-1"),
            Some("run-1"),
            Some("2026-08-01T00:00:00Z"),
        )
        .unwrap();
        assert_eq!(event["summary"], "summary");
        assert_eq!(event["agent_id"], "agent-1");
        assert_eq!(event["run_id"], "run-1");
        assert_eq!(event["confidence"], 0.8);
    }

    #[test]
    fn finding_event_optional_fields_and_errors() {
        let event = build_explore_finding_event(
            "g",
            "finding title",
            Some("f_1"),
            Some("h_1"),
            Some(FINDING_STATUS_CONFIRMED),
            Some("summary"),
            Some(0.9),
            Some("agent-1"),
            Some("run-1"),
            &["ref_1".to_string()],
            &["tag_1".to_string()],
            Some("old_id"),
            Some("2026-08-01T00:00:00Z"),
        )
        .unwrap();
        assert_eq!(event["summary"], "summary");
        assert_eq!(event["agent_id"], "agent-1");
        assert_eq!(event["run_id"], "run-1");
        assert_eq!(event["evidence_refs"], serde_json::json!(["ref_1"]));
        assert_eq!(event["tags"], serde_json::json!(["tag_1"]));
        assert_eq!(event["supersedes"], "old_id");
        assert_eq!(event["node_id"], "h_1");
        // node_id omitted → the field is absent
        let no_node = build_explore_finding_event(
            "g",
            "t",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            &[],
            None,
            None,
        )
        .unwrap();
        assert!(no_node.get("node_id").is_none());
        // empty title
        assert!(build_explore_finding_event(
            "g",
            "   ",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            &[],
            None,
            None,
        )
        .is_err());
        // unknown status
        assert!(build_explore_finding_event(
            "g",
            "t",
            None,
            None,
            Some("ghost"),
            None,
            None,
            None,
            None,
            &[],
            &[],
            None,
            None,
        )
        .is_err());
    }

    #[test]
    fn validation_covers_edge_finding_and_goal_errors() {
        let edge = build_explore_edge_event(
            "g",
            "h_1",
            "h_2",
            "supports",
            None,
            Some(0.8),
            None,
            None,
            Some("2026-08-01T00:00:00Z"),
        )
        .unwrap();
        assert_eq!(
            validate_explore_result_event(&edge, Some("g")).unwrap(),
            edge
        );
        let finding = build_explore_finding_event(
            "g",
            "t",
            Some("f_1"),
            Some("h_1"),
            Some(FINDING_STATUS_CONFIRMED),
            None,
            None,
            None,
            None,
            &["r".to_string()],
            &["tag".to_string()],
            None,
            Some("2026-08-01T00:00:00Z"),
        )
        .unwrap();
        assert_eq!(
            validate_explore_result_event(&finding, Some("g")).unwrap(),
            finding
        );
        let n = node(
            "h_1",
            NODE_KIND_HYPOTHESIS,
            NODE_STATUS_OPEN,
            "claim",
            "2026-08-01T00:00:00Z",
            None,
        );
        // expected_goal_id None → the goal check is skipped
        assert_eq!(validate_explore_result_event(&n, None).unwrap(), n);
        // invalid goal_id in the event
        let mut bad = n.clone();
        bad["goal_id"] = Value::String("a/b".into());
        assert!(validate_explore_result_event(&bad, None).is_err());
        // a node whose result_id fails to rebuild
        let mut bad_id = n.clone();
        bad_id["result_id"] = Value::String("1bad".into());
        assert!(validate_explore_result_event(&bad_id, None).is_err());
    }

    #[test]
    fn fold_skips_missing_result_id() {
        let events = vec![serde_json::json!({"event_kind": EVENT_KIND_NODE, "title": "no id"})];
        assert!(fold_by_result_id(&events, EVENT_KIND_NODE).is_empty());
    }

    #[test]
    fn node_and_finding_views_surface_lists() {
        let mut n = node(
            "h_1",
            NODE_KIND_HYPOTHESIS,
            NODE_STATUS_OPEN,
            "claim",
            "2026-08-01T00:00:00Z",
            None,
        );
        n["evidence_refs"] = serde_json::json!(["r1"]);
        n["tags"] = serde_json::json!(["t1"]);
        let view = node_view(&n, 2);
        assert_eq!(view["evidence_refs"], serde_json::json!(["r1"]));
        assert_eq!(view["tags"], serde_json::json!(["t1"]));
        let mut f = finding(
            "f_1",
            "h_1",
            FINDING_STATUS_TENTATIVE,
            "probe",
            Some(0.5),
            "2026-08-01T00:00:00Z",
        );
        f["evidence_refs"] = serde_json::json!(["r2"]);
        f["tags"] = serde_json::json!(["t2"]);
        let fv = finding_view(&f);
        assert_eq!(fv["evidence_refs"], serde_json::json!(["r2"]));
        assert_eq!(fv["tags"], serde_json::json!(["t2"]));
    }

    #[test]
    fn mermaid_and_tree_edge_cases() {
        assert_eq!(mermaid_label(""), "untitled");
        assert_eq!(mermaid_id("h-1"), "h_1");
        assert_eq!(mermaid_id("a.b:c"), "a_b_c");
        let two = vec![
            node(
                "h_1",
                NODE_KIND_HYPOTHESIS,
                NODE_STATUS_OPEN,
                "a",
                "2026-08-01T00:00:00Z",
                None,
            ),
            node(
                "h_2",
                NODE_KIND_HYPOTHESIS,
                NODE_STATUS_OPEN,
                "b",
                "2026-08-01T00:00:00Z",
                None,
            ),
        ];
        let mermaid = build_explore_mermaid(&two, &[], 1);
        assert!(mermaid.contains("1 more nodes omitted"));
        // a tree deeper than the depth limit prunes children
        let mut h1 = node(
            "h_1",
            NODE_KIND_HYPOTHESIS,
            NODE_STATUS_OPEN,
            "claim",
            "2026-08-01T00:00:00Z",
            None,
        );
        h1["parent_id"] = Value::String("area_1".into());
        let mut area = node(
            "area_1",
            NODE_KIND_AREA,
            NODE_STATUS_OPEN,
            "area",
            "2026-08-01T00:00:00Z",
            None,
        );
        area["parent_id"] = Value::String("root_1".into());
        let root = node(
            "root_1",
            NODE_KIND_AREA,
            NODE_STATUS_OPEN,
            "root",
            "2026-08-01T00:00:00Z",
            None,
        );
        let (nodes, _, _) = views(vec![root, area, h1]);
        let parents = parent_map(&nodes, &[]);
        let tree = build_tree(&nodes, &parents, 1);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0]["node_id"], "root_1");
        assert!(tree[0]["children"].as_array().unwrap().is_empty());
    }

    #[test]
    fn validation_rebuild_errors_tag_overlap_and_unattached_finding() {
        let edge_event = build_explore_edge_event(
            "g",
            "h_1",
            "h_2",
            "supports",
            None,
            None,
            None,
            None,
            Some("2026-08-01T00:00:00Z"),
        )
        .unwrap();
        let mut bad_edge = edge_event.clone();
        bad_edge["edge_type"] = Value::String("ghost".into());
        assert!(validate_explore_result_event(&bad_edge, Some("g")).is_err());
        let finding_event = build_explore_finding_event(
            "g",
            "t",
            Some("f_1"),
            Some("h_1"),
            Some(FINDING_STATUS_CONFIRMED),
            None,
            None,
            None,
            None,
            &[],
            &[],
            None,
            Some("2026-08-01T00:00:00Z"),
        )
        .unwrap();
        let mut bad_finding = finding_event.clone();
        bad_finding["status"] = Value::String("ghost".into());
        assert!(validate_explore_result_event(&bad_finding, Some("g")).is_err());
        let unattached = build_explore_finding_event(
            "g",
            "t",
            Some("f_2"),
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            &[],
            None,
            Some("2026-08-01T00:00:00Z"),
        )
        .unwrap();
        let projection = build_explore_result_projection(
            &[unattached],
            "g",
            DEFAULT_FINDING_LIMIT,
            DEFAULT_MERMAID_NODE_LIMIT,
        )
        .unwrap();
        assert_eq!(projection["counts"]["finding_count"], 1);
        let (nodes, findings, edges) = views(vec![
            node(
                "h_1",
                NODE_KIND_HYPOTHESIS,
                NODE_STATUS_OPEN,
                "claim",
                &at(1),
                None,
            ),
            edge("h_1", "subtopic_of", "e_y", None, &at(2)),
        ]);
        let record = verify(
            nodes.iter().find(|n| n["node_id"] == "h_1").unwrap(),
            &findings,
            &edges,
        );
        assert_eq!(record["verification_state"], VERIFICATION_UNVERIFIED);
    }

    #[test]
    fn graph_view_tag_overlap_matches() {
        let mut n = node(
            "h_1",
            NODE_KIND_HYPOTHESIS,
            NODE_STATUS_OPEN,
            "claim",
            &at(1),
            None,
        );
        n["tags"] = serde_json::json!(["foo"]);
        let (nodes, _, edges) = views(vec![n]);
        let view = build_explore_graph_view(&nodes, &edges, &[], &["foo"], true, 60).unwrap();
        assert_eq!(view["graph_counts"]["matched_node_count"], 1);
    }

    #[test]
    fn graph_view_tag_filter_and_ancestor_cycle_break() {
        let events = vec![
            node(
                "area_1",
                NODE_KIND_AREA,
                NODE_STATUS_OPEN,
                "root area",
                &at(1),
                None,
            ),
            node(
                "h_1",
                NODE_KIND_HYPOTHESIS,
                NODE_STATUS_RESOLVED,
                "supported",
                &at(1),
                None,
            ),
        ];
        let (nodes, _, edges) = views(events);
        // tags-only filter (no statuses) → status block skipped
        let view = build_explore_graph_view(&nodes, &edges, &[], &["ghost"], true, 60).unwrap();
        assert_eq!(view["graph_counts"]["node_count"], 0);
        // no filter at all → every node selected
        let view = build_explore_graph_view(&nodes, &edges, &[], &[], true, 60).unwrap();
        assert_eq!(view["graph_counts"]["node_count"], 2);
        // an ancestor cycle breaks the walk instead of looping forever
        let mut a = node(
            "a",
            NODE_KIND_HYPOTHESIS,
            NODE_STATUS_OPEN,
            "A",
            &at(1),
            None,
        );
        a["parent_id"] = Value::String("b".into());
        let mut b = node(
            "b",
            NODE_KIND_HYPOTHESIS,
            NODE_STATUS_OPEN,
            "B",
            &at(1),
            None,
        );
        b["parent_id"] = Value::String("a".into());
        let (nodes, _, edges) = views(vec![a, b]);
        let view =
            build_explore_graph_view(&nodes, &edges, &[NODE_STATUS_OPEN], &[], true, 60).unwrap();
        assert!(view["graph_counts"]["node_count"].as_u64().unwrap() >= 2);
    }

    #[test]
    fn verification_skips_unrelated_and_activity_edges() {
        let (nodes, findings, edges) = views(vec![
            node(
                "h_1",
                NODE_KIND_HYPOTHESIS,
                NODE_STATUS_OPEN,
                "claim",
                &at(1),
                None,
            ),
            finding(
                "f_other",
                "h_other",
                FINDING_STATUS_CONFIRMED,
                "other",
                None,
                &at(2),
            ),
            {
                let mut f = finding(
                    "f_ghost",
                    "h_1",
                    FINDING_STATUS_TENTATIVE,
                    "ghost",
                    None,
                    &at(2),
                );
                f["status"] = Value::String("ghost-status".into());
                f
            },
            edge("e_a", "answers", "e_b", None, &at(2)),
            edge("h_1", "leads_to", "e_x", None, &at(2)),
        ]);
        let record = verify(
            nodes.iter().find(|n| n["node_id"] == "h_1").unwrap(),
            &findings,
            &edges,
        );
        // the incident `leads_to` edge counts as activity → testing
        assert_eq!(record["verification_state"], VERIFICATION_TESTING);
    }

    #[test]
    fn strip_claim_prefix_is_case_insensitive() {
        assert_eq!(
            ExploreCapability::strip_claim_prefix("Hypothesis: bigger"),
            "bigger"
        );
        assert_eq!(
            ExploreCapability::strip_claim_prefix("HYPOTHESIS: bigger"),
            "bigger"
        );
        assert_eq!(
            ExploreCapability::strip_claim_prefix("hypothesis:bigger"),
            "bigger"
        );
    }

    #[test]
    fn propose_testing_hypothesis_advances_experiment() {
        let cap = ExploreCapability;
        let testing_h = node(
            "h_1",
            NODE_KIND_HYPOTHESIS,
            NODE_STATUS_EXPLORING,
            "claim",
            &at(1),
            None,
        );
        let input = serde_json::json!({"goal_id": "g", "events": [testing_h]}).to_string();
        let proposals = cap.propose(&input);
        assert_eq!(
            proposals[0].todo.as_ref().unwrap().action_kind.as_deref(),
            Some("advance_exploration_experiment")
        );
    }
}
