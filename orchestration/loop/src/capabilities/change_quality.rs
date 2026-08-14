//! Change-Quality capability (LoopX: change-quality — qualification of a
//! change before it is accepted; quality signals become repair /
//! no-follow-up proposals, not approvals).
//!
//! Wave 2 deepening: the 50-line keyword shell becomes a structured
//! qualification pipeline, porting the core subdomains of the reference
//! `capabilities/change_quality/` (policy / scope identity / result contract
//! / guardrail derivation / validation-plan oracles) as deterministic rule
//! versions:
//!
//! - **policy**: goal-level qualification flags (enabled / safe_fix /
//!   strict_receipt) parsed from `control_plane.change_quality_qualification`;
//! - **result contract**: `change_quality_agent_result_v2` normalization —
//!   bounded public-safe text, repo-relative evidence refs, reuse and
//!   simplification conclusions, sparse risks[] / validation[], exact-scope
//!   fingerprint agreement, and evidence-target grounding (every ref must
//!   name a changed file, a projected instruction, or a declared validator);
//! - **guardrail derivation**: per-lens states (blocked / risk / resolved /
//!   satisfied / not_triggered) are derived from risks[] + validation[] —
//!   never authored by prose — plus the blocking-code decision (pass/fail);
//! - **validation-plan oracles**: repository-declared quality tasks
//!   discovered from pyproject.toml (poe / hatch / mypy / pyright / pytest),
//!   package.json scripts, and .cargo/config.toml aliases — category
//!   inference (format/lint/typecheck/test) and stable sha256 oracle ids,
//!   without copying task bodies;
//! - **scope identity**: an exact-scope fingerprint over host-supplied git
//!   outputs (commits + per-source diff payloads + untracked paths);
//! - **propose**: routes payloads and free-text change evidence into a
//!   FINITE set of typed proposals (successor / gate / no-follow-up). A
//!   capability never writes state itself: it proposes; the kernel decides.
//!
//! Out of scope (deliberately): the git-subprocess scope capture, receipt
//! storage, and PR-evidence CLI of the reference — those stay LoopX-side;
//! this module ships the deterministic contract + oracle core and takes
//! host-supplied git outputs as input payloads.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{successor_todo, Capability, TypedProposal};

// ── schema vocabulary (reference policy.py / result.py constants) ─────────

pub const CHANGE_QUALITY_RESULT_SCHEMA_VERSION: &str = "change_quality_agent_result_v2";
pub const CHANGE_QUALITY_POLICY_SCHEMA_VERSION: &str = "change_quality_qualification_policy_v0";
pub const CHANGE_QUALITY_GUARDRAIL_SCHEMA_VERSION: &str = "change_quality_guardrail_summary_v0";
pub const CHANGE_QUALITY_VALIDATION_PLAN_SCHEMA_VERSION: &str = "change_quality_validation_plan_v0";
pub const CHANGE_QUALITY_SCOPE_SCHEMA_VERSION: &str = "change_quality_scope_v0";
pub const CHANGE_QUALITY_PREPARE_SCHEMA_VERSION: &str = "change_quality_prepare_packet_v2";

pub const RISK_SEVERITY_BLOCKER: &str = "blocker";
pub const RISK_SEVERITY_WARNING: &str = "warning";
pub const RISK_SEVERITY_ADVISORY: &str = "advisory";
pub const RISK_SEVERITIES: [&str; 3] = [
    RISK_SEVERITY_BLOCKER,
    RISK_SEVERITY_WARNING,
    RISK_SEVERITY_ADVISORY,
];

pub const REUSE_OUTCOMES: [&str; 4] = ["reused", "retained", "deferred", "not_applicable"];
pub const SIMPLIFICATION_OUTCOMES: [&str; 4] = ["fixed", "retained", "deferred", "not_applicable"];

pub const VALIDATION_STATUS_PASSED: &str = "passed";
pub const VALIDATION_STATUS_FAILED: &str = "failed";
pub const VALIDATION_STATUS_SKIPPED: &str = "skipped";
pub const VALIDATION_STATUSES: [&str; 3] = [
    VALIDATION_STATUS_PASSED,
    VALIDATION_STATUS_FAILED,
    VALIDATION_STATUS_SKIPPED,
];

pub const EVIDENCE_REF_KIND_PATH: &str = "path";
pub const EVIDENCE_REF_KIND_INSTRUCTION: &str = "instruction";
pub const EVIDENCE_REF_KIND_VALIDATOR: &str = "validator";
pub const EVIDENCE_REF_KINDS: [&str; 3] = [
    EVIDENCE_REF_KIND_PATH,
    EVIDENCE_REF_KIND_INSTRUCTION,
    EVIDENCE_REF_KIND_VALIDATOR,
];

pub const VALIDATION_CATEGORIES: [&str; 4] = ["format", "lint", "typecheck", "test"];

/// One review lens from the reference `REVIEW_LENSES` catalog: a durable
/// question an exact-scope review must answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReviewLens {
    pub lens_id: &'static str,
    pub question: &'static str,
}

pub const REVIEW_LENSES: [ReviewLens; 10] = [
    ReviewLens {
        lens_id: "reuse",
        question: "Does the change reuse established helpers and durable knowledge instead of duplicating them?",
    },
    ReviewLens {
        lens_id: "type_api_boundary",
        question: "Are types, schemas, compatibility windows, and caller-facing contracts explicit and coherent?",
    },
    ReviewLens {
        lens_id: "configuration",
        question: "Does configuration stay single-sourced, validated, and free of hidden mode coupling?",
    },
    ReviewLens {
        lens_id: "runtime_ownership",
        question: "Are lifecycle, concurrency, state, and side-effect ownership placed in the correct runtime boundary?",
    },
    ReviewLens {
        lens_id: "quality_simplification",
        question: "Can the final behavior be expressed with less indirection, branching, duplication, or speculative abstraction?",
    },
    ReviewLens {
        lens_id: "efficiency",
        question: "Does the change avoid unnecessary work, unbounded growth, and avoidable hot-path cost?",
    },
    ReviewLens {
        lens_id: "error_supervision",
        question: "Are failures observable, actionable, and supervised without silent fallback or broad exception plumbing?",
    },
    ReviewLens {
        lens_id: "test_validation",
        question: "Do tests and repository-native validators prove the intended semantics, including important negative paths?",
    },
    ReviewLens {
        lens_id: "documentation_comments",
        question: "Do names, comments, and docs explain the current contract without stale narration or duplicated truth?",
    },
    ReviewLens {
        lens_id: "security_release",
        question: "Are security, privacy, permissions, migrations, and release compatibility handled at the changed boundaries?",
    },
];

/// Primary result fields (reference `SIMPLIFY_PRIMARY_LENS_IDS`): the agent
/// authors these conclusions directly.
pub const SIMPLIFY_PRIMARY_LENS_IDS: [&str; 2] = ["reuse", "quality_simplification"];
/// Guardrail lenses (reference `SIMPLIFY_GUARDRAIL_LENS_IDS`): states are
/// derived from sparse risks[] + validation[], never authored by prose.
pub const SIMPLIFY_GUARDRAIL_LENS_IDS: [&str; 8] = [
    "type_api_boundary",
    "configuration",
    "runtime_ownership",
    "efficiency",
    "error_supervision",
    "test_validation",
    "documentation_comments",
    "security_release",
];

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

// ── policy (reference policy.py) ───────────────────────────────────────────

/// Qualification flags for one goal (reference `change_quality_goal_policy`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChangeQualityPolicy {
    pub enabled: bool,
    pub safe_fix: bool,
    pub strict_receipt: bool,
}

impl ChangeQualityPolicy {
    pub fn to_value(self) -> Value {
        serde_json::json!({
            "schema_version": CHANGE_QUALITY_POLICY_SCHEMA_VERSION,
            "enabled": self.enabled,
            "safe_fix": self.safe_fix,
            "strict_receipt": self.strict_receipt,
        })
    }
}

/// Parse the qualification policy from a goal JSON
/// (`control_plane.change_quality_qualification` — reference
/// `change_quality_goal_policy`).
pub fn change_quality_policy(goal: &Value) -> ChangeQualityPolicy {
    let control_plane = goal.get("control_plane").and_then(Value::as_object);
    let raw = control_plane
        .and_then(|plane| plane.get("change_quality_qualification"))
        .and_then(Value::as_object);
    ChangeQualityPolicy::from_qualification(raw)
}

impl ChangeQualityPolicy {
    /// Strict `is True` semantics (reference policy.py): flags default off.
    fn from_qualification(raw: Option<&serde_json::Map<String, Value>>) -> ChangeQualityPolicy {
        ChangeQualityPolicy {
            enabled: raw.and_then(|m| m.get("enabled")).and_then(Value::as_bool) == Some(true),
            safe_fix: raw.and_then(|m| m.get("safe_fix")).and_then(Value::as_bool) == Some(true),
            strict_receipt: raw
                .and_then(|m| m.get("strict_receipt"))
                .and_then(Value::as_bool)
                == Some(true),
        }
    }
}

// ── public-safe bounded text (reference result.py _bounded_text etc.) ─────

