//! PR Review Queue capability (LoopX: pull-request-review — the autonomous
//! PR review queue). P2-3 zero-implementation fill: the reference
//! `capabilities/pr_review_queue/` (core.py 506 lines + review_contract.py
//! 369 lines) becomes a deterministic rule version.
//!
//! Two surfaces, mirroring the reference split:
//!
//! - **queue observation** (core.py): one complete open-PR queue observation
//!   emits at most one exact-head review candidate. Fingerprints cover exact
//!   head, review decision, check state, draft state, and mergeability for
//!   every open PR; unchanged observations emit no duplicate candidate and
//!   advance only from an explicit handled exact-head cursor
//!   (`NUMBER@HEAD_OID`). Incomplete reads are `not_observed` and never
//!   count as unchanged.
//! - **review contracts** (review_contract.py): the shared execution
//!   contract (evidence requirements + completion gate + verdict policy),
//!   the per-PR review plan, the five-block review template
//!   (动机/改动思路/具体改动/对主干的风险/我的整体评价), and the
//!   agent-response contract. Host skills route the contract; they must not
//!   reimplement it.
//!
//! Propose-only, as always: candidate selection grants no GitHub review,
//! comment, push, merge, quota, or todo-write authority.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

use super::monitor_todo;
use super::successor_todo;
use super::Capability;
use super::TypedProposal;
use crate::state::Priority;

pub const OBSERVATION_SCHEMA_VERSION: &str = "pull_request_review_queue_observation_v0";
pub const CANDIDATE_SCHEMA_VERSION: &str = "pull_request_review_candidate_v0";
pub const TODO_PREVIEW_SCHEMA_VERSION: &str = "pull_request_review_todo_preview_v0";
pub const REVIEW_BACKLOG_SCHEMA_VERSION: &str = "pr_review_queue_backlog_v0";
pub const EXECUTION_CONTRACT_SCHEMA_VERSION: &str = "pull_request_review_execution_contract_v1";
pub const REVIEW_PLAN_SCHEMA_VERSION: &str = "pull_request_review_plan_v1";
pub const REVIEW_RESULT_SCHEMA_VERSION: &str = "pull_request_review_result_v1";
pub const FIVE_BLOCK_TEMPLATE_SCHEMA_VERSION: &str = "pr_review_five_block_template_v0";
pub const AGENT_RESPONSE_CONTRACT_SCHEMA_VERSION: &str = "pr_review_agent_response_contract_v0";

/// The five final sections every detailed review must render (reference
/// REQUIRED_FINAL_SECTIONS).
pub const REQUIRED_FINAL_SECTIONS: [&str; 5] = [
    "动机",
    "改动思路",
    "具体改动",
    "对主干的风险",
    "我的整体评价",
];

/// Code areas that make a change a code change (reference CODE_AREAS).
pub const CODE_AREAS: [&str; 4] = [
    "product_runtime",
    "app_or_ui_surface",
    "ci_or_release",
    "build_or_config",
];

/// Areas that additionally require a negative walkthrough (reference
/// NEGATIVE_PATH_AREAS).
pub const NEGATIVE_PATH_AREAS: [&str; 5] = [
    "product_runtime",
    "app_or_ui_surface",
    "ci_or_release",
    "build_or_config",
    "public_entry_or_policy",
];

// ── review verdict (the 通过 / 驳回 / 再修 contract surface) ────────────────

/// A published review verdict. `approve` 通过 / `request_changes` 驳回 /
/// `rework` 再修.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewVerdict {
    #[serde(rename = "approve")]
    Approve,
    #[serde(rename = "request_changes")]
    RequestChanges,
    #[serde(rename = "rework")]
    Rework,
}

impl ReviewVerdict {
    /// Parse a verdict token: canonical keys plus the operator-facing
    /// synonyms (`pass` / `reject`) and the Chinese labels.
    pub fn parse(token: &str) -> Option<Self> {
        match token.trim().to_lowercase().as_str() {
            "approve" | "pass" | "通过" => Some(Self::Approve),
            "request-changes" | "request_changes" | "reject" | "驳回" => {
                Some(Self::RequestChanges)
            }
            "rework" | "再修" => Some(Self::Rework),
            _ => None,
        }
    }

    /// Stable machine key (JSON / todo note encoding).
    pub fn key(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::RequestChanges => "request_changes",
            Self::Rework => "rework",
        }
    }

    /// Operator-facing label (通过 / 驳回 / 再修).
    pub fn label(self) -> &'static str {
        match self {
            Self::Approve => "通过",
            Self::RequestChanges => "驳回",
            Self::Rework => "再修",
        }
    }

    /// All verdicts in canonical order.
    pub const ALL: [Self; 3] = [Self::Approve, Self::RequestChanges, Self::Rework];
}

/// The shared verdict policy (reference review_execution_contract
/// verdict_policy): which published verdict a finding maps to.
pub fn published_verdict_for(pr_merged: bool, blocking_finding: bool) -> &'static str {
    if pr_merged {
        "POST_MERGE_AUDIT_COMMENT when a new actionable finding exists"
    } else if blocking_finding {
        "REQUEST_CHANGES"
    } else {
        "COMMENT"
    }
}

// ── queue observation (core.py port) ──────────────────────────────────────

/// Compact check counts (reference `_check_snapshot` counts).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckCounts {
    pub success: i64,
    pub failure: i64,
    pub pending: i64,
    pub unknown: i64,
}

/// Compact check snapshot: counts + failing/pending check names.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckSnapshot {
    pub counts: CheckCounts,
    pub failures: Vec<String>,
    pub pending: Vec<String>,
}

/// One ranked PR snapshot (internal + serialized candidate checks).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrSnapshot {
    pub number: Option<u64>,
    pub state: String,
    pub head_oid: String,
    pub review_decision: String,
    pub checks: CheckSnapshot,
    #[serde(rename = "is_draft")]
    pub is_draft: bool,
    pub merge_state: String,
    pub fingerprint: String,
    pub rank: u32,
    pub title: String,
    pub url: String,
}

/// One queue item in the observation payload (`number` + `fingerprint`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueueItemRef {
    pub number: Option<u64>,
    pub fingerprint: String,
}

/// The todo preview embedded in a candidate (reference `_candidate_packet`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TodoPreview {
    pub schema_version: String,
    pub role: String,
    pub priority: String,
    pub task_class: String,
    pub action_kind: String,
    pub task_repository: Option<String>,
    pub target_key: String,
    pub required_capabilities: Vec<String>,
    pub text: String,
}

/// At most one exact-head review candidate per observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidatePacket {
    pub schema_version: String,
    pub repository: Option<String>,
    pub number: Option<u64>,
    pub url: String,
    pub head_oid: String,
    pub review_decision: String,
    pub merge_state: String,
    pub checks: CheckSnapshot,
    pub fingerprint: Option<String>,
    pub todo_preview: TodoPreview,
}

/// Backlog projection: active/quiet cadence from unhandled actionable PRs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewBacklog {
    pub schema_version: String,
    pub actionable_unhandled_count: u32,
    pub pending_candidate_exact_head: Option<String>,
    pub recommended_poll_interval_minutes: u32,
    pub recommended_cadence: String,
}

/// The full read-only queue observation (reference OBSERVATION_STATES).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewQueueObservation {
    pub schema_version: String,
    pub repository: Option<String>,
    pub observation_state: String,
    pub reason: String,
    pub queue_fingerprint: Option<String>,
    pub previous_queue_fingerprint: Option<String>,
    pub baseline_preserved: bool,
    pub items: Vec<QueueItemRef>,
    pub queue_size: Option<usize>,
    pub changed_pr_numbers: Vec<Option<u64>>,
    pub removed_pr_numbers: Vec<u64>,
    pub candidate: Option<CandidatePacket>,
    pub candidate_count: u32,
    pub candidate_selection_reason: Option<String>,
    pub pending_candidate_exact_head: Option<String>,
    pub review_backlog: ReviewBacklog,
    pub handled_exact_heads: Vec<String>,
    pub handled_exact_head_count: usize,
    pub projected_candidate_exact_heads: Vec<String>,
    pub projected_candidate_count: usize,
    pub selection_policy: String,
    pub write_authority_granted: bool,
    pub external_write_performed: bool,
}

/// Stable fingerprint: sha256 of canonical JSON, first 16 hex chars
/// (reference `_fingerprint`).
pub fn fingerprint(value: &impl Serialize) -> String {
    use sha2::Digest;
    let encoded = serde_json::to_string(value).unwrap_or_default();
    let digest = sha2::Sha256::digest(encoded.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    hex[..16].to_string()
}

/// Uppercase a value or fall back to `default` (reference `_upper`).
fn upper(value: Option<&Value>, default: &str) -> String {
    match value {
        Some(v) => v.as_str().unwrap_or_default().trim().to_uppercase(),
        None => String::new(),
    }
    .pipe(|text| {
        if text.is_empty() {
            default.to_string()
        } else {
            text
        }
    })
}

/// Extension trait so `Option<T>` chains read naturally in ports.
trait Pipe: Sized {
    fn pipe<R>(self, f: impl FnOnce(Self) -> R) -> R {
        f(self)
    }
}
impl<T> Pipe for T {}

/// Sorted deduped trimmed string list from a JSON array (reference
/// `_string_list`).
fn string_list(value: Option<&Value>) -> Vec<String> {
    let Some(value) = value else { return vec![] };
    let Some(items) = value.as_array() else {
        return vec![];
    };
    let mut out: BTreeSet<String> = BTreeSet::new();
    for item in items {
        if let Some(text) = item.as_str() {
            let text = text.trim();
            if !text.is_empty() {
                out.insert(text.to_string());
            }
        }
    }
    out.into_iter().collect()
}

/// Compact a checks mapping (reference `_check_snapshot`).
fn check_snapshot(value: Option<&Value>) -> CheckSnapshot {
    let Some(checks) = value else {
        return CheckSnapshot::default();
    };
    let counts = checks.get("counts");
    let mut compact = CheckCounts::default();
    if let Some(counts) = counts {
        for key in ["success", "failure", "pending", "unknown"] {
            let parsed: i64 = counts.get(key).and_then(Value::as_i64).unwrap_or(0).max(0);
            match key {
                "success" => compact.success = parsed,
                "failure" => compact.failure = parsed,
                "pending" => compact.pending = parsed,
                _ => compact.unknown = parsed,
            }
        }
    }
    CheckSnapshot {
        counts: compact,
        failures: string_list(checks.get("failures")),
        pending: string_list(checks.get("pending")),
    }
}

/// Normalize a number-like JSON value to text (reference `str(number)`).
fn number_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(s)) => s.trim().to_string(),
        _ => String::new(),
    }
}