/// `C:\Users` / `C:/Users`-style drive paths (mirrors explore.rs).
fn contains_windows_abs_path(text: &str) -> bool {
    let bytes = text.as_bytes();
    for (index, _) in text.char_indices() {
        let Some(drive) = text[index..].chars().next() else {
            continue;
        };
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

/// Coerce any JSON scalar to text (reference `str(value or "")`).
fn text_of(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        _ => String::new(),
    }
}

fn reject_private_material(text: &str, field: &str) -> Result<(), String> {
    let lowered = text.to_lowercase();
    if lowered.contains("file://") || contains_windows_abs_path(text) {
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
    Ok(())
}

/// Collapse whitespace, reject private material, enforce the length limit
/// (reference `_bounded_text`: exceeding the limit is an error, not a
/// truncation).
fn bounded_text(value: &Value, field: &str, limit: usize) -> Result<String, String> {
    let text = text_of(value)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    reject_private_material(&text, field)?;
    if text.chars().count() > limit {
        return Err(format!("{field} exceeds {limit} characters"));
    }
    Ok(text)
}

/// Repo-relative path normalization (reference `_relative_path`): absolute
/// paths and `..` traversal are rejected; None for empty input.
fn relative_path(value: &Value, field: &str) -> Result<Option<String>, String> {
    let text = text_of(value).trim().replace('\\', "/");
    if text.is_empty() {
        return Ok(None);
    }
    if text.starts_with('/') || text.split('/').any(|part| part == "..") {
        return Err(format!("{field} must be a repo-relative path"));
    }
    reject_private_material(&text, field)?;
    Ok(Some(text))
}

fn normalize_evidence_ref(value: &Value, field: &str) -> Result<String, String> {
    let text = bounded_text(value, field, 400)?;
    let hint = format!("{field} must use one of [instruction:<ref>, path:<ref>, validator:<ref>]");
    let Some((kind, target)) = text.split_once(':') else {
        return Err(hint);
    };
    if !EVIDENCE_REF_KINDS.contains(&kind) {
        return Err(hint);
    }
    let normalized = if kind == EVIDENCE_REF_KIND_PATH || kind == EVIDENCE_REF_KIND_INSTRUCTION {
        relative_path(&Value::String(target.to_string()), field)?
    } else {
        Some(bounded_text(
            &Value::String(target.to_string()),
            field,
            160,
        )?)
    };
    let Some(target) = normalized else {
        return Err(format!("{field} requires a non-empty reference target"));
    };
    if target.is_empty() {
        return Err(format!("{field} requires a non-empty reference target"));
    }
    Ok(format!("{kind}:{target}"))
}

fn normalize_evidence_refs(value: Option<&Value>, field: &str) -> Result<Vec<String>, String> {
    let Some(array) = value.and_then(Value::as_array) else {
        return Err(format!("{field} must be an array"));
    };
    if array.len() > 20 {
        return Err(format!("{field} supports at most 20 items"));
    }
    let refs: Vec<String> = array
        .iter()
        .map(|item| normalize_evidence_ref(item, &format!("{field}[]")))
        .collect::<Result<_, _>>()?;
    let unique: BTreeSet<&str> = refs.iter().map(String::as_str).collect();
    if unique.len() != refs.len() {
        return Err(format!("{field} must not contain duplicates"));
    }
    Ok(refs)
}

// ── result normalization (reference result.py) ─────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conclusion {
    pub outcome: String,
    pub summary: String,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualityRisk {
    pub category: String,
    pub severity: String,
    pub code: String,
    pub message: String,
    pub resolved: bool,
    pub evidence_refs: Vec<String>,
    pub path: Option<String>,
    pub line: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationEntry {
    pub validator: String,
    pub status: String,
    pub scope: String,
    pub required: bool,
    pub command: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedChangeQualityResult {
    pub schema_version: String,
    pub scope_fingerprint: String,
    pub reviewed_final_scope: bool,
    pub reuse: Conclusion,
    pub simplification: Conclusion,
    pub safe_fix_applied: bool,
    pub risks: Vec<QualityRisk>,
    pub validation: Vec<ValidationEntry>,
}

fn normalize_conclusion(
    value: Option<&Value>,
    field: &str,
    allowed_outcomes: &[&str],
) -> Result<Conclusion, String> {
    let object = value
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{field} must be an object"))?;
    let outcome = object
        .get("outcome")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    if !allowed_outcomes.contains(&outcome.as_str()) {
        return Err(format!(
            "{field}.outcome must be one of {allowed_outcomes:?}"
        ));
    }
    let summary = bounded_text(
        object.get("summary").unwrap_or(&Value::Null),
        &format!("{field}.summary"),
        400,
    )?;
    if summary.is_empty() {
        return Err(format!("{field}.summary is required"));
    }
    let evidence_refs = normalize_evidence_refs(
        object.get("evidence_refs"),
        &format!("{field}.evidence_refs"),
    )?;
    if evidence_refs.is_empty() {
        return Err(format!(
            "{field}.evidence_refs must contain grounded evidence"
        ));
    }
    Ok(Conclusion {
        outcome,
        summary,
        evidence_refs,
    })
}

fn normalize_simplification(
    value: Option<&Value>,
    safe_fix_allowed: bool,
) -> Result<(Conclusion, bool), String> {
    let simplification = normalize_conclusion(value, "simplification", &SIMPLIFICATION_OUTCOMES)?;
    let object = value.and_then(Value::as_object);
    let safe_fix_applied =
        object.and_then(|o| o.get("safe_fix_applied")) == Some(&Value::Bool(true));
    if safe_fix_applied && !safe_fix_allowed {
        return Err(
            "result reports simplification.safe_fix_applied but goal policy forbids safe fixes"
                .to_string(),
        );
    }
    if safe_fix_applied != (simplification.outcome == "fixed") {
        return Err(
            "simplification.outcome=fixed and safe_fix_applied=true must agree".to_string(),
        );
    }
    Ok((simplification, safe_fix_applied))
}

fn normalize_risk(value: &Value, index: usize) -> Result<QualityRisk, String> {
    let field = format!("risks[{index}]");
    let object = value
        .as_object()
        .ok_or_else(|| format!("{field} must be an object"))?;
    let category = object
        .get("category")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if !SIMPLIFY_GUARDRAIL_LENS_IDS.contains(&category.as_str()) {
        return Err(format!(
            "{field}.category must be one of {SIMPLIFY_GUARDRAIL_LENS_IDS:?}"
        ));
    }
    let severity = object
        .get("severity")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    if !RISK_SEVERITIES.contains(&severity.as_str()) {
        return Err(format!(
            "{field}.severity must be one of {RISK_SEVERITIES:?}"
        ));
    }
    let code = bounded_text(
        object.get("code").unwrap_or(&Value::Null),
        &format!("{field}.code"),
        80,
    )?;
    let message = bounded_text(
        object.get("message").unwrap_or(&Value::Null),
        &format!("{field}.message"),
        320,
    )?;
    if code.is_empty() || message.is_empty() {
        return Err(format!("{field} requires code and message"));
    }
    let evidence_refs = normalize_evidence_refs(
        object.get("evidence_refs"),
        &format!("{field}.evidence_refs"),
    )?;
    if evidence_refs.is_empty() {
        return Err(format!(
            "{field}.evidence_refs must contain grounded evidence"
        ));
    }
    let path = relative_path(
        object.get("path").unwrap_or(&Value::Null),
        &format!("{field}.path"),
    )?;
    let line = match object.get("line") {
        Some(Value::Number(number)) => {
            if number.as_u64().is_none_or(|line| line < 1) {
                return Err(format!("{field}.line must be a positive integer"));
            }
            number.as_u64()
        }
        Some(_) => return Err(format!("{field}.line must be a positive integer")),
        None => None,
    };
    Ok(QualityRisk {
        category,
        severity,
        code,
        message,
        resolved: object.get("resolved") == Some(&Value::Bool(true)),
        evidence_refs,
        path,
        line,
    })
}

fn normalize_risks(value: Option<&Value>) -> Result<Vec<QualityRisk>, String> {
    let Some(array) = value.and_then(Value::as_array) else {
        return Err("risks must be an array".to_string());
    };
    if array.len() > 20 {
        return Err("risks supports at most 20 items".to_string());
    }
    let risks: Vec<QualityRisk> = array
        .iter()
        .enumerate()
        .map(|(index, item)| normalize_risk(item, index))
        .collect::<Result<_, _>>()?;
    let codes: BTreeSet<&str> = risks.iter().map(|risk| risk.code.as_str()).collect();
    if codes.len() != risks.len() {
        return Err("risk codes must be unique".to_string());
    }
    Ok(risks)
}

fn normalize_validation_entry(value: &Value, index: usize) -> Result<ValidationEntry, String> {
    let field = format!("validation[{index}]");
    let object = value
        .as_object()
        .ok_or_else(|| format!("{field} must be an object"))?;
    let status = object
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    if !VALIDATION_STATUSES.contains(&status.as_str()) {
        return Err(format!(
            "{field}.status must be one of {VALIDATION_STATUSES:?}"
        ));
    }
    let validator = bounded_text(
        object.get("validator").unwrap_or(&Value::Null),
        &format!("{field}.validator"),
        120,
    )?;
    let scope = bounded_text(
        object.get("scope").unwrap_or(&Value::Null),
        &format!("{field}.scope"),
        240,
    )?;
    if validator.is_empty() || scope.is_empty() {
        return Err(format!("{field} requires validator and scope"));
    }
    let required = object.get("required") != Some(&Value::Bool(false));
    let command = object
        .get("command")
        .map(|value| bounded_text(value, &format!("{field}.command"), 320))
        .transpose()?;
    let reason = object
        .get("reason")
        .map(|value| bounded_text(value, &format!("{field}.reason"), 320))
        .transpose()?;
    if matches!(
        status.as_str(),
        VALIDATION_STATUS_FAILED | VALIDATION_STATUS_SKIPPED
    ) && reason.is_none()
    {
        return Err(format!("{field} with status={status} requires reason"));
    }
    Ok(ValidationEntry {
        validator,
        status,
        scope,
        required,
        command,
        reason,
    })
}

fn normalize_validations(value: Option<&Value>) -> Result<Vec<ValidationEntry>, String> {
    let Some(array) = value.and_then(Value::as_array) else {
        return Err("validation must contain at least one item".to_string());
    };
    if array.is_empty() {
        return Err("validation must contain at least one item".to_string());
    }
    if array.len() > 20 {
        return Err("validation supports at most 20 items".to_string());
    }
    let validation: Vec<ValidationEntry> = array
        .iter()
        .enumerate()
        .map(|(index, item)| normalize_validation_entry(item, index))
        .collect::<Result<_, _>>()?;
    let validators: BTreeSet<&str> = validation
        .iter()
        .map(|entry| entry.validator.as_str())
        .collect();
    if validators.len() != validation.len() {
        return Err("validation validator ids must be unique".to_string());
    }
    Ok(validation)
}

fn validate_evidence_targets(
    refs: &[String],
    field: &str,
    changed_files: &BTreeSet<&str>,
    instruction_refs: &BTreeSet<&str>,
    validator_ids: &BTreeSet<&str>,
) -> Result<(), String> {
    for evidence_ref in refs {
        let Some((kind, target)) = evidence_ref.split_once(':') else {
            return Err(format!(
                "{field} references malformed evidence: {evidence_ref}"
            ));
        };
        let known = match kind {
            EVIDENCE_REF_KIND_PATH => changed_files.contains(target),
            EVIDENCE_REF_KIND_INSTRUCTION => instruction_refs.contains(target),
            EVIDENCE_REF_KIND_VALIDATOR => validator_ids.contains(target),
            _ => false,
        };
        if !known {
            return Err(format!(
                "{field} references unknown evidence: {evidence_ref}"
            ));
        }
    }
    Ok(())
}

/// Normalize an agent result against the exact-scope contract (reference
/// `normalize_change_quality_result`): schema + field whitelist + fingerprint
/// agreement + grounded conclusions/risks/validation.
pub fn normalize_change_quality_result(
    value: &Value,
    expected_fingerprint: &str,
    safe_fix_allowed: bool,
    expected_changed_files: Option<&[String]>,
    expected_instruction_refs: Option<&[String]>,
) -> Result<NormalizedChangeQualityResult, String> {
    let object = value
        .as_object()
        .ok_or("result JSON root must be an object")?;
    if object.get("schema_version").and_then(Value::as_str)
        != Some(CHANGE_QUALITY_RESULT_SCHEMA_VERSION)
    {
        return Err(format!(
            "result schema_version must be {CHANGE_QUALITY_RESULT_SCHEMA_VERSION}"
        ));
    }
    const ALLOWED_FIELDS: [&str; 7] = [
        "schema_version",
        "scope_fingerprint",
        "reviewed_final_scope",
        "reuse",
        "simplification",
        "risks",
        "validation",
    ];
    let mut unsupported: Vec<&str> = object
        .keys()
        .filter(|key| !ALLOWED_FIELDS.contains(&key.as_str()))
        .map(String::as_str)
        .collect();
    if !unsupported.is_empty() {
        unsupported.sort_unstable();
        return Err(format!(
            "result contains unsupported fields: {unsupported:?}"
        ));
    }
    let fingerprint = object
        .get("scope_fingerprint")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if fingerprint != expected_fingerprint {
        return Err(
            "result scope_fingerprint does not match the current exact diff; rerun prepare after every safe fix"
                .to_string(),
        );
    }
    if object.get("reviewed_final_scope") != Some(&Value::Bool(true)) {
        return Err("result must set reviewed_final_scope=true".to_string());
    }
    let reuse = normalize_conclusion(object.get("reuse"), "reuse", &REUSE_OUTCOMES)?;
    let (simplification, safe_fix_applied) =
        normalize_simplification(object.get("simplification"), safe_fix_allowed)?;
    let risks = normalize_risks(object.get("risks"))?;
    let validation = normalize_validations(object.get("validation"))?;
    let changed_files: BTreeSet<&str> = expected_changed_files
        .map(|files| files.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let instruction_refs: BTreeSet<&str> = expected_instruction_refs
        .map(|refs| refs.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let validator_ids: BTreeSet<&str> = validation
        .iter()
        .map(|entry| entry.validator.as_str())
        .collect();
    validate_evidence_targets(
        &reuse.evidence_refs,
        "reuse",
        &changed_files,
        &instruction_refs,
        &validator_ids,
    )?;
    validate_evidence_targets(
        &simplification.evidence_refs,
        "simplification",
        &changed_files,
        &instruction_refs,
        &validator_ids,
    )?;
    for (index, risk) in risks.iter().enumerate() {
        validate_evidence_targets(
            &risk.evidence_refs,
            &format!("risks[{index}]"),
            &changed_files,
            &instruction_refs,
            &validator_ids,
        )?;
        if risk.path.is_some()
            && expected_changed_files.is_some()
            && !changed_files.contains(risk.path.as_deref().unwrap_or_default())
        {
            return Err(format!("risks[{index}].path must name a changed file"));
        }
    }
    Ok(NormalizedChangeQualityResult {
        schema_version: CHANGE_QUALITY_RESULT_SCHEMA_VERSION.to_string(),
        scope_fingerprint: fingerprint.to_string(),
        reviewed_final_scope: true,
        reuse,
        simplification,
        safe_fix_applied,
        risks,
        validation,
    })
}

// ── guardrail derivation + decision (reference result.py) ─────────────────

/// Derive per-lens guardrail states from sparse risks[] + validation[]
/// (reference `derive_change_quality_guardrails`): LoopX is the status
/// owner — the agent never authors guardrail states.
pub fn derive_change_quality_guardrails(result: &NormalizedChangeQualityResult) -> Value {
    let mut states: Vec<Value> = Vec::new();
    let mut blocking_codes: Vec<String> = Vec::new();
    for lens_id in SIMPLIFY_GUARDRAIL_LENS_IDS {
        let category_risks: Vec<&QualityRisk> = result
            .risks
            .iter()
            .filter(|risk| risk.category == lens_id)
            .collect();
        let unresolved_blockers: Vec<String> = category_risks
            .iter()
            .filter(|risk| risk.severity == RISK_SEVERITY_BLOCKER && !risk.resolved)
            .map(|risk| risk.code.clone())
            .collect();
        let mut validation_blockers: Vec<String> = Vec::new();
        let mut optional_skips: Vec<String> = Vec::new();
        if lens_id == "test_validation" {
            validation_blockers = result
                .validation
                .iter()
                .filter(|entry| {
                    entry.status == VALIDATION_STATUS_FAILED
                        || (entry.status == VALIDATION_STATUS_SKIPPED && entry.required)
                })
                .map(|entry| format!("validator:{}", entry.validator))
                .collect();
            optional_skips = result
                .validation
                .iter()
                .filter(|entry| entry.status == VALIDATION_STATUS_SKIPPED && !entry.required)
                .map(|entry| format!("validator:{}", entry.validator))
                .collect();
        }
        let mut current_blockers = unresolved_blockers.clone();
        current_blockers.extend(validation_blockers.iter().cloned());
        blocking_codes.extend(current_blockers.iter().cloned());
        let unresolved_risks: Vec<String> = category_risks
            .iter()
            .filter(|risk| !risk.resolved)
            .map(|risk| risk.code.clone())
            .collect();
        let status = if !current_blockers.is_empty() {
            "blocked"
        } else if !unresolved_risks.is_empty() || !optional_skips.is_empty() {
            "risk"
        } else if !category_risks.is_empty() {
            "resolved"
        } else if lens_id == "test_validation" && !result.validation.is_empty() {
            "satisfied"
        } else {
            "not_triggered"
        };
        states.push(serde_json::json!({
            "guardrail_id": lens_id,
            "status": status,
            "risk_codes": category_risks.iter().map(|risk| risk.code.clone()).collect::<Vec<_>>(),
            "blocking_codes": current_blockers,
        }));
    }
    serde_json::json!({
        "schema_version": CHANGE_QUALITY_GUARDRAIL_SCHEMA_VERSION,
        "derived": true,
        "states": states,
        "blocking_codes": blocking_codes,
    })
}

/// The qualification decision (reference `change_quality_result_decision`):
/// any blocking code fails the change.
pub fn change_quality_result_decision(
    result: &NormalizedChangeQualityResult,
) -> (&'static str, Vec<String>) {
    let guardrails = derive_change_quality_guardrails(result);
    let blocking_codes: Vec<String> = guardrails
        .get("blocking_codes")
        .and_then(Value::as_array)
        .map(|codes| {
            codes
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if blocking_codes.is_empty() {
        ("pass", Vec::new())
    } else {
        ("fail", blocking_codes)
    }
}

// ── validation-plan oracles (reference oracles.py core) ────────────────────

pub const TASK_LIMIT: usize = 64;
const NON_ORACLE_PATH_PARTS: [&str; 5] = [
    "fixtures",
    "node_modules",
    "testdata",
    "third_party",
    "vendor",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationPlanCandidate {
    pub oracle_id: String,
    pub category: String,
    pub runner: String,
    pub task: String,
    pub source_ref: String,
    pub language_hints: Vec<String>,
    pub origin: &'static str,
    pub execution_mode: &'static str,
}

impl ValidationPlanCandidate {
    fn to_value(&self) -> Value {
        serde_json::json!({
            "oracle_id": self.oracle_id,
            "category": self.category,
            "runner": self.runner,
            "task": self.task,
            "source_ref": self.source_ref,
            "language_hints": self.language_hints,
            "origin": self.origin,
            "execution_mode": self.execution_mode,
        })
    }
}

/// Tokens of a task name split on non-alphanumeric runs (reference
/// `re.split(r"[^a-z0-9]+", task_name.lower())`).
fn task_tokens(name: &str) -> Vec<String> {
    name.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect()
}

/// Category inference for a repository-declared task name (reference
/// `_task_category`): format → lint → typecheck → test.
fn task_category(task_name: &str) -> Option<&'static str> {
    let tokens = task_tokens(task_name);
    let token_set: BTreeSet<&str> = tokens.iter().map(String::as_str).collect();
    let compact: String = tokens.concat();
    const FORMAT_TOKENS: [&str; 3] = ["fmt", "format", "formatting"];
    const LINT_TOKENS: [&str; 3] = ["lint", "linter", "clippy"];
    const TEST_TOKENS: [&str; 3] = ["test", "tests", "pytest"];
    const TYPECHECK_TOKENS: [&str; 2] = ["typecheck", "typechecking"];
    if FORMAT_TOKENS.iter().any(|token| token_set.contains(token)) {
        return Some("format");
    }
    if LINT_TOKENS.iter().any(|token| token_set.contains(token)) {
        return Some("lint");
    }
    let typecheck = TYPECHECK_TOKENS
        .iter()
        .any(|token| token_set.contains(token))
        || matches!(
            compact.as_str(),
            "typecheck" | "checktypes" | "typechecking"
        )
        || (token_set.contains("type") && token_set.contains("check"));
    if typecheck {
        return Some("typecheck");
    }
    if TEST_TOKENS.iter().any(|token| token_set.contains(token)) {
        return Some("test");
    }
    None
}

/// Reference `_SAFE_TASK_NAME` (no regex crate: hand-rolled ASCII check).
fn safe_task_name(task: &str) -> bool {
    let mut chars = task.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphanumeric() || first == '@') {
        return false;
    }
    task.len() <= 128
        && task
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '@' | '.' | '_' | ':' | '-'))
}

/// Stable oracle id: sha256(source_ref \0 runner \0 task) hex, first 16
/// chars (reference `_oracle_id`).
fn oracle_id(source_ref: &str, runner: &str, task: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source_ref.as_bytes());
    hasher.update([0u8]);
    hasher.update(runner.as_bytes());
    hasher.update([0u8]);
    hasher.update(task.as_bytes());
    let hex = format!("{:x}", hasher.finalize());
    format!("oracle_{}", &hex[..16])
}

fn source_ref(path: &str, parts: &[&str]) -> String {
    if parts.is_empty() {
        path.to_string()
    } else {
        format!("{path}#{}", parts.join("."))
    }
}

/// A declared task that passes the name/category contract (reference
/// `_candidate`).
fn declared_task_candidate(
    source_ref: &str,
    runner: &str,
    task: &str,
    languages: &[&str],
) -> Option<ValidationPlanCandidate> {
    if !safe_task_name(task) {
        return None;
    }
    let category = task_category(task)?;
    Some(ValidationPlanCandidate {
        oracle_id: oracle_id(source_ref, runner, task),
        category: category.to_string(),
        runner: runner.to_string(),
        task: task.to_string(),
        source_ref: source_ref.to_string(),
        language_hints: languages.iter().map(|l| (*l).to_string()).collect(),
        origin: "repository_declared_task",
        execution_mode: "host_resolved",
    })
}

fn sorted_keys(object: Option<&Value>) -> Vec<String> {
    let mut keys: Vec<String> = object
        .and_then(Value::as_object)
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default();
    keys.sort();
    keys
}

fn tool_config_candidate(
    path: &str,
    tool_name: &str,
    category: &'static str,
    config_parts: &[&str],
) -> ValidationPlanCandidate {
    ValidationPlanCandidate {
        oracle_id: oracle_id(
            &source_ref(path, config_parts),
            &format!("{tool_name}_config"),
            tool_name,
        ),
        category: category.to_string(),
        runner: format!("{tool_name}_config"),
        task: tool_name.to_string(),
        source_ref: source_ref(path, config_parts),
        language_hints: vec!["python".to_string()],
        origin: "repository_tool_config",
        execution_mode: "host_resolved",
    }
}

/// pyproject.toml candidates: poe tasks, hatch env scripts, and configured
/// mypy / pyright / pytest tools (reference `_python_candidates`).
fn python_candidates(path: &str, payload: &Value) -> Vec<ValidationPlanCandidate> {
    let mut candidates: Vec<ValidationPlanCandidate> = Vec::new();
    let tool = payload.get("tool").and_then(Value::as_object);

    for task in sorted_keys(tool.and_then(|t| t.get("poe")).and_then(|p| p.get("tasks"))) {
        if let Some(candidate) = declared_task_candidate(
            &source_ref(path, &["tool", "poe", "tasks", &task]),
            "poe_task",
            &task,
            &["python"],
        ) {
            candidates.push(candidate);
        }
    }

    let hatch_envs = tool
        .and_then(|t| t.get("hatch"))
        .and_then(|h| h.get("envs"));
    for environment in sorted_keys(hatch_envs) {
        let scripts = hatch_envs
            .and_then(|envs| envs.get(&environment))
            .and_then(|env| env.get("scripts"));
        for task in sorted_keys(scripts) {
            if let Some(candidate) = declared_task_candidate(
                &source_ref(
                    path,
                    &["tool", "hatch", "envs", &environment, "scripts", &task],
                ),
                "hatch_script",
                &task,
                &["python"],
            ) {
                candidates.push(candidate);
            }
        }
    }

    for (tool_name, category) in [
        ("mypy", "typecheck"),
        ("pyright", "typecheck"),
        ("pytest", "test"),
    ] {
        let config_parts: Vec<&str> = if tool_name == "pytest" {
            vec!["tool", "pytest", "ini_options"]
        } else {
            vec!["tool", tool_name]
        };
        let configured = {
            let mut current: Option<&serde_json::Map<String, Value>> = payload.as_object();
            for part in &config_parts {
                current = current
                    .and_then(|map| map.get(*part))
                    .and_then(Value::as_object);
            }
            current
        };
        if configured.is_some() {
            candidates.push(tool_config_candidate(
                path,
                tool_name,
                category,
                &config_parts,
            ));
        }
    }
    candidates
}

/// package.json scripts (reference `_node_candidates`).
fn node_candidates(path: &str, payload: &Value) -> Vec<ValidationPlanCandidate> {
    let mut candidates: Vec<ValidationPlanCandidate> = Vec::new();
    for task in sorted_keys(payload.get("scripts")) {
        if let Some(candidate) = declared_task_candidate(
            &source_ref(path, &["scripts", &task]),
            "package_script",
            &task,
            &["javascript", "typescript"],
        ) {
            candidates.push(candidate);
        }
    }
    candidates
}

/// .cargo/config.toml aliases (reference `_cargo_candidates`).
fn cargo_candidates(path: &str, payload: &Value) -> Vec<ValidationPlanCandidate> {
    let mut candidates: Vec<ValidationPlanCandidate> = Vec::new();
    for task in sorted_keys(payload.get("alias")) {
        if let Some(candidate) = declared_task_candidate(
            &source_ref(path, &["alias", &task]),
            "cargo_alias",
            &task,
            &["rust"],
        ) {
            candidates.push(candidate);
        }
    }
    candidates
}

/// toml::Value → serde_json::Value (manifest payloads carry content as
/// strings; the TOML crate does the parsing, JSON is the exchange shape).
fn toml_to_json(value: &toml::Value) -> Value {
    match value {
        toml::Value::String(text) => Value::String(text.clone()),
        toml::Value::Integer(number) => Value::Number((*number).into()),
        toml::Value::Float(number) => {
            serde_json::Number::from_f64(*number).map_or(Value::Null, Value::Number)
        }
        toml::Value::Boolean(flag) => Value::Bool(*flag),
        toml::Value::Datetime(datetime) => Value::String(datetime.to_string()),
        toml::Value::Array(items) => Value::Array(items.iter().map(toml_to_json).collect()),
        toml::Value::Table(table) => Value::Object(
            table
                .iter()
                .map(|(key, item)| (key.clone(), toml_to_json(item)))
                .collect(),
        ),
    }
}

/// Parse one manifest content string (reference `_read_manifest`).
fn read_manifest(path: &str, content: &str) -> (Option<Value>, Option<&'static str>) {
    let parsed = if path.trim_end().ends_with("package.json") {
        serde_json::from_str::<Value>(content).ok()
    } else {
        toml::from_str::<toml::Value>(content)
            .ok()
            .map(|value| toml_to_json(&value))
    };
    match parsed {
        Some(Value::Object(_)) => (parsed, None),
        Some(_) => (None, Some("manifest_root_not_object")),
        None => (None, Some("manifest_unreadable")),
    }
}

/// Reference `_is_non_oracle_manifest`: fixture/vendor directories never
/// contribute oracle tasks.
fn is_non_oracle_manifest(relative: &str) -> bool {
    relative.split('/').any(|part| {
        let lower = part.to_lowercase();
        NON_ORACLE_PATH_PARTS.contains(&lower.as_str())
    })
}

fn category_rank(category: &str) -> usize {
    VALIDATION_CATEGORIES
        .iter()
        .position(|item| *item == category)
        .unwrap_or(VALIDATION_CATEGORIES.len())
}

/// Discover repository-declared quality tasks from manifest payloads
/// (`[{"path": "pyproject.toml", "content": "…"}]`) without copying task
/// bodies (reference `build_change_quality_validation_plan`).
pub fn build_change_quality_validation_plan(
    manifests: &[Value],
    instruction_refs: &[String],
) -> Value {
    let mut candidates: Vec<ValidationPlanCandidate> = Vec::new();
    let mut warnings: Vec<Value> = Vec::new();
    let mut ignored_manifest_refs: Vec<String> = Vec::new();
    for manifest in manifests {
        let Some(path) = manifest.get("path").and_then(Value::as_str) else {
            continue;
        };
        let Some(content) = manifest.get("content").and_then(Value::as_str) else {
            warnings.push(serde_json::json!({"source_ref": path, "code": "manifest_unreadable"}));
            continue;
        };
        let relative = path.replace('\\', "/");
        if relative.starts_with('/') || relative.split('/').any(|part| part == "..") {
            continue;
        }
        if is_non_oracle_manifest(&relative) {
            ignored_manifest_refs.push(path.to_string());
            continue;
        }
        let parser = if relative.ends_with("pyproject.toml") {
            Some(python_candidates as fn(&str, &Value) -> Vec<ValidationPlanCandidate>)
        } else if relative.ends_with("package.json") {
            Some(node_candidates as fn(&str, &Value) -> Vec<ValidationPlanCandidate>)
        } else if relative.ends_with(".cargo/config.toml") {
            Some(cargo_candidates as fn(&str, &Value) -> Vec<ValidationPlanCandidate>)
        } else {
            None
        };
        let Some(parser) = parser else {
            continue;
        };
        let (payload, warning) = read_manifest(path, content);
        if let Some(code) = warning {
            warnings.push(serde_json::json!({"source_ref": path, "code": code}));
            continue;
        }
        candidates.extend(parser(
            path,
            payload.as_ref().expect("read_manifest returned a payload"),
        ));
    }
    // Reference order: truncate at TASK_LIMIT, then dedupe by oracle id.
    let mut ordered: Vec<ValidationPlanCandidate> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for candidate in candidates.into_iter().take(TASK_LIMIT) {
        if seen.insert(candidate.oracle_id.clone()) {
            ordered.push(candidate);
        }
    }
    ordered.sort_by(|left, right| {
        category_rank(&left.category)
            .cmp(&category_rank(&right.category))
            .then_with(|| left.source_ref.cmp(&right.source_ref))
    });
    let covered: BTreeSet<&str> = ordered
        .iter()
        .map(|candidate| candidate.category.as_str())
        .collect();
    let unresolved_categories: Vec<&str> = VALIDATION_CATEGORIES
        .iter()
        .copied()
        .filter(|category| !covered.contains(category))
        .collect();
    let reads: BTreeSet<&str> = instruction_refs.iter().map(String::as_str).collect();
    let required_reads: Vec<&str> = reads.into_iter().collect();
    serde_json::json!({
        "schema_version": CHANGE_QUALITY_VALIDATION_PLAN_SCHEMA_VERSION,
        "selection_policy": "repository_declared_only",
        "required_reads": required_reads,
        "candidates": ordered.iter().map(ValidationPlanCandidate::to_value).collect::<Vec<_>>(),
        "unresolved_categories": unresolved_categories,
        "ignored_manifest_refs": ignored_manifest_refs,
        "discovery_warnings": warnings,
        "auto_execute": false,
        "task_bodies_included": false,
    })
}

// ── scope identity (reference scope.py — host-supplied git outputs) ───────

/// Host-supplied git outputs for one exact-scope capture. The capability is
/// a pure deterministic rule version: the agent supplies `git diff` payloads
/// and file lists; this module computes the identity, it never shells out.
#[derive(Debug, Clone)]
pub struct ChangeScopeInput<'a> {
    pub base_ref: &'a str,
    pub base_commit: &'a str,
    pub head_commit: &'a str,
    pub committed: &'a [String],
    pub staged: &'a [String],
    pub unstaged: &'a [String],
    pub untracked: &'a [String],
    /// (label, content) pairs for the committed / staged / unstaged diffs.
    pub diff_texts: &'a [(&'a str, &'a str)],
}

/// Fold host-supplied git outputs into an exact-scope identity (reference
/// `build_change_quality_scope`): deduped changed-file list + a sha256
/// fingerprint over base/head commits, per-source diff payloads, and
/// untracked paths. Deviation from the reference: untracked file *contents*
/// are not hashed here (the capability never reads the filesystem) — the
/// host is expected to fold content hashes into the diff payloads it
/// supplies when exactness matters.
pub fn build_change_quality_scope(input: &ChangeScopeInput) -> Value {
    let mut changed_files: Vec<String> = Vec::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for source in [
        input.committed,
        input.staged,
        input.unstaged,
        input.untracked,
    ] {
        for file in source.iter() {
            if !file.is_empty() && seen.insert(file.as_str()) {
                changed_files.push(file.clone());
            }
        }
    }
    let mut hasher = Sha256::new();
    hasher.update(CHANGE_QUALITY_SCOPE_SCHEMA_VERSION.as_bytes());
    hasher.update([0u8]);
    hasher.update(input.base_commit.as_bytes());
    hasher.update([0u8]);
    hasher.update(input.head_commit.as_bytes());
    hasher.update([0u8]);
    for (label, text) in input.diff_texts {
        hasher.update(label.as_bytes());
        hasher.update([0u8]);
        hasher.update(text.as_bytes());
        hasher.update([0u8]);
    }
    for file in input.untracked.iter().filter(|file| !file.is_empty()) {
        hasher.update(file.as_bytes());
        hasher.update([0u8]);
    }
    let changed_file_count = changed_files.len();
    serde_json::json!({
        "schema_version": CHANGE_QUALITY_SCOPE_SCHEMA_VERSION,
        "base_ref": input.base_ref,
        "base_commit": input.base_commit,
        "head_commit": input.head_commit,
        "scope_fingerprint": format!("{:x}", hasher.finalize()),
        "changed_files": changed_files,
        "changed_file_count": changed_file_count,
        "sources": {
            "committed": input.committed,
            "staged": input.staged,
            "unstaged": input.unstaged,
            "untracked": input.untracked,
        },
    })
}

// ── free-text change classification (quality signal extraction) ───────────

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidationSignal {
    pub validator: String,
    /// passed | failed | skipped. A validator line with no status line
    /// defaults to skipped: nothing was proven either way.
    pub status: String,
    pub required: bool,
    pub command: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RiskSignal {
    pub category: &'static str,
    pub severity: &'static str,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeClassification {
    pub has_artifact: bool,
    pub tests_pass: bool,
    pub tests_fail: bool,
    pub validators: Vec<ValidationSignal>,
    pub changed_paths: Vec<String>,
    pub risk_signals: Vec<RiskSignal>,
}

/// Case-insensitive key match at line start; returns the original-case
/// remainder (keys are ASCII, so byte offsets match).
fn line_value<'a>(line: &'a str, keys: &[&str]) -> Option<&'a str> {
    for key in keys {
        if line.len() >= key.len()
            && line.is_char_boundary(key.len())
            && line[..key.len()].eq_ignore_ascii_case(key)
        {
            return Some(line[key.len()..].trim());
        }
    }
    None
}

fn parse_status(value: &str) -> &'static str {
    let lower = value.to_lowercase();
    if lower.contains("fail") || lower.contains("error") {
        VALIDATION_STATUS_FAILED
    } else if lower.contains("skip") {
        VALIDATION_STATUS_SKIPPED
    } else if lower.contains("pass") || lower.contains("ok") {
        VALIDATION_STATUS_PASSED
    } else {
        VALIDATION_STATUS_SKIPPED
    }
}

/// Keyword risk signals mapped onto guardrail lenses (deterministic rule
/// version of what a reviewer would flag; the structured result path is the
/// authoritative contract).
fn risk_keyword_signals(lower: &str) -> Vec<RiskSignal> {
    let mut signals: Vec<RiskSignal> = Vec::new();
    let mut push = |category: &'static str,
                    severity: &'static str,
                    code: &str,
                    message: &str,
                    needles: &[&str]| {
        if needles.iter().any(|needle| lower.contains(needle)) {
            signals.push(RiskSignal {
                category,
                severity,
                code: code.to_string(),
                message: message.to_string(),
            });
        }
    };
    push(
        "security_release",
        "blocker",
        "security_credential_exposure",
        "credential or secret material appears in the change evidence",
        &["secret", "credential", "api key"],
    );
    push(
        "security_release",
        "warning",
        "security_release_migration_risk",
        "migration or rollback compatibility is unproven",
        &["migration", "rollback"],
    );
    push(
        "error_supervision",
        "blocker",
        "error_supervision_unhandled_failure",
        "an unhandled failure mode is reported",
        &["panic", "crash", "unhandled"],
    );
    push(
        "error_supervision",
        "warning",
        "error_supervision_silent_fallback",
        "a failure may be silently swallowed",
        &["silent", "swallow"],
    );
    push(
        "reuse",
        "warning",
        "reuse_duplication",
        "the change may duplicate an established helper",
        &["duplicat", "copy-paste", "reinvent"],
    );
    push(
        "quality_simplification",
        "advisory",
        "simplification_indirection",
        "the change may add indirection or speculative abstraction",
        &["indirection", "over-abstract", "speculative"],
    );
    push(
        "quality_simplification",
        "advisory",
        "simplification_dead_code",
        "dead or unreachable code is reported",
        &["dead code", "unreachable"],
    );
    push(
        "type_api_boundary",
        "blocker",
        "type_api_boundary_break",
        "a caller-facing contract break is reported",
        &["api break", "schema break", "incompatib"],
    );
    push(
        "type_api_boundary",
        "warning",
        "type_api_boundary_contract_unclear",
        "the caller-facing contract is not made explicit",
        &["contract"],
    );
    push(
        "configuration",
        "warning",
        "configuration_mode_coupling",
        "configuration may be hardcoded or mode-coupled",
        &["hardcod", "hidden mode", "config drift"],
    );
    push(
        "runtime_ownership",
        "blocker",
        "runtime_ownership_race",
        "a concurrency race is reported",
        &["race condition", "data race", "concurren"],
    );
    push(
        "runtime_ownership",
        "warning",
        "runtime_ownership_resource_leak",
        "a resource leak is reported",
        &["leak"],
    );
    push(
        "efficiency",
        "warning",
        "efficiency_unbounded_growth",
        "unbounded growth or hot-path cost is reported",
        &["unbounded", "quadratic", "hot path"],
    );
    push(
        "test_validation",
        "warning",
        "test_validation_flaky",
        "a flaky validation is reported",
        &["flaky"],
    );
    push(
        "test_validation",
        "advisory",
        "test_validation_coverage_gap",
        "validation coverage is incomplete",
        &["untested", "coverage gap", "no tests"],
    );
    push(
        "documentation_comments",
        "advisory",
        "documentation_stale",
        "documentation may be stale or duplicated",
        &["stale comment", "outdated doc"],
    );
    signals
}

/// Classify free-text change evidence into structured quality signals:
/// line-level `path:` / `validator:` / `status:` / `exit code:` /
/// `command:` / `reason:` fields plus keyword pass/fail/artifact markers
/// and risk keywords mapped onto guardrail lenses.
pub fn classify_change_input(text: &str) -> ChangeClassification {
    let mut classification = ChangeClassification::default();
    let lower = text.to_lowercase();
    classification.has_artifact = [
        "diff",
        "patch",
        "written",
        "artifact",
        "changed files",
        "commit",
    ]
    .iter()
    .any(|marker| lower.contains(marker));
    classification.tests_pass = ["tests pass", "test pass", "all pass", "exit 0", "passes"]
        .iter()
        .any(|marker| lower.contains(marker));
    classification.tests_fail = [
        "tests fail",
        "test fail",
        "failing",
        "exit 1",
        "nonzero",
        "failure",
    ]
    .iter()
    .any(|marker| lower.contains(marker));
    classification.risk_signals = risk_keyword_signals(&lower);

    let mut current_validator: Option<ValidationSignal> = None;
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if let Some(value) = line_value(line, &["path:", "file:", "changed:"]) {
            if !value.is_empty() {
                classification.changed_paths.push(value.to_string());
            }
            continue;
        }
        if let Some(value) = line_value(line, &["validator:", "validation:", "check:"]) {
            if let Some(previous) = current_validator.take() {
                classification.validators.push(previous);
            }
            if !value.is_empty() {
                current_validator = Some(ValidationSignal {
                    validator: value.to_string(),
                    status: VALIDATION_STATUS_SKIPPED.to_string(),
                    required: true,
                    command: None,
                    reason: None,
                });
            }
            continue;
        }
        if let Some(value) = line_value(line, &["status:", "result:"]) {
            let status = parse_status(value);
            if let Some(validator) = current_validator.as_mut() {
                validator.status = status.to_string();
            }
            continue;
        }
        if let Some(value) = line_value(line, &["exit:", "exit_code:", "exit code:"]) {
            let status = if value.trim() == "0" {
                VALIDATION_STATUS_PASSED
            } else {
                VALIDATION_STATUS_FAILED
            };
            if let Some(validator) = current_validator.as_mut() {
                validator.status = status.to_string();
            }
            continue;
        }
        if let Some(value) = line_value(line, &["command:", "cmd:", "run:"]) {
            if let Some(validator) = current_validator.as_mut() {
                validator.command = Some(value.to_string());
            }
            continue;
        }
        if let Some(value) = line_value(line, &["reason:", "why:"]) {
            if let Some(validator) = current_validator.as_mut() {
                validator.reason = Some(value.to_string());
            }
        }
    }
    if let Some(previous) = current_validator.take() {
        classification.validators.push(previous);
    }
    for validator in &classification.validators {
        match validator.status.as_str() {
            VALIDATION_STATUS_PASSED => classification.tests_pass = true,
            VALIDATION_STATUS_FAILED => classification.tests_fail = true,
            _ => {}
        }
    }
    let mut seen: BTreeSet<String> = BTreeSet::new();
    classification
        .changed_paths
        .retain(|path| seen.insert(path.clone()));
    classification
}

// ── diff classification (Wave 2 contract: ChangeKind + signals + risk) ──

/// The finite set of change kinds a unified diff can classify as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ChangeKind {
    /// New file — only additions.
    Addition,
    /// Removed file — only deletions.
    Deletion,
    /// Renamed with content edits (`rename from/to` + hunks).
    Rename,
    /// Renamed with identical content (`similarity index 100%`).
    Move,
    /// Modified lines inside existing content.
    BehaviorChange,
}

impl ChangeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChangeKind::Addition => "addition",
            ChangeKind::Deletion => "deletion",
            ChangeKind::Rename => "rename",
            ChangeKind::Move => "move",
            ChangeKind::BehaviorChange => "behavior_change",
        }
    }
}

/// Classify a unified-diff text payload into a deterministic, sorted set of
/// change kinds. Markers take precedence (file-mode / rename headers);
/// otherwise hunks with mixed `+`/`-` content are behavior changes, and
/// pure additions/deletions fall back to line-shape classification.
pub fn classify(text_diff: &str) -> Vec<ChangeKind> {
    let lower = text_diff.to_lowercase();
    let has_hunk = text_diff.lines().any(|line| line.starts_with("@@ "));
    let added = text_diff
        .lines()
        .filter(|line| line.starts_with('+') && !line.starts_with("+++ "))
        .count();
    let removed = text_diff
        .lines()
        .filter(|line| line.starts_with('-') && !line.starts_with("--- "))
        .count();

    let mut kinds = BTreeSet::new();
    if lower.contains("new file mode") {
        kinds.insert(ChangeKind::Addition);
    }
    if lower.contains("deleted file mode") {
        kinds.insert(ChangeKind::Deletion);
    }
    if lower.contains("rename from") || lower.contains("rename to") {
        let pure_move = lower.contains("similarity index 100%") || !has_hunk;
        kinds.insert(if pure_move {
            ChangeKind::Move
        } else {
            ChangeKind::Rename
        });
    }
    if has_hunk && added > 0 && removed > 0 {
        kinds.insert(ChangeKind::BehaviorChange);
    }
    if kinds.is_empty() {
        if added > 0 && removed == 0 {
            kinds.insert(ChangeKind::Addition);
        } else if removed > 0 && added == 0 {
            kinds.insert(ChangeKind::Deletion);
        } else if has_hunk {
            kinds.insert(ChangeKind::BehaviorChange);
        }
    }
    kinds.into_iter().collect()
}