/// Build one PR snapshot + fingerprint (reference `_pr_snapshot`).
fn pr_snapshot(item: &Value) -> PrSnapshot {
    let number = number_text(item.get("number"));
    let number_parsed = number.parse::<u64>().ok();
    let snapshot = PrSnapshot {
        number: number_parsed,
        state: upper(item.get("state"), "OPEN"),
        head_oid: item
            .get("head_oid")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string(),
        review_decision: upper(item.get("review_decision"), "UNKNOWN"),
        checks: check_snapshot(item.get("checks")),
        is_draft: item.get("is_draft").and_then(Value::as_bool) == Some(true),
        merge_state: upper(item.get("merge_state"), "UNKNOWN"),
        fingerprint: String::new(),
        rank: 0,
        title: String::new(),
        url: String::new(),
    };
    PrSnapshot {
        fingerprint: fingerprint(&snapshot),
        ..snapshot
    }
}

/// `NUMBER@HEAD_OID` for a 40- or 64-hex head (reference `_exact_head_key`).
pub fn exact_head_key(number: Option<&Value>, head_oid: Option<&Value>) -> Option<String> {
    let number_txt = number_text(number);
    let head_text = number_text(head_oid).to_lowercase();
    if number_txt.is_empty() || !number_txt.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let is_hex_oid = (head_text.len() == 40 || head_text.len() == 64)
        && head_text.chars().all(|c| c.is_ascii_hexdigit());
    if !is_hex_oid {
        return None;
    }
    Some(format!("{number_txt}@{head_text}"))
}

/// Normalize + validate handled exact heads (reference
/// `_normalize_handled_exact_heads`).
pub fn normalize_handled_exact_heads(value: &[String]) -> Result<Vec<String>, String> {
    let mut normalized: BTreeSet<String> = BTreeSet::new();
    for item in value {
        let text = item.trim();
        let Some((number, head_oid)) = text.split_once('@') else {
            return Err(
                "handled exact head must use NUMBER@HEAD_OID with a 40- or 64-hex head".to_string(),
            );
        };
        let key = exact_head_key(
            Some(&Value::String(number.to_string())),
            Some(&Value::String(head_oid.to_string())),
        )
        .ok_or_else(|| {
            "handled exact head must use NUMBER@HEAD_OID with a 40- or 64-hex head".to_string()
        })?;
        normalized.insert(key);
    }
    Ok(sort_exact_heads(normalized.into_iter().collect()))
}

/// Sort exact-head keys by (number, key) — lexicographic sort would put
/// `10@…` before `2@…` (reference sorts numerically).
pub fn sort_exact_heads(mut keys: Vec<String>) -> Vec<String> {
    keys.sort_by_key(|key| {
        let number: u64 = key
            .split('@')
            .next()
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        (number, key.clone())
    });
    keys
}

/// Decide the candidate action for one ranked snapshot (reference
/// `_candidate_action`): drafts and non-OPEN PRs are never actionable.
fn candidate_action(item: &PrSnapshot) -> Option<(&'static str, &'static str)> {
    if item.is_draft || item.state != "OPEN" {
        return None;
    }
    if item.number.is_none()
        || (item.head_oid.len() != 40 && item.head_oid.len() != 64)
        || !item.head_oid.chars().all(|c| c.is_ascii_hexdigit())
    {
        return None;
    }
    match item.review_decision.as_str() {
        "CHANGES_REQUESTED" => Some(("rereview_pull_request_exact_head", "P0")),
        "APPROVED" => Some(("qualify_pull_request_merge_readiness", "P1")),
        _ => Some(("review_pull_request_exact_head", "P1")),
    }
}

/// Build the candidate packet for one ranked item (reference
/// `_candidate_packet`).
fn candidate_packet(item: &PrSnapshot, repository: &str) -> Option<CandidatePacket> {
    // `item` is already a ranked snapshot; the action re-derives from the
    // snapshot fields (draft/state gating matches `_candidate_action`).
    let (action_kind, priority) = candidate_action(item)?;
    let number = item.number?;
    let url = item.url.trim().to_string();
    let task_repository = if repository.is_empty() {
        None
    } else {
        Some(format!("git:github.com/{repository}"))
    };
    let verb = match action_kind {
        "rereview_pull_request_exact_head" => "Re-review",
        "qualify_pull_request_merge_readiness" => "Qualify merge readiness for",
        _ => "Review",
    };
    let text = format!(
        "[{priority}] {verb} PR #{number} at exact head {}; read the diff and checks, publish a review state matching the evidence, and route any merge through repository policy.",
        item.head_oid
    );
    Some(CandidatePacket {
        schema_version: CANDIDATE_SCHEMA_VERSION.to_string(),
        repository: if repository.is_empty() {
            None
        } else {
            Some(repository.to_string())
        },
        number: Some(number),
        url,
        head_oid: item.head_oid.clone(),
        review_decision: item.review_decision.clone(),
        merge_state: item.merge_state.clone(),
        checks: item.checks.clone(),
        fingerprint: Some(item.fingerprint.clone()),
        todo_preview: TodoPreview {
            schema_version: TODO_PREVIEW_SCHEMA_VERSION.to_string(),
            role: "agent".to_string(),
            priority: priority.to_string(),
            task_class: "advancement_task".to_string(),
            action_kind: action_kind.to_string(),
            task_repository,
            target_key: format!("github-pr-review:{repository}#{number}@{}", item.head_oid),
            required_capabilities: vec![
                "network".to_string(),
                "external_evidence_poll".to_string(),
            ],
            text,
        },
    })
}

/// Backlog projection (reference `_review_backlog`).
fn review_backlog(
    items: &[PrSnapshot],
    handled_set: &BTreeSet<String>,
    pending_candidate_exact_head: Option<&str>,
) -> ReviewBacklog {
    let mut actionable_unhandled_count = 0u32;
    for item in items {
        let Some(number) = item.number else { continue };
        let key = exact_head_key(
            Some(&Value::Number(number.into())),
            Some(&Value::String(item.head_oid.clone())),
        );
        let Some(key) = key else { continue };
        if handled_set.contains(&key) {
            continue;
        }
        let actionable = !item.is_draft
            && item.state == "OPEN"
            && (item.head_oid.len() == 40 || item.head_oid.len() == 64)
            && item.head_oid.chars().all(|c| c.is_ascii_hexdigit());
        if actionable {
            actionable_unhandled_count += 1;
        }
    }
    let active = actionable_unhandled_count > 0 || pending_candidate_exact_head.is_some();
    ReviewBacklog {
        schema_version: REVIEW_BACKLOG_SCHEMA_VERSION.to_string(),
        actionable_unhandled_count,
        pending_candidate_exact_head: pending_candidate_exact_head.map(|s| s.to_string()),
        recommended_poll_interval_minutes: if active { 3 } else { 15 },
        recommended_cadence: if active {
            "active_review".to_string()
        } else {
            "quiet_wait".to_string()
        },
    }
}

/// State extracted from a previous observation (reference `_previous_*`).
#[derive(Debug, Clone, Default)]
struct PreviousObservation {
    repository: String,
    previous_fingerprint: Option<String>,
    handled_exact_heads: Vec<String>,
    candidate_exact_head: Option<String>,
    projected_exact_heads: Vec<String>,
    /// number → fingerprint of the previous observation's items.
    items: BTreeMap<u64, String>,
}