/// Bounded quality signals derived from a diff plus a recent-change history
/// keyed by path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QualitySignals {
    /// Total content lines changed (excluding `+++`/`---` headers).
    pub churn: usize,
    /// Number of `@@` hunks — how spread out the change is.
    pub hunk_dispersion: usize,
    /// Sum of recent-change counts for every path this diff touches.
    pub repeated_change_count: usize,
    /// Repo-relative paths touched, deduped and sorted.
    pub touched_paths: Vec<String>,
}

/// Compute quality signals from a unified diff; `recent_path_counts` maps a
/// repo-relative path to how many recent changes already touched it (the
/// "recent N" window is the caller's responsibility).
pub fn quality_signals(
    text_diff: &str,
    recent_path_counts: &BTreeMap<String, usize>,
) -> QualitySignals {
    let churn = text_diff
        .lines()
        .filter(|line| {
            (line.starts_with('+') || line.starts_with('-'))
                && !line.starts_with("+++ ")
                && !line.starts_with("--- ")
        })
        .count();
    let hunk_dispersion = text_diff
        .lines()
        .filter(|line| line.starts_with("@@ "))
        .count();
    let touched_paths = touched_paths(text_diff);
    let repeated_change_count = touched_paths
        .iter()
        .map(|path| recent_path_counts.get(path).copied().unwrap_or(0))
        .sum();
    QualitySignals {
        churn,
        hunk_dispersion,
        repeated_change_count,
        touched_paths,
    }
}