/// Unwrap `{autonomous_review: …}` wrappers (reference
/// `_previous_observation`) and extract the durable cursor state.
fn extract_previous(value: Option<&Value>) -> Result<PreviousObservation, String> {
    let Some(value) = value else {
        return Ok(PreviousObservation::default());
    };
    let observation = value
        .get("autonomous_review")
        .filter(|v| v.is_object())
        .unwrap_or(value);
    let mut out = PreviousObservation {
        repository: observation
            .get("repository")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string(),
        ..PreviousObservation::default()
    };
    out.previous_fingerprint = ["queue_fingerprint", "previous_queue_fingerprint"]
        .iter()
        .find_map(|k| {
            observation
                .get(*k)
                .and_then(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });
    let handled: Vec<String> = observation
        .get("handled_exact_heads")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    out.handled_exact_heads = normalize_handled_exact_heads(&handled)?;
    // candidate exact head: the embedded candidate or the pending cursor.
    if let Some(candidate) = observation.get("candidate").filter(|v| v.is_object()) {
        if let Some(key) = exact_head_key(candidate.get("number"), candidate.get("head_oid")) {
            out.candidate_exact_head = Some(key);
        }
    }
    if out.candidate_exact_head.is_none() {
        let pending = observation
            .get("pending_candidate_exact_head")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        if pending.contains('@') {
            if let Some((number, head_oid)) = pending.split_once('@') {
                out.candidate_exact_head = exact_head_key(
                    Some(&Value::String(number.to_string())),
                    Some(&Value::String(head_oid.to_string())),
                );
            }
        }
    }
    let projected: Vec<String> = observation
        .get("projected_candidate_exact_heads")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    out.projected_exact_heads = normalize_handled_exact_heads(&projected)?;
    // items: only observations that were actually observed carry item
    // fingerprints (reference `_previous_items`).
    let state = observation
        .get("observation_state")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let baseline_preserved = observation
        .get("baseline_preserved")
        .and_then(Value::as_bool)
        == Some(true);
    let usable = matches!(state, "observed_unchanged" | "material_transition")
        || (state == "not_observed" && baseline_preserved);
    if usable {
        if let Some(items) = observation.get("items").and_then(Value::as_array) {
            for item in items {
                let number = item
                    .get("number")
                    .and_then(|n| n.as_i64())
                    .and_then(|n| u64::try_from(n).ok());
                if let Some(number) = number {
                    let fp = item
                        .get("fingerprint")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    out.items.insert(number, fp);
                }
            }
        }
    }
    Ok(out)
}

/// Build one read-only queue observation and at most one exact-head
/// candidate (reference `build_pull_request_review_queue_observation`).
///
/// `pull_requests` items are tolerant JSON objects (`number`, `title`,
/// `url`, `state`, `head_oid`, `review_decision`, `is_draft`,
/// `merge_state`, `checks`); `result_completeness` must be
/// `{"complete": true}` for the queue to count as observed.
#[allow(clippy::too_many_lines)]
pub fn build_pull_request_review_queue_observation(
    repository: Option<&str>,
    pull_requests: &[Value],
    result_completeness: &Value,
    previous_observation: Option<&Value>,
    handled_exact_heads: &[String],
) -> Result<ReviewQueueObservation, String> {
    let normalized_repository = repository.unwrap_or_default().trim().to_string();
    let previous = extract_previous(previous_observation)?;
    // Cursors are repository-scoped (reference `_previous_handled_exact_heads`
    // etc.): a previous observation of a different repository contributes no
    // handled, candidate, or projected state.
    let same_repository = previous.repository == normalized_repository;
    let previous_handled = if same_repository {
        previous.handled_exact_heads.clone()
    } else {
        Vec::new()
    };
    let previous_candidate = if same_repository {
        previous.candidate_exact_head.clone()
    } else {
        None
    };
    let previous_projected = if same_repository {
        previous.projected_exact_heads.clone()
    } else {
        Vec::new()
    };
    let supplied_handled = normalize_handled_exact_heads(handled_exact_heads)?;

    // handled cursors must match a prior candidate or a persisted cursor
    // (reference: unexpected handled heads are rejected).
    let mut allowed_supplied: BTreeSet<String> = previous_handled.iter().cloned().collect();
    let previous_projected: BTreeSet<String> = previous_projected.iter().cloned().collect();
    let mut projected_set: BTreeSet<String> = previous_projected.clone();
    for key in &previous_projected {
        allowed_supplied.insert(key.clone());
    }
    if let Some(candidate) = &previous_candidate {
        projected_set.insert(candidate.clone());
        allowed_supplied.insert(candidate.clone());
    }
    let unexpected: Vec<&String> = supplied_handled
        .iter()
        .filter(|key| !allowed_supplied.contains(*key))
        .collect();
    if !unexpected.is_empty() {
        return Err(format!(
            "handled exact head must match the prior candidate or a persisted handled cursor: {}",
            unexpected
                .iter()
                .map(|k| k.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let mut handled: BTreeSet<String> = previous_handled.iter().cloned().collect();
    for key in &supplied_handled {
        handled.insert(key.clone());
    }
    projected_set.retain(|key| !handled.contains(key));

    let previous_fingerprint = previous.previous_fingerprint.clone();
    let previous_items = previous.items;

    let selection_policy = "rotate through unprojected unhandled PRs in the existing pr-review sequence; exact head required".to_string();

    // Incomplete reads are never observed (reference: complete gate). The
    // backlog still counts actionable unhandled items over the supplied
    // payload (reference `_review_backlog(pull_requests, …)`).
    if result_completeness.get("complete").and_then(Value::as_bool) != Some(true) {
        let pending_candidate_exact_head = previous_candidate.filter(|key| !handled.contains(key));
        let supplied_snapshots: Vec<PrSnapshot> = pull_requests.iter().map(pr_snapshot).collect();
        let backlog = review_backlog(
            &supplied_snapshots,
            &handled,
            pending_candidate_exact_head.as_deref(),
        );
        return Ok(ReviewQueueObservation {
            schema_version: OBSERVATION_SCHEMA_VERSION.to_string(),
            repository: if normalized_repository.is_empty() {
                None
            } else {
                Some(normalized_repository.clone())
            },
            observation_state: "not_observed".to_string(),
            reason: "complete_open_queue_required".to_string(),
            queue_fingerprint: None,
            previous_queue_fingerprint: previous_fingerprint.clone(),
            baseline_preserved: previous_fingerprint.is_some(),
            items: previous_items
                .iter()
                .map(|(number, fingerprint)| QueueItemRef {
                    number: Some(*number),
                    fingerprint: fingerprint.clone(),
                })
                .collect(),
            queue_size: None,
            changed_pr_numbers: vec![],
            removed_pr_numbers: vec![],
            candidate: None,
            candidate_count: 0,
            candidate_selection_reason: None,
            pending_candidate_exact_head,
            review_backlog: backlog,
            handled_exact_heads: sort_exact_heads(handled.iter().cloned().collect()),
            handled_exact_head_count: handled.len(),
            projected_candidate_exact_heads: sort_exact_heads(
                projected_set.iter().cloned().collect(),
            ),
            projected_candidate_count: projected_set.len(),
            selection_policy,
            write_authority_granted: false,
            external_write_performed: false,
        });
    }

    // Rank open PRs in input order; the exact-head key requires a full OID.
    let mut ranked_items: Vec<PrSnapshot> = Vec::new();
    for (rank, item) in pull_requests.iter().enumerate() {
        if upper(item.get("state"), "OPEN") != "OPEN" {
            continue;
        }
        let mut snapshot = pr_snapshot(item);
        snapshot.rank = u32::try_from(rank + 1).unwrap_or(0);
        snapshot.title = item
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        snapshot.url = item
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        ranked_items.push(snapshot);
    }

    let current_exact_heads: BTreeSet<String> = ranked_items
        .iter()
        .filter_map(|item| {
            exact_head_key(
                item.number.map(|n| Value::Number(n.into())).as_ref(),
                Some(&Value::String(item.head_oid.clone())),
            )
        })
        .collect();
    let actionable_exact_heads: BTreeSet<String> = ranked_items
        .iter()
        .filter_map(|item| {
            let key = exact_head_key(
                item.number.map(|n| Value::Number(n.into())).as_ref(),
                Some(&Value::String(item.head_oid.clone())),
            );
            if candidate_action(item).is_some() {
                key
            } else {
                None
            }
        })
        .collect();

    // Prune handled/projected cursors to the live queue.
    handled.retain(|key| current_exact_heads.contains(key));
    let handled_set = handled.clone();
    projected_set.retain(|key| current_exact_heads.contains(key) && !handled_set.contains(key));

    let mut queue_items: Vec<QueueItemRef> = ranked_items
        .iter()
        .map(|item| QueueItemRef {
            number: item.number,
            fingerprint: item.fingerprint.clone(),
        })
        .collect();
    queue_items.sort_by_key(|item| item.number.unwrap_or(0));
    let queue_fingerprint = fingerprint(&serde_json::json!({
        "repository": normalized_repository,
        "items": queue_items,
    }));

    // Changed detection needs the prior fingerprints of the SAME repository.
    let previous_repository = previous.repository.clone();
    let prior_items: BTreeMap<u64, String> = if previous_repository == normalized_repository {
        previous_items.clone()
    } else {
        BTreeMap::new()
    };
    let changed: Vec<&PrSnapshot> = ranked_items
        .iter()
        .filter(|item| {
            let prior = item
                .number
                .and_then(|n| prior_items.get(&n))
                .map(|s| s.as_str())
                .unwrap_or_default();
            prior != item.fingerprint
        })
        .collect();
    let removed_numbers: Vec<u64> = {
        let current_numbers: BTreeSet<u64> =
            ranked_items.iter().filter_map(|item| item.number).collect();
        prior_items
            .keys()
            .filter(|number| !current_numbers.contains(number))
            .copied()
            .collect()
    };
    let unchanged = previous_fingerprint.as_deref() == Some(queue_fingerprint.as_str());
    let observation_state = if unchanged {
        "observed_unchanged"
    } else {
        "material_transition"
    };

    // At most one candidate: unhandled material transitions first, then the
    // unhandled, unprojected backlog rotation.
    let mut candidate: Option<CandidatePacket> = None;
    let mut candidate_selection_reason = None;
    if observation_state == "material_transition" {
        for item in &changed {
            if let Some(number) = item.number {
                let key = exact_head_key(
                    Some(&Value::Number(number.into())),
                    Some(&Value::String(item.head_oid.clone())),
                );
                if key.is_none_or(|k| handled_set.contains(&k)) {
                    continue;
                }
            }
            if let Some(packet) = candidate_packet(item, &normalized_repository) {
                candidate = Some(packet);
                candidate_selection_reason = Some("unhandled_material_transition".to_string());
                break;
            }
        }
    }
    if candidate.is_none() {
        for item in &ranked_items {
            let Some(number) = item.number else { continue };
            let key = exact_head_key(
                Some(&Value::Number(number.into())),
                Some(&Value::String(item.head_oid.clone())),
            );
            let Some(key) = key else { continue };
            if handled_set.contains(&key) || projected_set.contains(&key) {
                continue;
            }
            if let Some(packet) = candidate_packet(item, &normalized_repository) {
                candidate = Some(packet);
                candidate_selection_reason = Some("unhandled_backlog_progression".to_string());
                break;
            }
        }
    }
    let candidate_exact_head = candidate.as_ref().and_then(|packet| {
        exact_head_key(
            packet.number.map(|n| Value::Number(n.into())).as_ref(),
            Some(&Value::String(packet.head_oid.clone())),
        )
    });
    let mut pending_candidate_exact_head = candidate_exact_head.clone();
    if pending_candidate_exact_head.is_none() {
        if let Some(previous_candidate) = &previous_candidate {
            if actionable_exact_heads.contains(previous_candidate)
                && !handled_set.contains(previous_candidate)
            {
                pending_candidate_exact_head = Some(previous_candidate.clone());
            }
        }
    }
    if let Some(key) = &candidate_exact_head {
        projected_set.insert(key.clone());
    }

    let backlog = review_backlog(
        &ranked_items,
        &handled_set,
        pending_candidate_exact_head.as_deref(),
    );
    let reason = if unchanged {
        "queue_fingerprint_unchanged"
    } else if previous_fingerprint.is_none() {
        "initial_complete_observation"
    } else {
        "review_material_fingerprint_changed"
    };

    Ok(ReviewQueueObservation {
        schema_version: OBSERVATION_SCHEMA_VERSION.to_string(),
        repository: if normalized_repository.is_empty() {
            None
        } else {
            Some(normalized_repository.clone())
        },
        observation_state: observation_state.to_string(),
        reason: reason.to_string(),
        queue_fingerprint: Some(queue_fingerprint),
        previous_queue_fingerprint: previous_fingerprint,
        baseline_preserved: true,
        items: queue_items,
        queue_size: Some(ranked_items.len()),
        changed_pr_numbers: changed.iter().map(|item| item.number).collect(),
        removed_pr_numbers: removed_numbers,
        candidate,
        candidate_count: if candidate_exact_head.is_some() { 1 } else { 0 },
        candidate_selection_reason,
        pending_candidate_exact_head,
        review_backlog: backlog,
        handled_exact_heads: sort_exact_heads(handled_set.iter().cloned().collect()),
        handled_exact_head_count: handled_set.len(),
        projected_candidate_exact_heads: sort_exact_heads(projected_set.iter().cloned().collect()),
        projected_candidate_count: projected_set.len(),
        selection_policy,
        write_authority_granted: false,
        external_write_performed: false,
    })
}

// ── review contracts (review_contract.py port) ────────────────────────────

/// The shared review execution contract: the evidence that must exist before
/// a detailed review verdict; host skills route this contract but must not
/// reimplement it.
pub fn build_review_execution_contract() -> Value {
    serde_json::json!({
        "schema_version": EXECUTION_CONTRACT_SCHEMA_VERSION,
        "purpose": "Define the evidence that must exist before a detailed review verdict; host skills route this contract but must not reimplement it.",
        "evidence_status_values": ["verified", "unverified", "not_applicable"],
        "evidence_requirements": [
            {
                "evidence_id": "problem_context",
                "required_when": "always",
                "fields": [
                    "author_claim", "old_behavior", "affected_caller_or_operator",
                    "compounding_cost", "before_after_scenario", "smaller_fix_analysis",
                    "observable_outcome", "non_goals"
                ]
            },
            {
                "evidence_id": "architecture_flow",
                "required_when": "always",
                "fields": [
                    "entry_point", "authoritative_input_or_state", "decision_owner",
                    "side_effect_or_transition", "receipt_or_consumer",
                    "failure_or_retry_owner"
                ]
            },
            {
                "evidence_id": "changed_line_classification",
                "required_when": "always",
                "categories": ["production", "tests_or_fixtures", "docs", "generated", "mechanical_moves"],
                "fields": ["files", "additions", "deletions", "behavior_role"],
                "rule": "Classify the exact base..head diff before judging code volume."
            },
            {
                "evidence_id": "symbol_map",
                "required_when": "code_change",
                "item_count": {"minimum": 2, "maximum": 5},
                "item_fields": [
                    "path", "line", "symbol", "responsibility", "before_after_behavior",
                    "input_or_pre_state", "critical_branch_or_invariant",
                    "callee_or_side_effect", "return_or_consumer",
                    "failure_fallback_or_retry_owner", "caller_evidence"
                ],
                "source_rule": "Use exact-head definitions plus unchanged surrounding callers; select symbols by behavior, not diff size."
            },
            {
                "evidence_id": "walkthroughs",
                "required_when": "always",
                "positive_fields": ["trigger", "ordered_symbols_or_steps", "state_transitions", "observable_result"],
                "negative_required_when": "runtime, selector, policy, authority, lifecycle, installer, bridge, or public-entry contract changes",
                "negative_fields": [
                    "triggering_input_or_state", "ordered_symbols_or_steps",
                    "rejection_fallback_or_wrong_outcome", "error_or_retry_owner"
                ]
            },
            {
                "evidence_id": "validation_matrix",
                "required_when": "always",
                "item_fields": ["invariant_or_case", "command_or_check", "status", "result", "required", "skip_or_failure_reason"],
                "required_cases": [
                    "changed invariant positive case",
                    "material negative or failure case when applicable",
                    "repository-required checks"
                ]
            },
            {
                "evidence_id": "failure_analysis",
                "required_when": "always",
                "fields": [
                    "strongest_regression_scenario", "triggering_state",
                    "permitting_or_preventing_code_path", "blast_radius", "observability",
                    "rollback_or_recovery", "minimum_repair", "regression_test"
                ]
            },
            {
                "evidence_id": "code_volume",
                "required_when": "always",
                "verdict_values": ["necessary", "partly_avoidable", "not_yet_proven"],
                "fields": [
                    "changed_line_shape", "largest_production_hotspots",
                    "active_call_site_evidence", "compatibility_or_migration_need",
                    "verdict", "highest_value_simplification",
                    "behavior_preserving_validation"
                ]
            }
        ],
        "finding_contract": {
            "findings_first": true,
            "required_fields": [
                "severity", "trigger", "code_path", "incorrect_or_risky_outcome",
                "location", "minimum_repair", "regression_test"
            ],
            "no_finding_rule": "Say no blocking finding explicitly, then name residual risk and the strongest missing validation."
        },
        "completion_gate": {
            "metadata_only_verdict_allowed": false,
            "required_status": "Every applicable evidence item is verified or explicitly unverified with a reason.",
            "code_change_symbol_minimum": 2,
            "exact_head_recheck_required": true,
            "stale_head_verdict_allowed": false,
            "required_final_sections": REQUIRED_FINAL_SECTIONS
        },
        "verdict_policy": {
            "open_pr_blocking_finding": "REQUEST_CHANGES",
            "open_pr_non_blocking_finding": "COMMENT",
            "open_pr_no_finding": "COMMENT unless approval was explicitly requested through merge policy",
            "merged_pr": "POST_MERGE_AUDIT_COMMENT when a new actionable finding exists",
            "publication_exceptions": ["explicit local-only request", "private or security-sensitive finding"]
        }
    })
}

/// The per-PR review plan: which evidence ids are required, given the
/// change areas (reference `build_review_plan`).
pub fn build_review_plan(item: &Value) -> Value {
    let areas: BTreeSet<String> = item
        .get("areas")
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter(|(_, count)| {
                    !count.is_null()
                        && count != &&Value::Bool(false)
                        && count != &&Value::Number(0.into())
                })
                .map(|(area, _)| area.clone())
                .collect()
        })
        .unwrap_or_default();
    let code_change = CODE_AREAS.iter().any(|area| areas.contains(*area));
    let docs_only = !areas.is_empty()
        && areas
            .iter()
            .all(|area| area == "public_docs" || area == "public_entry_or_policy");
    let mut required_evidence = vec![
        "problem_context",
        "architecture_flow",
        "changed_line_classification",
        "walkthroughs",
        "validation_matrix",
        "failure_analysis",
        "code_volume",
    ];
    if code_change {
        required_evidence.insert(3, "symbol_map");
    }
    let number = number_text(item.get("number"));
    let head_oid = item
        .get("head_oid")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let target_key = if !number.is_empty() && !head_oid.is_empty() {
        Some(format!("{number}@{head_oid}"))
    } else {
        None
    };
    serde_json::json!({
        "schema_version": REVIEW_PLAN_SCHEMA_VERSION,
        "contract_ref": "agent_response_contract.review_execution_contract",
        "target": {
            "number": number.parse::<u64>().ok(),
            "base_ref": item.get("base_ref").and_then(Value::as_str).unwrap_or_default(),
            "head_oid": if head_oid.is_empty() { Value::Null } else { Value::String(head_oid) },
            "exact_head_key": target_key
        },
        "applicability": {
            "areas": areas.iter().cloned().collect::<Vec<_>>(),
            "code_change": code_change,
            "docs_only": docs_only,
            "symbol_map_required": code_change,
            "negative_walkthrough_required": NEGATIVE_PATH_AREAS.iter().any(|area| areas.contains(*area))
        },
        "required_evidence_ids": required_evidence,
        "result_template": {
            "schema_version": REVIEW_RESULT_SCHEMA_VERSION,
            "target_exact_head": target_key,
            "evidence": required_evidence.iter().map(|id| (id.to_string(), serde_json::json!({"status": "unverified"}))).collect::<serde_json::Map<_, _>>(),
            "findings": [],
            "residual_risk": "",
            "verdict": "unverified"
        }
    })
}

/// The five-block review template: an empty scaffold the reviewer fills from
/// verified review evidence, not PR metadata (reference `build_review_template`).
pub fn build_review_template(item: &Value) -> Value {
    let key_files: Vec<&Value> = item
        .get("key_files")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter(|candidate| candidate.is_object())
                .collect()
        })
        .unwrap_or_default();
    let mut ranked: Vec<(&Value, i64)> = key_files
        .iter()
        .map(|candidate| {
            let churn = ["additions", "deletions"]
                .iter()
                .filter_map(|field| candidate.get(*field).and_then(Value::as_i64))
                .sum::<i64>();
            (*candidate, churn)
        })
        .collect();
    ranked.sort_by_key(|(_, churn)| std::cmp::Reverse(*churn));
    let review_order: Vec<String> = ranked
        .iter()
        .take(5)
        .filter_map(|(candidate, _)| {
            candidate
                .get("path")
                .and_then(Value::as_str)
                .map(|path| path.to_string())
        })
        .collect();

    fn section(label: &str, word_hint: &str, instruction: &str) -> Value {
        serde_json::json!({
            "label": label,
            "word_hint": word_hint,
            "content": "",
            "agent_instruction": instruction
        })
    }

    serde_json::json!({
        "schema_version": FIVE_BLOCK_TEMPLATE_SCHEMA_VERSION,
        "purpose": "Empty scaffold only; fill it from the verified review evidence, not PR metadata.",
        "sections": [
            section(
                "动机",
                "200-350字",
                "Use evidence `problem_context`: old behavior, affected caller, concrete cost, before/after outcome, and why the nearest smaller fix is or is not enough."
            ),
            section(
                "改动思路",
                "300-500字",
                "Use `architecture_flow` and `walkthroughs`: entry point, authoritative state, decision boundary, positive path, alternative, and ownership trade-off."
            ),
            section(
                "具体改动",
                "450-800字",
                "Use `changed_line_classification` and `symbol_map`. Code changes require `### 关键代码讲解` for 2-5 behavior-bearing exact-head symbols; docs-only changes use `### 关键内容讲解`."
            ),
            section(
                "对主干的风险",
                "250-500字",
                "Use `failure_analysis`, `walkthroughs.negative`, and `validation_matrix`; trace each finding from triggering state to observed outcome and minimum repair."
            ),
            section(
                "我的整体评价",
                "150-300字",
                "Use `code_volume`, validation results, residual risk, and exact-head freshness to state the verdict and the evidence needed for re-review."
            )
        ],
        "review_order": review_order,
        "output_hint": "Render the verified structured result using the five sections. The capability-owned review_execution_contract is the evidence and completeness authority."
    })
}

/// The agent-response contract (reference `build_agent_response_contract`).
pub fn build_agent_response_contract() -> Value {
    serde_json::json!({
        "schema_version": AGENT_RESPONSE_CONTRACT_SCHEMA_VERSION,
        "table_only_response_allowed": false,
        "slash_prefix_dominates_intent": true,
        "stats_only_requires_explicit_opt_out": true,
        "queue_table_role": "preface_only",
        "default_review_scope": "Review PRs in review_groups.unmerged first, then review_groups.merged, bounded by the requested limit.",
        "required_packet_fields_to_preserve": [
            "agent_response_contract",
            "agent_response_contract.review_execution_contract",
            "result_completeness",
            "review_groups",
            "pull_requests[].review_plan",
            "pull_requests[].review_template",
            "pull_requests[].evidence_commands"
        ],
        "stats_only_opt_out_examples": ["只统计", "只列出", "stats only", "list only", "不要 review", "不用分析"],
        "required_final_sections": REQUIRED_FINAL_SECTIONS,
        "review_execution_contract": build_review_execution_contract(),
        "explanation_depth_contract": {
            "schema_version": "pr_review_explanation_depth_v0",
            "authority": "agent_response_contract.review_execution_contract",
            "reader_profile": "A technically curious reader who may not know this PR or subsystem.",
            "verdict_preface": "Lead with one evidence-based verdict and the highest-severity reason.",
            "freshness": "Record and recheck the remote head SHA; do not publish a stale verdict."
        },
        "instructions": [
            "Use review_groups as the queue and require result_completeness.complete=true for exhaustive review.",
            "Execute each pull_requests[].review_plan against the shared review_execution_contract before drafting prose.",
            "Do not infer verified evidence from title, labels, changed-file counts, metadata_risk_hint, or green CI alone.",
            "Recheck the exact remote head before verdict and publication.",
            "Render the verified result through pull_requests[].review_template; host skills must not maintain a competing depth checklist."
        ]
    })
}

// ── capability ────────────────────────────────────────────────────────────

pub struct PrReviewQueueCapability;

impl PrReviewQueueCapability {
    /// Extract a queue payload from the proposal input, when present:
    /// `{"repository": …, "pull_requests": […], "result_completeness": …}`.
    fn queue_payload(input: &str) -> Option<Value> {
        let value: Value = serde_json::from_str(input.trim()).ok()?;
        let object = value.as_object()?;
        object.get("pull_requests").and_then(Value::as_array)?;
        Some(value)
    }
}

impl Capability for PrReviewQueueCapability {
    fn name(&self) -> &'static str {
        "pr_review_queue"
    }

    fn describe(&self) -> &'static str {
        "observe one complete open-PR queue and emit at most one exact-head review candidate; verdicts route through the shared review contract"
    }

    fn propose(&self, input: &str) -> Vec<TypedProposal> {
        let text = input.trim();
        if text.is_empty() {
            return vec![TypedProposal::no_followup(
                "empty PR review queue observation",
            )];
        }
        let Some(payload) = Self::queue_payload(text) else {
            return vec![TypedProposal::gate(
                "Provide a complete PR queue payload (JSON with pull_requests + result_completeness.complete=true) before any review candidate is proposed.",
                "input is not a PR queue payload",
            )];
        };
        let repository = payload.get("repository").and_then(Value::as_str);
        let pull_requests = payload
            .get("pull_requests")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let completeness = payload
            .get("result_completeness")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({"complete": true}));
        let previous = payload
            .get("previous_observation")
            .or_else(|| payload.get("autonomous_review"));
        let handled: Vec<String> = payload
            .get("handled_exact_heads")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let observation = match build_pull_request_review_queue_observation(
            repository,
            &pull_requests,
            &completeness,
            previous,
            &handled,
        ) {
            Ok(observation) => observation,
            Err(err) => {
                return vec![TypedProposal::gate(
                    &format!("Fix the queue payload before re-observing: {err}"),
                    "queue payload rejected by the observation contract",
                )];
            }
        };

        if observation.observation_state == "not_observed" {
            if observation.baseline_preserved {
                let due_secs =
                    u64::from(observation.review_backlog.recommended_poll_interval_minutes) * 60;
                let repo = observation.repository.as_deref().unwrap_or("(repository)");
                let mut todo = monitor_todo(
                    "pr-review",
                    &format!(
                        "Re-poll the {repo} open-PR queue with a complete observation; the previous baseline fingerprint is preserved until a complete read succeeds."
                    ),
                    due_secs,
                );
                todo.monitor_target = Some(format!("git:github.com/{repo}"));
                todo.monitor_policy =
                    Some("read_only_observation_then_no_spend_if_unchanged".to_string());
                todo.monitor_cadence = Some(format!(
                    "{}m",
                    observation.review_backlog.recommended_poll_interval_minutes
                ));
                return vec![TypedProposal::monitor(
                    todo,
                    "queue read incomplete — baseline preserved, re-poll at the backlog cadence",
                )];
            }
            return vec![TypedProposal::successor(
                successor_todo(
                    "pr-review",
                    "Collect a complete open-PR queue observation (all open PRs with exact heads, review decisions, checks, draft and merge state) before any review candidate.",
                ),
                "queue read incomplete and no baseline exists yet — observe completely first",
            )];
        }

        if let Some(candidate) = observation.candidate {
            let priority = if candidate.todo_preview.priority == "P0" {
                Priority::P0
            } else {
                Priority::P1
            };
            let mut todo = successor_todo("pr-review", &candidate.todo_preview.text);
            todo.priority = priority;
            todo.action_kind = Some(candidate.todo_preview.action_kind.clone());
            todo.task_repository = candidate.todo_preview.task_repository.clone();
            todo.required_capability = Some("pr_review_queue".to_string());
            todo.capability_binding_ref = Some("pr_review_queue".to_string());
            return vec![TypedProposal::successor(
                todo,
                &format!(
                    "one exact-head review candidate selected ({})",
                    observation
                        .candidate_selection_reason
                        .as_deref()
                        .unwrap_or("candidate")
                ),
            )];
        }

        if observation.review_backlog.actionable_unhandled_count > 0
            || observation.pending_candidate_exact_head.is_some()
        {
            let due_secs =
                u64::from(observation.review_backlog.recommended_poll_interval_minutes) * 60;
            let repo = observation.repository.as_deref().unwrap_or("(repository)");
            let mut todo = monitor_todo(
                "pr-review",
                &format!(
                    "Re-observe the {repo} open-PR queue; the current projected candidates are handled — advance only from an explicit handled exact-head cursor."
                ),
                due_secs,
            );
            todo.monitor_target = Some(format!("git:github.com/{repo}"));
            todo.monitor_policy =
                Some("read_only_observation_then_no_spend_if_unchanged".to_string());
            todo.monitor_cadence = Some(format!(
                "{}m",
                observation.review_backlog.recommended_poll_interval_minutes
            ));
            return vec![TypedProposal::monitor(
                todo,
                "backlog still active but no unhandled candidate — periodic re-observation",
            )];
        }

        vec![TypedProposal::no_followup(
            "queue quiet — all actionable exact heads are handled and no candidate is projected",
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::ProposalKind;

    fn pr(
        number: u64,
        head: Option<&str>,
        decision: &str,
        draft: bool,
        merge_state: &str,
        failures: &[&str],
    ) -> Value {
        serde_json::json!({
            "number": number,
            "title": format!("PR {number}"),
            "url": format!("https://github.com/owner/repo/pull/{number}"),
            "state": "OPEN",
            "head_oid": head.unwrap_or(&format!("{number:040}")).to_string(),
            "review_decision": decision,
            "is_draft": draft,
            "merge_state": merge_state,
            "checks": {
                "counts": {"success": 1, "failure": failures.len(), "pending": 0, "unknown": 0},
                "failures": failures,
                "pending": []
            }
        })
    }

    fn observe(
        items: &[Value],
        complete: bool,
        previous: Option<&Value>,
        handled: &[&str],
    ) -> Result<ReviewQueueObservation, String> {
        let handled: Vec<String> = handled.iter().map(|s| s.to_string()).collect();
        build_pull_request_review_queue_observation(
            Some("owner/repo"),
            items,
            &serde_json::json!({"complete": complete}),
            previous,
            &handled,
        )
    }

    /// Serialize a previous observation into the JSON value the raw builder
    /// expects.
    fn prev(observation: &ReviewQueueObservation) -> Value {
        serde_json::to_value(observation).expect("serialize previous observation")
    }

    #[test]
    fn exact_head_key_requires_full_oid() {
        let key = exact_head_key(
            Some(&Value::Number(7.into())),
            Some(&Value::String("a".repeat(40))),
        );
        assert_eq!(
            key.as_deref(),
            Some(format!("7@{}", "a".repeat(40)).as_str())
        );
        assert!(exact_head_key(
            Some(&Value::Number(7.into())),
            Some(&Value::String("short".to_string())),
        )
        .is_none());
        assert!(exact_head_key(None, Some(&Value::String("a".repeat(40)))).is_none());
    }

    #[test]
    fn handled_exact_heads_normalize_sort_and_validate() {
        let heads = normalize_handled_exact_heads(&[
            "10@".to_string() + &"a".repeat(40),
            "2@".to_string() + &"b".repeat(40),
        ])
        .unwrap();
        assert_eq!(heads.len(), 2);
        // numeric order, not lexicographic
        assert!(heads[0].starts_with("2@"));
        assert!(heads[1].starts_with("10@"));
        assert!(normalize_handled_exact_heads(&["1@short".to_string()]).is_err());
        assert!(normalize_handled_exact_heads(&["nope".to_string()]).is_err());
    }

    #[test]
    fn incomplete_queue_is_not_observed_and_preserves_baseline() {
        let baseline = observe(
            &[pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[])],
            true,
            None,
            &[],
        )
        .unwrap();
        let result = observe(
            &[
                pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
                pr(2, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
            ],
            false,
            Some(&serde_json::json!({"autonomous_review": baseline})),
            &[],
        )
        .unwrap();
        assert_eq!(result.observation_state, "not_observed");
        assert!(result.queue_fingerprint.is_none());
        assert_eq!(
            result.previous_queue_fingerprint.as_deref(),
            baseline.queue_fingerprint.as_deref()
        );
        assert!(result.baseline_preserved);
        assert_eq!(result.items.len(), 1);
        assert!(result.queue_size.is_none());
        assert_eq!(result.candidate_count, 0);
        assert!(result.candidate.is_none());
        assert_eq!(
            result.pending_candidate_exact_head.as_deref(),
            Some(format!("1@{:040}", 1).as_str())
        );

        // recovery: a complete read of the same queue is observed_unchanged.
        let recovered = observe(
            &[pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[])],
            true,
            Some(&prev(&result)),
            &[],
        )
        .unwrap();
        assert_eq!(recovered.observation_state, "observed_unchanged");
        assert!(recovered.candidate.is_none());

        // advancing with an explicit handled cursor picks PR 2.
        let advanced = observe(
            &[
                pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
                pr(2, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
            ],
            true,
            Some(&prev(&result)),
            &[&format!("1@{:040}", 1)],
        )
        .unwrap();
        assert_eq!(advanced.candidate.as_ref().unwrap().number, Some(2));
    }

    #[test]
    fn initial_complete_observation_selects_one_exact_head_candidate() {
        let result = observe(
            &[
                pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
                pr(2, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
            ],
            true,
            None,
            &[],
        )
        .unwrap();
        assert_eq!(result.observation_state, "material_transition");
        assert_eq!(result.changed_pr_numbers, vec![Some(1), Some(2)]);
        assert_eq!(result.candidate_count, 1);
        assert_eq!(result.repository.as_deref(), Some("owner/repo"));
        assert_eq!(result.queue_size, Some(2));
        assert!(result.items.iter().all(|item| item.number.is_some()));
        let candidate = result.candidate.as_ref().unwrap();
        assert_eq!(candidate.number, Some(1));
        assert_eq!(candidate.head_oid, format!("{:040}", 1));
        let todo = &candidate.todo_preview;
        assert_eq!(todo.action_kind, "review_pull_request_exact_head");
        assert_eq!(
            todo.task_repository.as_deref(),
            Some("git:github.com/owner/repo")
        );
        assert_eq!(
            todo.required_capabilities,
            vec!["network".to_string(), "external_evidence_poll".to_string()]
        );
        assert!(!result.write_authority_granted);
        assert!(!result.external_write_performed);
    }

    #[test]
    fn unchanged_observation_emits_no_duplicate_candidate() {
        let first = observe(
            &[
                pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
                pr(2, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
            ],
            true,
            None,
            &[],
        )
        .unwrap();
        let repeated = observe(
            &[
                pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
                pr(2, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
            ],
            true,
            Some(&prev(&first)),
            &[],
        )
        .unwrap();
        assert_eq!(repeated.observation_state, "observed_unchanged");
        assert!(repeated.changed_pr_numbers.is_empty());
        assert_eq!(repeated.candidate.as_ref().unwrap().number, Some(2));
        assert_eq!(
            repeated.pending_candidate_exact_head.as_deref(),
            Some(format!("2@{:040}", 2).as_str())
        );
    }

    #[test]
    fn round_robin_rotates_through_projected_candidates() {
        let items = [
            pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
            pr(2, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
            pr(3, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
        ];
        let first = observe(&items, true, None, &[]).unwrap();
        assert_eq!(first.candidate.as_ref().unwrap().number, Some(1));
        assert_eq!(
            first.projected_candidate_exact_heads,
            vec![format!("1@{:040}", 1)]
        );
        let second = observe(&items, true, Some(&prev(&first)), &[]).unwrap();
        assert_eq!(second.candidate.as_ref().unwrap().number, Some(2));
        assert_eq!(
            second.projected_candidate_exact_heads,
            vec![format!("1@{:040}", 1), format!("2@{:040}", 2)]
        );
        let third = observe(&items, true, Some(&prev(&second)), &[]).unwrap();
        assert_eq!(third.candidate.as_ref().unwrap().number, Some(3));
        let exhausted = observe(&items, true, Some(&prev(&third)), &[]).unwrap();
        assert!(exhausted.candidate.is_none());
        assert_eq!(
            exhausted.pending_candidate_exact_head.as_deref(),
            Some(format!("3@{:040}", 3).as_str())
        );
        assert_eq!(exhausted.projected_candidate_count, 3);
    }

    #[test]
    fn handled_exact_head_advances_unchanged_backlog() {
        let items = [
            pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
            pr(2, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
        ];
        let first = observe(&items, true, None, &[]).unwrap();
        let handled = format!("1@{:040}", 1);
        let repeated = observe(&items, true, Some(&prev(&first)), &[&handled]).unwrap();
        assert_eq!(repeated.observation_state, "observed_unchanged");
        assert!(repeated.changed_pr_numbers.is_empty());
        assert_eq!(repeated.handled_exact_heads, vec![handled.clone()]);
        assert_eq!(repeated.candidate.as_ref().unwrap().number, Some(2));
        assert_eq!(
            repeated.candidate_selection_reason.as_deref(),
            Some("unhandled_backlog_progression")
        );
        // no further handled → no duplicate candidate.
        let still_pending = observe(&items, true, Some(&prev(&repeated)), &[]).unwrap();
        assert_eq!(still_pending.observation_state, "observed_unchanged");
        assert!(still_pending.candidate.is_none());
        assert_eq!(
            still_pending.pending_candidate_exact_head.as_deref(),
            Some(format!("2@{:040}", 2).as_str())
        );
    }

    #[test]
    fn handled_exact_head_must_match_prior_candidate() {
        let first = observe(
            &[
                pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
                pr(2, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
            ],
            true,
            None,
            &[],
        )
        .unwrap();
        let err = observe(
            &[
                pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
                pr(2, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
            ],
            true,
            Some(&prev(&first)),
            &[&format!("2@{:040}", 2)],
        )
        .unwrap_err();
        assert!(err.contains("prior candidate"), "{err}");
    }

    #[test]
    fn new_head_reopens_a_previously_handled_pr() {
        let first = observe(
            &[
                pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
                pr(2, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
            ],
            true,
            None,
            &[],
        )
        .unwrap();
        let handled = format!("1@{:040}", 1);
        let result = observe(
            &[
                pr(
                    1,
                    Some(&"f".repeat(40)),
                    "REVIEW_REQUIRED",
                    false,
                    "CLEAN",
                    &[],
                ),
                pr(2, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
            ],
            true,
            Some(&prev(&first)),
            &[&handled],
        )
        .unwrap();
        let candidate = result.candidate.as_ref().unwrap();
        assert_eq!(candidate.number, Some(1));
        assert_eq!(candidate.head_oid, "f".repeat(40));
        assert_eq!(
            result.candidate_selection_reason.as_deref(),
            Some("unhandled_material_transition")
        );
        assert!(result.handled_exact_heads.is_empty());
    }

    #[test]
    fn approved_transition_routes_to_merge_policy_without_granting_it() {
        let first = observe(
            &[pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[])],
            true,
            None,
            &[],
        )
        .unwrap();
        let approved = observe(
            &[pr(1, None, "APPROVED", false, "CLEAN", &[])],
            true,
            Some(&prev(&first)),
            &[],
        )
        .unwrap();
        let todo = &approved.candidate.as_ref().unwrap().todo_preview;
        assert_eq!(todo.action_kind, "qualify_pull_request_merge_readiness");
        assert!(todo
            .text
            .contains("route any merge through repository policy"));
        assert!(!approved.write_authority_granted);
    }

    #[test]
    fn changes_requested_routes_to_p0_rereview() {
        let first = observe(
            &[pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[])],
            true,
            None,
            &[],
        )
        .unwrap();
        let changed = observe(
            &[pr(1, None, "CHANGES_REQUESTED", false, "CLEAN", &[])],
            true,
            Some(&prev(&first)),
            &[],
        )
        .unwrap();
        let todo = &changed.candidate.as_ref().unwrap().todo_preview;
        assert_eq!(todo.action_kind, "rereview_pull_request_exact_head");
        assert_eq!(todo.priority, "P0");
    }

    #[test]
    fn drafts_are_never_candidates_but_still_material() {
        let first = observe(
            &[pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[])],
            true,
            None,
            &[],
        )
        .unwrap();
        let draft = observe(
            &[pr(1, None, "REVIEW_REQUIRED", true, "CLEAN", &[])],
            true,
            Some(&prev(&first)),
            &[],
        )
        .unwrap();
        assert_eq!(draft.observation_state, "material_transition");
        assert_eq!(draft.changed_pr_numbers, vec![Some(1)]);
        assert!(draft.candidate.is_none());
        // draft-only queue: quiet backlog cadence.
        let only_draft = observe(
            &[pr(1, None, "REVIEW_REQUIRED", true, "CLEAN", &[])],
            true,
            None,
            &[],
        )
        .unwrap();
        assert_eq!(only_draft.review_backlog.actionable_unhandled_count, 0);
        assert_eq!(
            only_draft.review_backlog.recommended_poll_interval_minutes,
            15
        );
        assert_eq!(only_draft.review_backlog.recommended_cadence, "quiet_wait");
    }

    #[test]
    fn backlog_tracks_unhandled_count_and_cadence() {
        let items = [
            pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
            pr(2, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
            pr(3, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
        ];
        let first = observe(&items, true, None, &[]).unwrap();
        assert_eq!(first.review_backlog.actionable_unhandled_count, 3);
        assert_eq!(first.review_backlog.recommended_poll_interval_minutes, 3);
        assert_eq!(first.review_backlog.recommended_cadence, "active_review");

        let after_one = observe(
            &items,
            true,
            Some(&prev(&first)),
            &[&format!("1@{:040}", 1)],
        )
        .unwrap();
        assert_eq!(after_one.candidate.as_ref().unwrap().number, Some(2));
        assert_eq!(after_one.review_backlog.actionable_unhandled_count, 2);

        let after_two = observe(
            &items,
            true,
            Some(&prev(&after_one)),
            &[&format!("2@{:040}", 2)],
        )
        .unwrap();
        assert_eq!(after_two.candidate.as_ref().unwrap().number, Some(3));
        assert_eq!(after_two.review_backlog.actionable_unhandled_count, 1);

        let after_three = observe(
            &items,
            true,
            Some(&prev(&after_two)),
            &[&format!("3@{:040}", 3)],
        )
        .unwrap();
        assert!(after_three.candidate.is_none());
        assert_eq!(after_three.review_backlog.actionable_unhandled_count, 0);
        assert_eq!(
            after_three.review_backlog.recommended_poll_interval_minutes,
            15
        );
        assert_eq!(after_three.review_backlog.recommended_cadence, "quiet_wait");
    }

    #[test]
    fn queue_fingerprint_is_repository_scoped() {
        let first = build_pull_request_review_queue_observation(
            Some("owner/one"),
            &[pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[])],
            &serde_json::json!({"complete": true}),
            None,
            &[],
        )
        .unwrap();
        let second = build_pull_request_review_queue_observation(
            Some("owner/two"),
            &[pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[])],
            &serde_json::json!({"complete": true}),
            Some(&serde_json::json!({"autonomous_review": first})),
            &[],
        )
        .unwrap();
        assert_eq!(second.observation_state, "material_transition");
        assert_ne!(second.queue_fingerprint, first.queue_fingerprint);
        assert_eq!(
            second.candidate.as_ref().unwrap().repository.as_deref(),
            Some("owner/two")
        );
    }

    #[test]
    fn cursors_do_not_leak_across_repositories() {
        // Repo A: one observation projects PR 1 as candidate (handled empty,
        // projected/candidate = 1@…).
        let first = build_pull_request_review_queue_observation(
            Some("owner/one"),
            &[pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[])],
            &serde_json::json!({"complete": true}),
            None,
            &[],
        )
        .unwrap();
        assert_eq!(first.candidate.as_ref().unwrap().number, Some(1));
        // Repo B: the same PR number must be re-selectable — repo A's
        // handled/projected/candidate cursors never suppress it.
        let second = build_pull_request_review_queue_observation(
            Some("owner/two"),
            &[pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[])],
            &serde_json::json!({"complete": true}),
            Some(&serde_json::json!({"autonomous_review": first})),
            &[],
        )
        .unwrap();
        assert_eq!(second.candidate.as_ref().unwrap().number, Some(1));
        assert!(second.handled_exact_heads.is_empty());
        assert_eq!(second.projected_candidate_count, 1);
        // A handled cursor supplied from repo A is not a valid cursor for
        // repo B (the persisted-cursor validation is repository-scoped).
        let one_at = format!("1@{:040}", 1);
        let err = build_pull_request_review_queue_observation(
            Some("owner/two"),
            &[pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[])],
            &serde_json::json!({"complete": true}),
            Some(&serde_json::json!({"autonomous_review": first})),
            std::slice::from_ref(&one_at),
        )
        .unwrap_err();
        assert!(err.contains("handled exact head must match"), "{err}");
        // The same cursor IS valid for repo A (persisted-cursor advance).
        let advanced = build_pull_request_review_queue_observation(
            Some("owner/one"),
            &[pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[])],
            &serde_json::json!({"complete": true}),
            Some(&serde_json::json!({"autonomous_review": first})),
            &[one_at],
        )
        .unwrap();
        assert!(advanced.candidate.is_none());
        assert_eq!(advanced.handled_exact_head_count, 1);
    }

    #[test]
    fn incomplete_read_backlog_counts_the_supplied_items() {
        // An incomplete read still projects the actionable backlog over the
        // supplied payload (reference `_review_backlog(pull_requests, …)`).
        let result = observe(
            &[
                pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
                pr(2, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
            ],
            false,
            None,
            &[],
        )
        .unwrap();
        assert_eq!(result.observation_state, "not_observed");
        assert_eq!(result.review_backlog.actionable_unhandled_count, 2);
        assert_eq!(result.review_backlog.recommended_poll_interval_minutes, 3);
        assert_eq!(result.review_backlog.recommended_cadence, "active_review");
        // A payload with only non-actionable items counts zero and goes quiet.
        let quiet = observe(
            &[pr(1, None, "REVIEW_REQUIRED", true, "CLEAN", &[])],
            false,
            None,
            &[],
        )
        .unwrap();
        assert_eq!(quiet.review_backlog.actionable_unhandled_count, 0);
        assert_eq!(quiet.review_backlog.recommended_cadence, "quiet_wait");
    }

    #[test]
    fn check_and_mergeability_changes_are_material() {
        let first = observe(
            &[
                pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
                pr(2, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
            ],
            true,
            None,
            &[],
        )
        .unwrap();
        let changed = observe(
            &[
                pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
                pr(2, None, "REVIEW_REQUIRED", false, "BLOCKED", &["pytest"]),
            ],
            true,
            Some(&prev(&first)),
            &[],
        )
        .unwrap();
        assert_eq!(changed.changed_pr_numbers, vec![Some(2)]);
        assert_eq!(changed.candidate.as_ref().unwrap().number, Some(2));
        assert_eq!(changed.candidate.as_ref().unwrap().merge_state, "BLOCKED");
        assert_eq!(
            changed.candidate.as_ref().unwrap().checks.failures,
            vec!["pytest".to_string()]
        );
    }

    // ── review contract tests ─────────────────────────────────────────────

    #[test]
    fn verdict_parsing_and_labels() {
        assert_eq!(
            ReviewVerdict::parse("approve"),
            Some(ReviewVerdict::Approve)
        );
        assert_eq!(ReviewVerdict::parse("pass"), Some(ReviewVerdict::Approve));
        assert_eq!(ReviewVerdict::parse("通过"), Some(ReviewVerdict::Approve));
        assert_eq!(
            ReviewVerdict::parse("request-changes"),
            Some(ReviewVerdict::RequestChanges)
        );
        assert_eq!(
            ReviewVerdict::parse("驳回"),
            Some(ReviewVerdict::RequestChanges)
        );
        assert_eq!(ReviewVerdict::parse("rework"), Some(ReviewVerdict::Rework));
        assert_eq!(ReviewVerdict::parse("再修"), Some(ReviewVerdict::Rework));
        assert_eq!(ReviewVerdict::parse("maybe"), None);
        assert_eq!(ReviewVerdict::Approve.label(), "通过");
        assert_eq!(ReviewVerdict::RequestChanges.label(), "驳回");
        assert_eq!(ReviewVerdict::Rework.label(), "再修");
        assert_eq!(ReviewVerdict::RequestChanges.key(), "request_changes");
    }

    #[test]
    fn execution_contract_defines_all_eight_evidence_ids() {
        let contract = build_review_execution_contract();
        let ids: Vec<String> = contract["evidence_requirements"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["evidence_id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            ids,
            vec![
                "problem_context",
                "architecture_flow",
                "changed_line_classification",
                "symbol_map",
                "walkthroughs",
                "validation_matrix",
                "failure_analysis",
                "code_volume"
            ]
        );
        let gate = &contract["completion_gate"];
        assert_eq!(gate["stale_head_verdict_allowed"], false);
        assert_eq!(gate["code_change_symbol_minimum"], 2);
        assert_eq!(
            gate["required_final_sections"].as_array().unwrap().len(),
            REQUIRED_FINAL_SECTIONS.len()
        );
        let policy = &contract["verdict_policy"];
        assert_eq!(policy["open_pr_blocking_finding"], "REQUEST_CHANGES");
        assert_eq!(published_verdict_for(false, true), "REQUEST_CHANGES");
        assert_eq!(published_verdict_for(false, false), "COMMENT");
        assert!(published_verdict_for(true, true).contains("POST_MERGE_AUDIT_COMMENT"));
    }

    #[test]
    fn review_plan_requires_symbol_map_only_for_code_changes() {
        let code = build_review_plan(&serde_json::json!({
            "number": 7,
            "head_oid": "a".repeat(40),
            "base_ref": "main",
            "areas": {"product_runtime": 3, "ci_or_release": 1}
        }));
        let ids: Vec<String> = code["required_evidence_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert!(ids.contains(&"symbol_map".to_string()));
        assert_eq!(code["applicability"]["code_change"], true);
        assert_eq!(
            code["target"]["exact_head_key"],
            format!("7@{}", "a".repeat(40))
        );

        let docs = build_review_plan(&serde_json::json!({
            "number": 8,
            "head_oid": "b".repeat(40),
            "areas": {"public_docs": 1}
        }));
        let ids: Vec<String> = docs["required_evidence_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert!(!ids.contains(&"symbol_map".to_string()));
        assert_eq!(docs["applicability"]["docs_only"], true);
        assert_eq!(docs["applicability"]["code_change"], false);
    }

    #[test]
    fn review_template_orders_key_files_by_churn() {
        let template = build_review_template(&serde_json::json!({
            "key_files": [
                {"path": "a.rs", "additions": 10, "deletions": 2},
                {"path": "b.rs", "additions": 200, "deletions": 50},
                {"path": "c.md", "additions": 1, "deletions": 0},
                {"path": "d.rs", "additions": 30, "deletions": 30},
                {"path": "e.rs", "additions": 5, "deletions": 1},
                {"path": "f.rs", "additions": 4, "deletions": 4}
            ]
        }));
        let order: Vec<String> = template["review_order"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(order, vec!["b.rs", "d.rs", "a.rs", "f.rs", "e.rs"]);
        let sections = template["sections"].as_array().unwrap();
        assert_eq!(sections.len(), REQUIRED_FINAL_SECTIONS.len());
        let labels: Vec<String> = sections
            .iter()
            .map(|s| s["label"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(labels, REQUIRED_FINAL_SECTIONS);
    }

    #[test]
    fn agent_response_contract_embeds_execution_contract() {
        let contract = build_agent_response_contract();
        assert_eq!(
            contract["review_execution_contract"]["schema_version"],
            EXECUTION_CONTRACT_SCHEMA_VERSION
        );
        assert_eq!(contract["stats_only_requires_explicit_opt_out"], true);
    }

    // ── capability propose tests ──────────────────────────────────────────

    fn queue_input(items: Vec<Value>, handled: Vec<&str>) -> String {
        serde_json::json!({
            "repository": "owner/repo",
            "pull_requests": items,
            "result_completeness": {"complete": true},
            "handled_exact_heads": handled
        })
        .to_string()
    }

    #[test]
    fn propose_selects_one_candidate_and_binds_the_capability() {
        let cap = PrReviewQueueCapability;
        let input = queue_input(
            vec![
                pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
                pr(2, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
            ],
            vec![],
        );
        let proposals = cap.propose(&input);
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].kind, ProposalKind::SuccessorTodo);
        let todo = proposals[0].todo.as_ref().unwrap();
        assert_eq!(
            todo.action_kind.as_deref(),
            Some("review_pull_request_exact_head")
        );
        assert_eq!(
            todo.task_repository.as_deref(),
            Some("git:github.com/owner/repo")
        );
        assert_eq!(todo.required_capability.as_deref(), Some("pr_review_queue"));
        assert_eq!(
            todo.capability_binding_ref.as_deref(),
            Some("pr_review_queue")
        );
        assert!(todo.text.contains("PR #1"));
        assert!(todo.text.contains("exact head"));
    }

    #[test]
    fn propose_p0_rereview_and_quiet_wait_and_empty_input() {
        let cap = PrReviewQueueCapability;
        // CHANGES_REQUESTED → P0 candidate
        let proposals = cap.propose(&queue_input(
            vec![pr(1, None, "CHANGES_REQUESTED", false, "CLEAN", &[])],
            vec![],
        ));
        let todo = proposals[0].todo.as_ref().unwrap();
        assert_eq!(todo.priority, Priority::P0);
        assert_eq!(
            todo.action_kind.as_deref(),
            Some("rereview_pull_request_exact_head")
        );

        // empty input → no-follow-up
        let proposals = cap.propose("   ");
        assert_eq!(proposals[0].kind, ProposalKind::NoFollowUp);

        // non-payload input → gate
        let proposals = cap.propose("please review pr 5");
        assert_eq!(proposals[0].kind, ProposalKind::Gate);
    }

    #[test]
    fn propose_monitors_an_active_but_handled_backlog() {
        let cap = PrReviewQueueCapability;
        let items = [
            pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
            pr(2, None, "REVIEW_REQUIRED", false, "CLEAN", &[]),
        ];
        // Advance the handled cursor past PR 1 and let the round-robin
        // project PR 2: the candidate stays projected but unhandled — an
        // active backlog that must keep being re-observed. (Handled heads
        // must match a prior candidate or persisted cursor, so both heads
        // cannot be claimed from a single observation.)
        let first = observe(&items, true, None, &[]).unwrap();
        let second = observe(
            &items,
            true,
            Some(&prev(&first)),
            &[&format!("1@{:040}", 1)],
        )
        .unwrap();
        let input = serde_json::json!({
            "repository": "owner/repo",
            "pull_requests": items,
            "result_completeness": {"complete": true},
            "handled_exact_heads": [format!("1@{:040}", 1)],
            "previous_observation": second
        })
        .to_string();
        let proposals = cap.propose(&input);
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].kind, ProposalKind::Monitor);
        assert!(proposals[0].reason.contains("periodic re-observation"));
    }

    #[test]
    fn propose_quiet_wait_when_queue_fully_handled() {
        let cap = PrReviewQueueCapability;
        let first = observe(
            &[pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[])],
            true,
            None,
            &[],
        )
        .unwrap();
        let input = serde_json::json!({
            "repository": "owner/repo",
            "pull_requests": [pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[])],
            "result_completeness": {"complete": true},
            "handled_exact_heads": [format!("1@{:040}", 1)],
            "previous_observation": first
        })
        .to_string();
        let proposals = cap.propose(&input);
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].kind, ProposalKind::NoFollowUp);
        assert!(proposals[0].reason.contains("quiet"));
    }

    #[test]
    fn verdict_keys_cover_all_variants() {
        assert_eq!(ReviewVerdict::Approve.key(), "approve");
        assert_eq!(ReviewVerdict::RequestChanges.key(), "request_changes");
        assert_eq!(ReviewVerdict::Rework.key(), "rework");
    }

    #[test]
    fn upper_defaults_when_absent_empty_or_non_string() {
        assert_eq!(upper(None, "OPEN"), "OPEN");
        assert_eq!(upper(Some(&Value::String(String::new())), "OPEN"), "OPEN");
        assert_eq!(upper(Some(&Value::Bool(true)), "OPEN"), "OPEN");
        assert_eq!(upper(Some(&Value::String("open".into())), "OPEN"), "OPEN");
    }

    #[test]
    fn string_list_handles_absent_non_array_and_non_string_items() {
        assert_eq!(string_list(None), Vec::<String>::new());
        assert_eq!(string_list(Some(&Value::Bool(true))), Vec::<String>::new());
        assert_eq!(
            string_list(Some(&serde_json::json!([" b ", "a", true, "a"]))),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn check_snapshot_defaults_when_absent_or_without_counts() {
        assert_eq!(check_snapshot(None), CheckSnapshot::default());
        let snap = check_snapshot(Some(&serde_json::json!({"failures": ["x"]})));
        assert_eq!(snap.failures, vec!["x".to_string()]);
        assert_eq!(snap.counts, CheckCounts::default());
    }

    #[test]
    fn invalid_exact_head_and_missing_number_skip_candidate_selection() {
        let mut short = pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[]);
        short["head_oid"] = serde_json::json!("short");
        let mut nonum = pr(2, None, "REVIEW_REQUIRED", false, "CLEAN", &[]);
        nonum["number"] = serde_json::Value::Null;
        let result = observe(&[short, nonum], true, None, &[]).unwrap();
        assert_eq!(result.candidate_count, 0);
        assert!(result.candidate.is_none());
    }

    #[test]
    fn empty_repository_yields_no_repository_or_task_repository() {
        let result = build_pull_request_review_queue_observation(
            None,
            &[pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[])],
            &serde_json::json!({"complete": true}),
            None,
            &[],
        )
        .unwrap();
        assert_eq!(result.repository, None);
        let candidate = result.candidate.as_ref().unwrap();
        assert_eq!(candidate.repository, None);
        assert_eq!(candidate.todo_preview.task_repository, None);
    }

    #[test]
    fn non_open_prs_are_skipped_in_ranking() {
        let mut merged = pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[]);
        merged["state"] = serde_json::json!("MERGED");
        let result = observe(&[merged], true, None, &[]).unwrap();
        assert_eq!(result.queue_size, Some(0));
        assert!(result.candidate.is_none());
    }

    #[test]
    fn extract_previous_handles_missing_at_and_unusable_state() {
        let p = extract_previous(Some(&serde_json::json!({
            "pending_candidate_exact_head": "noatsign"
        })))
        .unwrap();
        assert!(p.candidate_exact_head.is_none());
        let p = extract_previous(Some(&serde_json::json!({
            "observation_state": "observed_unchanged"
        })))
        .unwrap();
        assert!(p.items.is_empty());
        let p = extract_previous(Some(&serde_json::json!({
            "observation_state": "not_observed"
        })))
        .unwrap();
        assert!(p.items.is_empty());
    }

    #[test]
    fn review_plan_target_is_null_without_number_or_head() {
        let plan = build_review_plan(&serde_json::json!({"areas": {"src": 1}}));
        assert!(plan["target"]["exact_head_key"].is_null());
        assert!(plan["target"]["number"].is_null());
    }

    #[test]
    fn propose_rejects_unexpected_handled_head() {
        let cap = PrReviewQueueCapability;
        let input = serde_json::json!({
            "repository": "owner/repo",
            "pull_requests": [pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[])],
            "result_completeness": {"complete": true},
            "handled_exact_heads": [format!("9@{:040}", 9)],
        })
        .to_string();
        let proposals = cap.propose(&input);
        assert_eq!(proposals[0].kind, ProposalKind::Gate);
    }

    #[test]
    fn propose_monitors_an_incomplete_read_with_baseline() {
        let cap = PrReviewQueueCapability;
        let baseline = observe(
            &[pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[])],
            true,
            None,
            &[],
        )
        .unwrap();
        let input = serde_json::json!({
            "repository": "owner/repo",
            "pull_requests": [pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[])],
            "result_completeness": {"complete": false},
            "previous_observation": baseline,
        })
        .to_string();
        let proposals = cap.propose(&input);
        assert_eq!(proposals[0].kind, ProposalKind::Monitor);
    }

    #[test]
    fn propose_successor_for_incomplete_read_without_baseline() {
        let cap = PrReviewQueueCapability;
        let input = serde_json::json!({
            "repository": "owner/repo",
            "pull_requests": [pr(1, None, "REVIEW_REQUIRED", false, "CLEAN", &[])],
            "result_completeness": {"complete": false},
        })
        .to_string();
        let proposals = cap.propose(&input);
        assert_eq!(proposals[0].kind, ProposalKind::SuccessorTodo);
    }
}