/// Repo-relative paths referenced by `diff --git` / `+++` / `---` headers.
fn touched_paths(text_diff: &str) -> Vec<String> {
    let mut paths = BTreeSet::new();
    for line in text_diff.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            if let Some(first) = rest.split_whitespace().next() {
                paths.insert(first.trim_start_matches("a/").to_string());
            }
        }
        for prefix in ["+++ ", "--- "] {
            if let Some(rest) = line.strip_prefix(prefix) {
                let candidate = rest.trim();
                if candidate != "/dev/null" && !candidate.is_empty() {
                    paths.insert(
                        candidate
                            .trim_start_matches("b/")
                            .trim_start_matches("a/")
                            .to_string(),
                    );
                }
            }
        }
    }
    paths.into_iter().collect()
}

/// Risk score threshold above which a diff proposes repair instead of review.
pub const RISK_REPAIR_THRESHOLD: u8 = 70;

/// A bounded 0–100 risk score plus the factors that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RiskAssessment {
    pub score: u8,
    pub reasons: Vec<String>,
}

/// Score quality signals on 0–100 with per-factor reasons: churn contributes
/// at most 40, hunk dispersion at most 30, repeated recent changes at most
/// 30 — so the sum is always within bounds.
pub fn risk_assessment(signals: &QualitySignals) -> RiskAssessment {
    let mut score: u32 = 0;
    let mut reasons = Vec::new();

    if signals.churn > 0 {
        let churn_score = (5 + signals.churn / 10).min(40) as u32;
        score += churn_score;
        reasons.push(format!(
            "churn of {} changed lines (+{})",
            signals.churn, churn_score
        ));
    }
    if signals.hunk_dispersion > 0 {
        let dispersion_score = (signals.hunk_dispersion * 2).min(30) as u32;
        score += dispersion_score;
        reasons.push(format!(
            "{} scattered hunks (+{})",
            signals.hunk_dispersion, dispersion_score
        ));
    }
    if signals.repeated_change_count > 0 {
        let repeat_score = (signals.repeated_change_count * 10).min(30) as u32;
        score += repeat_score;
        reasons.push(format!(
            "{} recent changes touch the same paths (+{})",
            signals.repeated_change_count, repeat_score
        ));
    }
    RiskAssessment {
        score: score.min(100) as u8,
        reasons,
    }
}

/// Convenience: the bare 0–100 risk score.
pub fn risk_score(signals: &QualitySignals) -> u8 {
    risk_assessment(signals).score
}

/// Whether free text looks like a unified diff rather than prose evidence.
fn looks_like_diff(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with("diff --git")
        || trimmed.starts_with("--- ")
        || text.lines().any(|line| line.starts_with("@@ "))
        || (text.lines().any(|line| line.starts_with("+++ "))
            && text.lines().any(|line| line.starts_with("--- ")))
}

// ── capability ────────────────────────────────────────────────────────────

pub struct ChangeQualityCapability;

impl ChangeQualityCapability {
    /// A JSON change-quality payload, when present.
    fn payload(input: &str) -> Option<Value> {
        let value: Value = serde_json::from_str(input.trim()).ok()?;
        value.as_object()?;
        Some(value)
    }

    fn string_list(values: Option<&Value>) -> Vec<String> {
        values
            .and_then(Value::as_array)
            .map(|array| {
                array
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn payload_policy(payload: &Value) -> ChangeQualityPolicy {
        if let Some(goal) = payload.get("goal") {
            return change_quality_policy(goal);
        }
        if let Some(policy) = payload.get("policy").and_then(Value::as_object) {
            return ChangeQualityPolicy::from_qualification(Some(policy));
        }
        ChangeQualityPolicy::default()
    }

    /// Qualification path: `{"goal_id", "result", "scope_fingerprint" |
    /// "scope": {…}, optional "policy"/"goal", "instruction_refs"}`.
    /// Normalize the agent result against the exact scope, derive the
    /// guardrails, and propose acceptance packaging or a bounded repair.
    fn qualification_proposals(payload: &Value) -> Vec<TypedProposal> {
        let Some(result) = payload.get("result") else {
            return vec![TypedProposal::gate(
                "Provide a change-quality qualification payload: {\"goal_id\": …, \"result\": {…}, \"scope_fingerprint\": …} plus optional policy/scope/instruction_refs.",
                "qualification payload requires a result object",
            )];
        };
        let Some(fingerprint) = payload
            .get("scope_fingerprint")
            .and_then(Value::as_str)
            .or_else(|| {
                payload
                    .get("scope")
                    .and_then(|scope| scope.get("scope_fingerprint"))
                    .and_then(Value::as_str)
            })
            .filter(|fingerprint| !fingerprint.is_empty())
        else {
            return vec![TypedProposal::gate(
                "Provide the exact-scope fingerprint (scope_fingerprint) the result was reviewed against.",
                "qualification requires the exact scope fingerprint",
            )];
        };
        let policy = Self::payload_policy(payload);
        let changed_files = payload
            .get("scope")
            .and_then(|scope| scope.get("changed_files"))
            .map(|files| Self::string_list(Some(files)));
        let instruction_refs = payload
            .get("instruction_refs")
            .map(|refs| Self::string_list(Some(refs)))
            .or_else(|| {
                payload
                    .get("scope")
                    .and_then(|scope| scope.get("repository_context"))
                    .and_then(|context| context.get("instruction_refs"))
                    .map(|refs| Self::string_list(Some(refs)))
            });
        match normalize_change_quality_result(
            result,
            fingerprint,
            policy.safe_fix,
            changed_files.as_deref(),
            instruction_refs.as_deref(),
        ) {
            Err(err) => vec![TypedProposal::gate(
                &format!("Change-quality result rejected: {err}. Fix the result JSON before recording a receipt."),
                "change-quality result rejected by the result contract",
            )],
            Ok(normalized) => {
                let (decision, blockers) = change_quality_result_decision(&normalized);
                if decision == "pass" {
                    let mut todo = successor_todo(
                        "quality",
                        &format!(
                            "Package the validated change for the review/merge surface with the evidence packet; all guardrails pass for the exact scope (fingerprint {fingerprint})."
                        ),
                    );
                    todo.action_kind = Some("package_validated_change".to_string());
                    todo.required_capability = Some("change_quality".to_string());
                    todo.capability_binding_ref = Some("change_quality".to_string());
                    vec![TypedProposal::successor(
                        todo,
                        "change validated: all guardrails pass",
                    )]
                } else {
                    let mut todo = successor_todo(
                        "quality",
                        &format!(
                            "Repair: fix the blocking change-quality codes ({}), then re-review the exact scope and record a new change_quality_agent_result_v2.",
                            blockers.join(", ")
                        ),
                    );
                    todo.action_kind = Some("repair_change_quality_blockers".to_string());
                    todo.required_capability = Some("change_quality".to_string());
                    todo.capability_binding_ref = Some("change_quality".to_string());
                    vec![TypedProposal::successor(
                        todo,
                        &format!(
                            "qualification failed with {} blocking code(s): {}",
                            blockers.len(),
                            blockers.join(", ")
                        ),
                    )]
                }
            }
        }
    }

    /// Prepare path: `{"goal_id", "scope": {…}, "manifests":
    /// [{"path", "content"}…], optional "instruction_refs"}`. Discover the
    /// repository-declared validation plan and propose the exact-scope
    /// review.
    fn prepare_proposals(payload: &Value) -> Vec<TypedProposal> {
        let scope = payload.get("scope").cloned().unwrap_or(Value::Null);
        let fingerprint = scope
            .get("scope_fingerprint")
            .and_then(Value::as_str)
            .unwrap_or("(unset)");
        let instruction_refs = Self::string_list(payload.get("instruction_refs"));
        let manifests: Vec<Value> = payload
            .get("manifests")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let plan = build_change_quality_validation_plan(&manifests, &instruction_refs);
        let candidate_count = plan
            .get("candidates")
            .and_then(Value::as_array)
            .map_or(0, |candidates| candidates.len());
        let unresolved = plan
            .get("unresolved_categories")
            .and_then(Value::as_array)
            .map_or(0, |categories| categories.len());
        let by_category: Vec<String> = VALIDATION_CATEGORIES
            .iter()
            .map(|category| {
                let count =
                    plan.get("candidates")
                        .and_then(Value::as_array)
                        .map_or(0, |candidates| {
                            candidates
                                .iter()
                                .filter(|candidate| {
                                    candidate.get("category").and_then(Value::as_str)
                                        == Some(category)
                                })
                                .count()
                        });
                format!("{category}={count}")
            })
            .collect();
        let mut todo = successor_todo(
            "quality",
            &format!(
                "Review the exact changed scope (fingerprint {fingerprint}) against the validation plan (oracles: {}): resolve the projected instruction and ownership references, run the discovered repository oracles, then record a grounded change_quality_agent_result_v2 with reuse/simplification conclusions, sparse risks[], and validation[] entries.",
                by_category.join(" ")
            ),
        );
        todo.action_kind = Some("run_change_quality_validation".to_string());
        todo.required_capability = Some("change_quality".to_string());
        todo.capability_binding_ref = Some("change_quality".to_string());
        vec![TypedProposal::successor(
            todo,
            &format!(
                "validation plan discovered {candidate_count} repository oracles ({unresolved} categories unresolved) — review the exact scope"
            ),
        )]
    }
}

impl Capability for ChangeQualityCapability {
    fn name(&self) -> &'static str {
        "change_quality"
    }
    fn describe(&self) -> &'static str {
        "classify a change's validation evidence, discover repository-declared quality oracles, normalize agent results into risk-scored guardrails, and propose repair or acceptance"
    }

    fn propose(&self, input: &str) -> Vec<TypedProposal> {
        let text = input.trim();
        if text.is_empty() {
            return vec![TypedProposal::no_followup("no change evidence provided")];
        }

        // Payload path: qualification (result) or prepare (scope/manifests).
        if let Some(payload) = Self::payload(text) {
            if payload.get("result").is_some() {
                return Self::qualification_proposals(&payload);
            }
            if payload.get("scope").is_some() || payload.get("manifests").is_some() {
                return Self::prepare_proposals(&payload);
            }
            return vec![TypedProposal::gate(
                "Provide a change-quality payload: {\"goal_id\", \"result\", \"scope_fingerprint\"} for qualification, or {\"goal_id\", \"scope\", \"manifests\"} for a prepare packet.",
                "change-quality payload shape not recognized",
            )];
        }

        // Free-text path: unified diffs classify into change kinds + quality
        // signals with a bounded risk score (repair above the threshold,
        // review below); other text flows through the evidence classifier.
        if looks_like_diff(text) {
            let kinds = classify(text);
            let signals = quality_signals(text, &BTreeMap::new());
            let risk = risk_assessment(&signals);
            if kinds.is_empty() {
                return vec![TypedProposal::gate(
                    "Provide a unified diff (diff --git headers or @@ hunks) or a change-quality payload.",
                    "diff shape not recognized",
                )];
            }
            let kind_summary = kinds
                .iter()
                .map(ChangeKind::as_str)
                .collect::<Vec<_>>()
                .join("+");
            if risk.score >= RISK_REPAIR_THRESHOLD {
                let mut todo = successor_todo(
                    "quality",
                    &format!(
                        "Repair: change classifies as {} with risk {}/100 ({}). Reduce churn, split hunks, or re-validate the repeatedly touched paths before acceptance.",
                        kind_summary,
                        risk.score,
                        risk.reasons.join("; ")
                    ),
                );
                todo.action_kind = Some("repair_risky_change".to_string());
                todo.required_capability = Some("change_quality".to_string());
                todo.capability_binding_ref = Some("change_quality".to_string());
                return vec![TypedProposal::successor(
                    todo,
                    "high change risk — bounded repair",
                )];
            }
            let mut todo = successor_todo(
                "quality",
                &format!(
                    "Review: change classifies as {} with risk {}/100 (churn {}, {} hunks, {} repeated touches) — attach validation evidence, then merge.",
                    kind_summary,
                    risk.score,
                    signals.churn,
                    signals.hunk_dispersion,
                    signals.repeated_change_count
                ),
            );
            todo.action_kind = Some("review_qualified_change".to_string());
            todo.required_capability = Some("change_quality".to_string());
            todo.capability_binding_ref = Some("change_quality".to_string());
            return vec![TypedProposal::successor(
                todo,
                "low-risk change — review suggestion",
            )];
        }

        // Evidence-text path: classify the change evidence, then a finite
        // validated / repair / evidence-thin proposal.
        let classification = classify_change_input(text);
        if classification.tests_fail {
            let mut todo = successor_todo(
                "quality",
                "Repair: run the validation again and fix the failing assertion before reporting success.",
            );
            todo.action_kind = Some("repair_failing_validation".to_string());
            todo.required_capability = Some("change_quality".to_string());
            todo.capability_binding_ref = Some("change_quality".to_string());
            vec![TypedProposal::successor(
                todo,
                "validation evidence is missing or failing",
            )]
        } else if classification.tests_pass && classification.has_artifact {
            let mut todo = successor_todo(
                "quality",
                "Package the validated change for the review/merge surface with the evidence packet.",
            );
            todo.action_kind = Some("package_validated_change".to_string());
            todo.required_capability = Some("change_quality".to_string());
            todo.capability_binding_ref = Some("change_quality".to_string());
            vec![TypedProposal::successor(
                todo,
                "change validated (tests pass + artifact present)",
            )]
        } else {
            let mut todo = successor_todo(
                "quality",
                "Record concrete change evidence (paths, diffs, test output) before acceptance.",
            );
            todo.action_kind = Some("record_change_evidence".to_string());
            todo.required_capability = Some("change_quality".to_string());
            todo.capability_binding_ref = Some("change_quality".to_string());
            vec![TypedProposal::successor(todo, "artifact evidence is thin")]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::ProposalKind;

    fn goal_with(qualification: Value) -> Value {
        serde_json::json!({
            "control_plane": { "change_quality_qualification": qualification }
        })
    }

    fn valid_result() -> Value {
        serde_json::json!({
            "schema_version": CHANGE_QUALITY_RESULT_SCHEMA_VERSION,
            "scope_fingerprint": "fp_1",
            "reviewed_final_scope": true,
            "reuse": {
                "outcome": "reused",
                "summary": "reuses the existing helper",
                "evidence_refs": ["path:src/lib.rs"]
            },
            "simplification": {
                "outcome": "retained",
                "summary": "kept direct control flow",
                "evidence_refs": ["path:src/lib.rs"],
                "safe_fix_applied": false
            },
            "risks": [],
            "validation": [
                {
                    "validator": "cargo test",
                    "status": "passed",
                    "scope": "workspace",
                    "required": true
                }
            ]
        })
    }

    // ── policy ──

    #[test]
    fn policy_parses_goal_qualification_flags() {
        let goal = goal_with(serde_json::json!({
            "enabled": true,
            "safe_fix": true,
            "strict_receipt": false
        }));
        let policy = change_quality_policy(&goal);
        assert!(policy.enabled);
        assert!(policy.safe_fix);
        assert!(!policy.strict_receipt);
        // Missing flags default off (strict `is True` semantics).
        let policy = change_quality_policy(&goal_with(serde_json::json!({})));
        assert!(!policy.enabled && !policy.safe_fix && !policy.strict_receipt);
        // No control plane at all → disabled.
        assert!(!change_quality_policy(&serde_json::json!({})).enabled);
    }

    // ── result normalization ──

    #[test]
    fn result_normalization_accepts_grounded_result() {
        let normalized = normalize_change_quality_result(
            &valid_result(),
            "fp_1",
            false,
            Some(&["src/lib.rs".to_string()]),
            Some(&["CONTRIBUTING.md".to_string()]),
        )
        .expect("valid result");
        assert_eq!(normalized.reuse.outcome, "reused");
        assert_eq!(normalized.validation.len(), 1);
        assert!(!normalized.safe_fix_applied);
    }

    #[test]
    fn result_normalization_rejects_fingerprint_mismatch() {
        let err = normalize_change_quality_result(&valid_result(), "other_fp", false, None, None)
            .unwrap_err();
        assert!(err.contains("scope_fingerprint"), "{err}");
    }

    #[test]
    fn result_normalization_rejects_unknown_evidence_targets() {
        // path evidence must name a changed file; empty expectation → reject.
        let err = normalize_change_quality_result(&valid_result(), "fp_1", false, None, None)
            .unwrap_err();
        assert!(err.contains("unknown evidence"), "{err}");
    }

    #[test]
    fn result_normalization_rejects_unsupported_fields() {
        let mut forged = valid_result();
        forged["extra_opinion"] = Value::Bool(true);
        let err = normalize_change_quality_result(&forged, "fp_1", false, None, None).unwrap_err();
        assert!(err.contains("unsupported fields"), "{err}");
    }

    #[test]
    fn result_normalization_requires_reviewed_final_scope() {
        let mut forged = valid_result();
        forged["reviewed_final_scope"] = Value::Bool(false);
        let err = normalize_change_quality_result(&forged, "fp_1", false, None, None).unwrap_err();
        assert!(err.contains("reviewed_final_scope"), "{err}");
    }

    #[test]
    fn simplification_safe_fix_must_agree_with_policy_and_outcome() {
        let mut forged = valid_result();
        forged["simplification"] = serde_json::json!({
            "outcome": "fixed",
            "summary": "simplified",
            "evidence_refs": ["path:src/lib.rs"],
            "safe_fix_applied": true
        });
        // safe_fix_applied without policy permission → reject.
        let err = normalize_change_quality_result(
            &forged,
            "fp_1",
            false,
            Some(&["src/lib.rs".to_string()]),
            None,
        )
        .unwrap_err();
        assert!(err.contains("safe_fix"), "{err}");
        // permitted, but outcome=fixed must agree with the flag.
        let normalized = normalize_change_quality_result(
            &forged,
            "fp_1",
            true,
            Some(&["src/lib.rs".to_string()]),
            None,
        )
        .expect("allowed safe fix");
        assert!(normalized.safe_fix_applied);
        let mut mismatch = forged.clone();
        mismatch["simplification"]["safe_fix_applied"] = Value::Bool(false);
        let err = normalize_change_quality_result(
            &mismatch,
            "fp_1",
            true,
            Some(&["src/lib.rs".to_string()]),
            None,
        )
        .unwrap_err();
        assert!(err.contains("must agree"), "{err}");
    }

    #[test]
    fn validation_fail_requires_reason_and_unique_ids() {
        let mut forged = valid_result();
        forged["validation"] = serde_json::json!([
            {"validator": "cargo test", "status": "failed", "scope": "workspace"}
        ]);
        let err = normalize_change_quality_result(
            &forged,
            "fp_1",
            false,
            Some(&["src/lib.rs".to_string()]),
            None,
        )
        .unwrap_err();
        assert!(err.contains("requires reason"), "{err}");
        let mut duplicate = valid_result();
        duplicate["validation"] = serde_json::json!([
            {"validator": "cargo test", "status": "passed", "scope": "a"},
            {"validator": "cargo test", "status": "passed", "scope": "b"}
        ]);
        let err = normalize_change_quality_result(
            &duplicate,
            "fp_1",
            false,
            Some(&["src/lib.rs".to_string()]),
            None,
        )
        .unwrap_err();
        assert!(err.contains("unique"), "{err}");
    }

    #[test]
    fn risk_contract_checks_category_severity_grounding_and_codes() {
        let mut forged = valid_result();
        forged["risks"] = serde_json::json!([{
            "category": "reuse",
            "severity": "warning",
            "code": "r1",
            "message": "duplication",
            "evidence_refs": ["path:src/lib.rs"]
        }]);
        // reuse is a primary lens, not a guardrail category.
        let err = normalize_change_quality_result(
            &forged,
            "fp_1",
            false,
            Some(&["src/lib.rs".to_string()]),
            None,
        )
        .unwrap_err();
        assert!(err.contains("category"), "{err}");
        let mut bad_severity = valid_result();
        bad_severity["risks"] = serde_json::json!([{
            "category": "efficiency",
            "severity": "catastrophic",
            "code": "r1",
            "message": "slow",
            "evidence_refs": ["path:src/lib.rs"]
        }]);
        let err = normalize_change_quality_result(
            &bad_severity,
            "fp_1",
            false,
            Some(&["src/lib.rs".to_string()]),
            None,
        )
        .unwrap_err();
        assert!(err.contains("severity"), "{err}");
        let mut duplicate_codes = valid_result();
        duplicate_codes["risks"] = serde_json::json!([
            {"category": "efficiency", "severity": "warning", "code": "r1", "message": "a", "evidence_refs": ["path:src/lib.rs"]},
            {"category": "test_validation", "severity": "warning", "code": "r1", "message": "b", "evidence_refs": ["path:src/lib.rs"]}
        ]);
        let err = normalize_change_quality_result(
            &duplicate_codes,
            "fp_1",
            false,
            Some(&["src/lib.rs".to_string()]),
            None,
        )
        .unwrap_err();
        assert!(err.contains("unique"), "{err}");
        // A risk path outside the changed set is rejected.
        let mut wrong_path = valid_result();
        wrong_path["risks"] = serde_json::json!([{
            "category": "efficiency",
            "severity": "warning",
            "code": "r1",
            "message": "slow",
            "evidence_refs": ["path:src/lib.rs"],
            "path": "other/mod.rs"
        }]);
        let err = normalize_change_quality_result(
            &wrong_path,
            "fp_1",
            false,
            Some(&["src/lib.rs".to_string()]),
            None,
        )
        .unwrap_err();
        assert!(err.contains("changed file"), "{err}");
    }

    #[test]
    fn bounded_text_rejects_private_material_and_oversize() {
        let err = bounded_text(
            &Value::String("/Users/geilige/secret".into()),
            "summary",
            400,
        )
        .unwrap_err();
        assert!(err.contains("private"), "{err}");
        let err = bounded_text(&Value::String("x".repeat(401)), "summary", 400).unwrap_err();
        assert!(err.contains("exceeds"), "{err}");
    }

    // ── guardrails + decision ──

    #[test]
    fn guardrails_derive_blocked_statuses_and_blocking_codes() {
        let mut result = valid_result();
        result["risks"] = serde_json::json!([{
            "category": "type_api_boundary",
            "severity": "blocker",
            "code": "contract_break",
            "message": "public API renamed",
            "evidence_refs": ["path:src/lib.rs"]
        }]);
        result["validation"] = serde_json::json!([
            {"validator": "cargo test", "status": "failed", "scope": "workspace", "required": true, "reason": "assertion"}
        ]);
        let normalized = normalize_change_quality_result(
            &result,
            "fp_1",
            false,
            Some(&["src/lib.rs".to_string()]),
            None,
        )
        .expect("grounded blocker result");
        let guardrails = derive_change_quality_guardrails(&normalized);
        let states = guardrails.get("states").and_then(Value::as_array).unwrap();
        let boundary = states
            .iter()
            .find(|state| {
                state.get("guardrail_id").and_then(Value::as_str) == Some("type_api_boundary")
            })
            .unwrap();
        assert_eq!(
            boundary.get("status").and_then(Value::as_str),
            Some("blocked")
        );
        let test_lens = states
            .iter()
            .find(|state| {
                state.get("guardrail_id").and_then(Value::as_str) == Some("test_validation")
            })
            .unwrap();
        assert_eq!(
            test_lens.get("status").and_then(Value::as_str),
            Some("blocked")
        );
        let blocking = guardrails
            .get("blocking_codes")
            .and_then(Value::as_array)
            .unwrap();
        assert!(blocking.iter().any(|code| code == "contract_break"));
        assert!(blocking.iter().any(|code| code == "validator:cargo test"));
        let (decision, blockers) = change_quality_result_decision(&normalized);
        assert_eq!(decision, "fail");
        assert_eq!(blockers.len(), 2);
    }

    #[test]
    fn guardrails_pass_decision_on_clean_result() {
        let normalized = normalize_change_quality_result(
            &valid_result(),
            "fp_1",
            false,
            Some(&["src/lib.rs".to_string()]),
            None,
        )
        .expect("clean result");
        let guardrails = derive_change_quality_guardrails(&normalized);
        let states = guardrails.get("states").and_then(Value::as_array).unwrap();
        let test_lens = states
            .iter()
            .find(|state| {
                state.get("guardrail_id").and_then(Value::as_str) == Some("test_validation")
            })
            .unwrap();
        assert_eq!(
            test_lens.get("status").and_then(Value::as_str),
            Some("satisfied")
        );
        assert_eq!(
            guardrails
                .get("blocking_codes")
                .and_then(Value::as_array)
                .unwrap()
                .len(),
            0
        );
        assert_eq!(change_quality_result_decision(&normalized).0, "pass");
    }

    #[test]
    fn guardrails_derive_resolved_and_risk_statuses() {
        let mut result = valid_result();
        result["risks"] = serde_json::json!([
            {"category": "efficiency", "severity": "blocker", "code": "e1", "message": "hot loop", "resolved": true, "evidence_refs": ["path:src/lib.rs"]},
            {"category": "documentation_comments", "severity": "advisory", "code": "d1", "message": "stale", "evidence_refs": ["path:src/lib.rs"]}
        ]);
        let normalized = normalize_change_quality_result(
            &result,
            "fp_1",
            false,
            Some(&["src/lib.rs".to_string()]),
            None,
        )
        .expect("grounded");
        let guardrails = derive_change_quality_guardrails(&normalized);
        let states = guardrails.get("states").and_then(Value::as_array).unwrap();
        let efficiency = states
            .iter()
            .find(|state| state.get("guardrail_id").and_then(Value::as_str) == Some("efficiency"))
            .unwrap();
        assert_eq!(
            efficiency.get("status").and_then(Value::as_str),
            Some("resolved")
        );
        let docs = states
            .iter()
            .find(|state| {
                state.get("guardrail_id").and_then(Value::as_str) == Some("documentation_comments")
            })
            .unwrap();
        assert_eq!(docs.get("status").and_then(Value::as_str), Some("risk"));
    }

    // ── validation plan oracles ──

    #[test]
    fn task_category_classifies_declared_task_names() {
        assert_eq!(task_category("cargo fmt"), Some("format"));
        assert_eq!(task_category("lint-clippy"), Some("lint"));
        assert_eq!(task_category("cargo test"), Some("test"));
        assert_eq!(task_category("pytest"), Some("test"));
        assert_eq!(task_category("type-check"), Some("typecheck"));
        assert_eq!(task_category("typechecking"), Some("typecheck"));
        assert_eq!(task_category("checktypes"), Some("typecheck"));
        assert_eq!(task_category("build"), None);
        assert_eq!(task_category(""), None);
    }

    #[test]
    fn oracle_ids_are_deterministic_and_task_sensitive() {
        let first = oracle_id("pyproject.toml#tool.poe.tasks.test", "poe_task", "test");
        let second = oracle_id("pyproject.toml#tool.poe.tasks.test", "poe_task", "test");
        let other = oracle_id("pyproject.toml#tool.poe.tasks.lint", "poe_task", "lint");
        assert_eq!(first, second);
        assert_ne!(first, other);
        assert!(first.starts_with("oracle_") && first.len() == "oracle_".len() + 16);
    }

    #[test]
    fn validation_plan_discovers_manifest_oracles_sorted_and_deduped() {
        let manifests = vec![
            serde_json::json!({
                "path": "pyproject.toml",
                "content": "[tool.poe.tasks]\ntest = \"pytest\"\nformat = \"ruff format\"\nlint = \"ruff\"\n"
            }),
            serde_json::json!({
                "path": "package.json",
                "content": "{\"scripts\": {\"test\": \"vitest\", \"build\": \"tsc\"}}"
            }),
            serde_json::json!({
                "path": ".cargo/config.toml",
                "content": "[alias]\ntest-all = \"check --workspace\"\n"
            }),
        ];
        let plan = build_change_quality_validation_plan(&manifests, &["AGENTS.md".to_string()]);
        assert_eq!(
            plan.get("schema_version").and_then(Value::as_str),
            Some(CHANGE_QUALITY_VALIDATION_PLAN_SCHEMA_VERSION)
        );
        let candidates = plan.get("candidates").and_then(Value::as_array).unwrap();
        // format, lint, test, test, test — sorted by category order; build dropped.
        assert_eq!(candidates.len(), 5);
        let categories: Vec<&str> = candidates
            .iter()
            .filter_map(|c| c.get("category").and_then(Value::as_str))
            .collect();
        assert_eq!(categories, ["format", "lint", "test", "test", "test"]);
        assert_eq!(
            plan.get("unresolved_categories")
                .and_then(Value::as_array)
                .unwrap()
                .len(),
            1 // typecheck
        );
        assert_eq!(
            plan.get("required_reads")
                .and_then(Value::as_array)
                .unwrap()
                .len(),
            1
        );
        assert!(!plan.get("auto_execute").and_then(Value::as_bool).unwrap());
        assert!(plan.get("task_bodies_included").is_some());
    }

    #[test]
    fn validation_plan_ignores_fixture_manifests_and_reports_unreadable() {
        let manifests = vec![
            serde_json::json!({"path": "testdata/pyproject.toml", "content": "[tool.poe.tasks]\ntest = \"x\"\n"}),
            serde_json::json!({"path": "node_modules/pkg/package.json", "content": "{}"}),
            serde_json::json!({"path": "pyproject.toml", "content": "not [valid toml"}),
        ];
        let plan = build_change_quality_validation_plan(&manifests, &[]);
        assert_eq!(
            plan.get("candidates")
                .and_then(Value::as_array)
                .unwrap()
                .len(),
            0
        );
        let ignored = plan
            .get("ignored_manifest_refs")
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(ignored.len(), 2);
        let warnings = plan
            .get("discovery_warnings")
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            warnings[0].get("code").and_then(Value::as_str),
            Some("manifest_unreadable")
        );
    }

    #[test]
    fn python_tool_config_candidates_use_fixed_categories() {
        let manifests = vec![serde_json::json!({
            "path": "pyproject.toml",
            "content": "[tool.mypy]\nstrict = true\n\n[tool.pytest.ini_options]\naddopts = \"-q\"\n"
        })];
        let plan = build_change_quality_validation_plan(&manifests, &[]);
        let candidates = plan.get("candidates").and_then(Value::as_array).unwrap();
        let by_runner: BTreeSet<&str> = candidates
            .iter()
            .filter_map(|c| c.get("runner").and_then(Value::as_str))
            .collect();
        assert!(by_runner.contains("mypy_config"));
        assert!(by_runner.contains("pytest_config"));
        let mypy = candidates
            .iter()
            .find(|c| c.get("runner").and_then(Value::as_str) == Some("mypy_config"))
            .unwrap();
        assert_eq!(
            mypy.get("category").and_then(Value::as_str),
            Some("typecheck")
        );
        assert_eq!(
            mypy.get("origin").and_then(Value::as_str),
            Some("repository_tool_config")
        );
    }

    #[test]
    fn unsafe_task_names_are_dropped() {
        let manifests = vec![serde_json::json!({
            "path": "package.json",
            "content": "{\"scripts\": {\"evil task!\": \"rm -rf\", \"ok-test\": \"vitest\"}}"
        })];
        let plan = build_change_quality_validation_plan(&manifests, &[]);
        let candidates = plan.get("candidates").and_then(Value::as_array).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].get("task").and_then(Value::as_str),
            Some("ok-test")
        );
    }

    // ── scope identity ──

    #[test]
    fn scope_fingerprint_is_deterministic_and_diff_sensitive() {
        let input = ChangeScopeInput {
            base_ref: "origin/main",
            base_commit: "abc",
            head_commit: "def",
            committed: &["src/a.rs".to_string()],
            staged: &["src/a.rs".to_string(), "src/b.rs".to_string()],
            unstaged: &[],
            untracked: &["notes.txt".to_string()],
            diff_texts: &[("committed", "+a"), ("staged", "+b")],
        };
        let scope = build_change_quality_scope(&input);
        let fingerprint = scope
            .get("scope_fingerprint")
            .and_then(Value::as_str)
            .unwrap();
        assert_eq!(
            scope
                .get("changed_files")
                .and_then(Value::as_array)
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            scope.get("changed_file_count").and_then(Value::as_u64),
            Some(3)
        );
        // Same input → same identity.
        let again = build_change_quality_scope(&input);
        assert_eq!(
            fingerprint,
            again
                .get("scope_fingerprint")
                .and_then(Value::as_str)
                .unwrap()
        );
        // A different diff → different identity.
        let mut changed = input.clone();
        changed.diff_texts = &[("committed", "+a"), ("staged", "+b2")];
        assert_ne!(
            fingerprint,
            build_change_quality_scope(&changed)
                .get("scope_fingerprint")
                .and_then(Value::as_str)
                .unwrap()
        );
    }

    // ── classification ──

    #[test]
    fn classify_parses_evidence_lines_and_validator_statuses() {
        let classification = classify_change_input(
            "path: src/lib.rs\npath: src/main.rs\npath: src/lib.rs\nvalidator: cargo test\nstatus: passed\nexit_code: 0\nvalidator: cargo clippy\nresult: FAILED\nreason: warnings\n",
        );
        assert_eq!(classification.changed_paths, ["src/lib.rs", "src/main.rs"]);
        assert_eq!(classification.validators.len(), 2);
        assert_eq!(classification.validators[0].status, "passed");
        assert_eq!(classification.validators[1].status, "failed");
        assert!(classification.validators[1].reason.is_some());
        assert!(classification.tests_pass);
        assert!(classification.tests_fail);
    }

    #[test]
    fn classify_maps_risk_keywords_to_guardrail_lenses() {
        let classification = classify_change_input(
            "the refactor panics under concurrency and duplicates an existing helper",
        );
        let codes: Vec<&str> = classification
            .risk_signals
            .iter()
            .map(|signal| signal.code.as_str())
            .collect();
        assert!(codes.contains(&"error_supervision_unhandled_failure"));
        assert!(codes.contains(&"runtime_ownership_race"));
        assert!(codes.contains(&"reuse_duplication"));
        assert!(!classification.tests_pass && !classification.tests_fail);
    }

    // ── propose ──

    #[test]
    fn propose_empty_is_no_followup() {
        let proposals = ChangeQualityCapability.propose("");
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].kind, ProposalKind::NoFollowUp);
    }

    #[test]
    fn propose_legacy_free_text_arms() {
        let thin = ChangeQualityCapability.propose("I made some edits");
        assert_eq!(thin[0].kind, ProposalKind::SuccessorTodo);
        assert!(thin[0].reason.contains("evidence"));
        let validated = ChangeQualityCapability.propose("all pass + diff written");
        assert_eq!(validated[0].kind, ProposalKind::SuccessorTodo);
        assert!(validated[0].reason.contains("validated"));
        let failing = ChangeQualityCapability.propose("tests fail");
        assert_eq!(failing[0].kind, ProposalKind::SuccessorTodo);
        assert!(failing[0].reason.contains("validation"));
        let thin_pass = ChangeQualityCapability.propose("all pass");
        assert!(thin_pass[0].reason.contains("thin"));
    }

    #[test]
    fn propose_qualification_accepts_a_grounded_result() {
        let input = serde_json::json!({
            "goal_id": "g1",
            "scope_fingerprint": "fp_1",
            "result": valid_result(),
            "scope": {"changed_files": ["src/lib.rs"]}
        })
        .to_string();
        let proposals = ChangeQualityCapability.propose(&input);
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].kind, ProposalKind::SuccessorTodo);
        let todo = proposals[0].todo.as_ref().unwrap();
        assert_eq!(
            todo.action_kind.as_deref(),
            Some("package_validated_change")
        );
        assert!(proposals[0].reason.contains("all guardrails pass"));
    }

    #[test]
    fn propose_qualification_proposes_repair_for_blockers() {
        let mut result = valid_result();
        result["risks"] = serde_json::json!([{
            "category": "security_release",
            "severity": "blocker",
            "code": "credential_leak",
            "message": "key committed",
            "evidence_refs": ["path:src/lib.rs"]
        }]);
        let input = serde_json::json!({
            "goal_id": "g1",
            "scope_fingerprint": "fp_1",
            "result": result,
            "scope": {"changed_files": ["src/lib.rs"]}
        })
        .to_string();
        let proposals = ChangeQualityCapability.propose(&input);
        assert_eq!(proposals[0].kind, ProposalKind::SuccessorTodo);
        assert_eq!(
            proposals[0].todo.as_ref().unwrap().action_kind.as_deref(),
            Some("repair_change_quality_blockers")
        );
        assert!(proposals[0].reason.contains("credential_leak"));
    }

    #[test]
    fn propose_qualification_gates_forged_results() {
        let input = serde_json::json!({
            "goal_id": "g1",
            "scope_fingerprint": "other_fp",
            "result": valid_result()
        })
        .to_string();
        let proposals = ChangeQualityCapability.propose(&input);
        assert_eq!(proposals[0].kind, ProposalKind::Gate);
        assert!(proposals[0]
            .gate_question
            .as_deref()
            .unwrap()
            .contains("fingerprint"));
    }

    #[test]
    fn propose_qualification_honors_goal_policy_safe_fix() {
        let mut result = valid_result();
        result["simplification"] = serde_json::json!({
            "outcome": "fixed",
            "summary": "simplified",
            "evidence_refs": ["path:src/lib.rs"],
            "safe_fix_applied": true
        });
        let forbidden = serde_json::json!({
            "goal_id": "g1",
            "scope_fingerprint": "fp_1",
            "goal": {"control_plane": {"change_quality_qualification": {"enabled": true, "safe_fix": false}}},
            "result": result,
            "scope": {"changed_files": ["src/lib.rs"]}
        })
        .to_string();
        let proposals = ChangeQualityCapability.propose(&forbidden);
        assert_eq!(proposals[0].kind, ProposalKind::Gate);
        assert!(proposals[0]
            .gate_question
            .as_deref()
            .unwrap()
            .contains("safe_fix"));
        let allowed = serde_json::json!({
            "goal_id": "g1",
            "scope_fingerprint": "fp_1",
            "policy": {"safe_fix": true},
            "result": result,
            "scope": {"changed_files": ["src/lib.rs"]}
        })
        .to_string();
        let proposals = ChangeQualityCapability.propose(&allowed);
        assert_eq!(proposals[0].kind, ProposalKind::SuccessorTodo);
    }

    #[test]
    fn propose_prepare_discovers_plan_and_proposes_review() {
        let input = serde_json::json!({
            "goal_id": "g1",
            "scope": {
                "scope_fingerprint": "fp_1",
                "changed_files": ["src/lib.rs"]
            },
            "instruction_refs": ["AGENTS.md"],
            "manifests": [
                {"path": "pyproject.toml", "content": "[tool.poe.tasks]\ntest = \"pytest\"\n"}
            ]
        })
        .to_string();
        let proposals = ChangeQualityCapability.propose(&input);
        assert_eq!(proposals[0].kind, ProposalKind::SuccessorTodo);
        let todo = proposals[0].todo.as_ref().unwrap();
        assert_eq!(
            todo.action_kind.as_deref(),
            Some("run_change_quality_validation")
        );
        assert!(proposals[0].reason.contains("repository oracles"));
        assert!(todo.text.contains("test=1"));
    }

    #[test]
    fn propose_gates_unrecognized_payload_shapes() {
        let proposals = ChangeQualityCapability.propose(r#"{"goal_id": "g1"}"#);
        assert_eq!(proposals[0].kind, ProposalKind::Gate);
        assert!(proposals[0].gate_question.is_some());
    }

    #[test]
    fn every_review_lens_has_a_question_and_splits_cleanly() {
        assert_eq!(REVIEW_LENSES.len(), 10);
        for lens in REVIEW_LENSES {
            assert!(!lens.question.is_empty());
        }
        let guardrails: BTreeSet<&str> = SIMPLIFY_GUARDRAIL_LENS_IDS.iter().copied().collect();
        let primaries: BTreeSet<&str> = SIMPLIFY_PRIMARY_LENS_IDS.iter().copied().collect();
        assert!(guardrails.is_disjoint(&primaries));
        assert_eq!(
            guardrails.len() + primaries.len(),
            REVIEW_LENSES.len(),
            "every lens is either a primary conclusion or a derived guardrail"
        );
    }

    // ── diff classification contract ──

    #[test]
    fn classify_new_file_is_addition() {
        let diff = "diff --git a/new.rs b/new.rs\nnew file mode 100644\n@@ -0,0 +1,2 @@\n+fn new() {}\n+fn main() {}\n";
        assert_eq!(classify(diff), vec![ChangeKind::Addition]);
    }

    #[test]
    fn classify_deleted_file_is_deletion() {
        let diff = "diff --git a/old.rs b/old.rs\ndeleted file mode 100644\n@@ -1,2 +0,0 @@\n-fn old() {}\n-fn main() {}\n";
        assert_eq!(classify(diff), vec![ChangeKind::Deletion]);
    }

    #[test]
    fn classify_pure_rename_is_move() {
        let diff = "diff --git a/old.rs b/new.rs\nsimilarity index 100%\nrename from old.rs\nrename to new.rs\n";
        assert_eq!(classify(diff), vec![ChangeKind::Move]);
    }

    #[test]
    fn classify_edited_rename_is_rename_plus_behavior_change() {
        let diff = "diff --git a/old.rs b/new.rs\nsimilarity index 90%\nrename from old.rs\nrename to new.rs\n@@ -1,1 +1,1 @@\n-fn old() {}\n+fn new() {}\n";
        let kinds = classify(diff);
        assert!(kinds.contains(&ChangeKind::Rename), "{kinds:?}");
        assert!(kinds.contains(&ChangeKind::BehaviorChange), "{kinds:?}");
        assert!(!kinds.contains(&ChangeKind::Move));
    }

    #[test]
    fn classify_mixed_hunks_is_behavior_change() {
        let diff = "diff --git a/lib.rs b/lib.rs\n@@ -1,2 +1,3 @@\n let x = 1;\n+let y = 2;\n-let z = 3;\n+let w = 4;\n";
        assert_eq!(classify(diff), vec![ChangeKind::BehaviorChange]);
    }

    #[test]
    fn classify_empty_text_is_empty() {
        assert!(classify("").is_empty());
        assert!(classify("not a diff at all").is_empty());
    }

    #[test]
    fn signals_count_churn_and_hunks_but_not_headers() {
        let diff = "diff --git a/lib.rs b/lib.rs\n--- a/lib.rs\n+++ b/lib.rs\n@@ -1,1 +1,2 @@\n let a = 1;\n+let b = 2;\n@@ -10,1 +11,1 @@\n-let c = 3;\n+let d = 4;\n";
        let signals = quality_signals(diff, &BTreeMap::new());
        assert_eq!(signals.churn, 3);
        assert_eq!(signals.hunk_dispersion, 2);
        assert_eq!(signals.touched_paths, vec!["lib.rs".to_string()]);
        assert_eq!(signals.repeated_change_count, 0);
    }

    #[test]
    fn signals_sum_recent_history_per_touched_path() {
        let diff = "diff --git a/a.rs b/a.rs\n+++ b/a.rs\n@@ -0,0 +1 @@\n+fn a() {}\ndiff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -0,0 +1 @@\n+fn b() {}\n";
        let history = BTreeMap::from([
            ("a.rs".to_string(), 3),
            ("b.rs".to_string(), 1),
            ("c.rs".to_string(), 9),
        ]);
        let signals = quality_signals(diff, &history);
        assert_eq!(signals.repeated_change_count, 4);
        assert_eq!(signals.churn, 2);
    }

    #[test]
    fn risk_score_is_bounded_and_monotonic() {
        assert_eq!(risk_score(&QualitySignals::default()), 0);
        let low = QualitySignals {
            churn: 10,
            hunk_dispersion: 1,
            repeated_change_count: 0,
            touched_paths: vec!["a.rs".to_string()],
        };
        let high = QualitySignals {
            churn: 500,
            hunk_dispersion: 20,
            repeated_change_count: 5,
            touched_paths: vec!["a.rs".to_string()],
        };
        let low_score = risk_score(&low);
        let high_score = risk_score(&high);
        assert!(low_score <= 10, "{low_score}");
        assert!(high_score >= RISK_REPAIR_THRESHOLD, "{high_score}");
        assert!(high_score > low_score);
        assert!(high_score <= 100);
        assert_eq!(high_score, 100, "saturated caps sum to exactly 100");
    }

    #[test]
    fn risk_assessment_reports_per_factor_reasons() {
        let signals = QualitySignals {
            churn: 120,
            hunk_dispersion: 4,
            repeated_change_count: 2,
            touched_paths: vec!["a.rs".to_string()],
        };
        let assessment = risk_assessment(&signals);
        assert_eq!(assessment.score, 17 + 8 + 20);
        assert_eq!(assessment.reasons.len(), 3);
        assert!(assessment.reasons[0].contains("churn of 120"));
        assert!(assessment.reasons[1].contains("4 scattered hunks"));
        assert!(assessment.reasons[2].contains("2 recent changes"));
        let empty = risk_assessment(&QualitySignals::default());
        assert!(empty.reasons.is_empty());
        assert_eq!(empty.score, 0);
    }

    #[test]
    fn propose_diff_high_risk_is_repair() {
        let mut diff = String::new();
        for i in 0..35 {
            diff.push_str(&format!("@@ -{},1 +{},10 @@\n", i * 10, i * 10));
            for j in 0..5 {
                diff.push_str(&format!("+let y = {}_{};\n", i, j));
                diff.push_str(&format!("-let z = {}_{};\n", i, j));
            }
        }
        let cap = ChangeQualityCapability;
        let proposals = cap.propose(&diff);
        assert_eq!(proposals.len(), 1);
        let proposal = &proposals[0];
        assert_eq!(proposal.kind, ProposalKind::SuccessorTodo);
        let todo = proposal.todo.as_ref().unwrap();
        assert_eq!(todo.action_kind.as_deref(), Some("repair_risky_change"));
        assert!(todo.text.contains("behavior_change"), "{}", todo.text);
        assert!(todo.text.contains("risk 70/100"), "{}", todo.text);
    }

    #[test]
    fn propose_diff_low_risk_is_review() {
        let diff = "diff --git a/lib.rs b/lib.rs\n+++ b/lib.rs\n@@ -1,1 +1,2 @@\n let a = 1;\n+let b = 2;\n";
        let cap = ChangeQualityCapability;
        let proposals = cap.propose(diff);
        assert_eq!(proposals.len(), 1);
        let proposal = &proposals[0];
        assert_eq!(proposal.kind, ProposalKind::SuccessorTodo);
        let todo = proposal.todo.as_ref().unwrap();
        assert_eq!(todo.action_kind.as_deref(), Some("review_qualified_change"));
        assert!(todo.text.contains("addition"), "{}", todo.text);
        assert!(todo.text.contains("risk 7/100"), "{}", todo.text);
    }

    #[test]
    fn propose_unrecognized_diff_shape_is_gated() {
        let cap = ChangeQualityCapability;
        let proposals = cap.propose("--- a/lib.rs\n+++ b/lib.rs\n");
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].kind, ProposalKind::Gate);
    }
}
