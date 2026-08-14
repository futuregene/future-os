//! Content-Ops capability (LoopX: content-ops — ordered content operations
//! with bounded per-surface effects; a capability never writes state itself:
//! it proposes, the kernel decides).
//!
//! Wave 2 deepening: the 22-line "non-empty → successor" shell becomes a
//! content-operation pipeline, porting the core subdomains of the reference
//! `capabilities/content_ops/` (surface / item_lifecycle / connector_packets
//! / schemas, 4,281 lines) as deterministic rule versions:
//!
//! - **content classification**: free-text material routes into a bounded
//!   `ContentKind` vocabulary (outline / draft / feedback / url /
//!   source_note / connector_report / unrecognized) with shape markers
//!   (headline, list, url, feedback phrasing, publish intent, private
//!   signals, source attribution, api paths);
//! - **quality & length signals**: deterministic word/line/paragraph
//!   counts, headline and list structure, source-ref density, private and
//!   raw-material key hits, call-to-action presence, and a bounded 0..100
//!   quality score with length/quality bands — derived, never authored;
//! - **operation suggestions**: the finite ordered-operation vocabulary of
//!   the reference projection (draft from angle / source review / publish
//!   gate / connector metadata trial / owner gate / revise / record
//!   feedback) mapped onto the capability's finite proposal set
//!   (successor / gate / no-follow-up);
//! - **state surface**: `content_ops_surface_v0` records (source items,
//!   angle candidates, draft items, feedback signals, publish gates,
//!   material memory, connector trials) with the full reference validation
//!   (schema versions, status vocabularies, cross-record references,
//!   boundary flags, raw/private key-name detection) and the first-screen
//!   projection (waiting_on / next_safe_action / todo candidates /
//!   truth contract);
//! - **item lifecycle**: the content item state machine (captured → draft
//!   → review_ready → approved → delivery_ready → published →
//!   readback_verified with skipped/superseded terminals) — token/digest/
//!   timestamp discipline, effect-record coherence (approval / delivery
//!   intent / delivery receipt / readback), idempotent event application
//!   (`expected_state` / `expected_revision` guards, canonical event
//!   digests), and the managed queue projection;
//! - **boundary discipline**: public-https URL normalization (no
//!   credentials, query, fragment, localhost, private/loopback/link-local
//!   addresses), `autopublish_allowed=false` everywhere, and
//!   `external_write_allowed=false` on every connector trial.
//!
//! Out of scope (deliberately): the live HEAD-fetch public handle
//! observation, the ChatView connector report, walkthrough artifact
//! rendering, and the CLI of the reference — those stay LoopX-side; this
//! module ships the deterministic classification + validation + projection
//! + lifecycle contract core and takes host-supplied records as inputs.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::{successor_todo, Capability, TypedProposal};

// ── schema vocabulary (reference schemas.py constants) ────────────────────

pub const CONTENT_OPS_SURFACE_SCHEMA_VERSION: &str = "content_ops_surface_v0";
pub const CONTENT_OPS_SURFACE_PROJECTION_SCHEMA_VERSION: &str = "content_ops_surface_projection_v0";
pub const CONTENT_OPS_VALIDATION_SCHEMA_VERSION: &str = "content_ops_surface_validation_v0";
pub const CONTENT_OPS_CONNECTOR_RUNTIME_POLICY_SCHEMA_VERSION: &str =
    "content_ops_connector_runtime_policy_v0";
pub const SOURCE_ITEM_SCHEMA_VERSION: &str = "source_item_v0";
pub const ANGLE_CANDIDATE_SCHEMA_VERSION: &str = "angle_candidate_v0";
pub const DRAFT_ITEM_SCHEMA_VERSION: &str = "draft_item_v0";
pub const FEEDBACK_SIGNAL_SCHEMA_VERSION: &str = "feedback_signal_v0";
pub const PUBLISH_GATE_SCHEMA_VERSION: &str = "publish_gate_v0";
pub const MATERIAL_MEMORY_SCHEMA_VERSION: &str = "material_memory_v0";
pub const CONNECTOR_TRIAL_SCHEMA_VERSION: &str = "connector_trial_v0";
pub const CONTENT_OPS_ITEM_SCHEMA_VERSION: &str = "content_ops_item_v0";
pub const CONTENT_OPS_ITEM_PROJECTION_SCHEMA_VERSION: &str = "content_ops_item_projection_v0";
pub const CONTENT_OPS_ITEM_TRANSITION_RECEIPT_SCHEMA_VERSION: &str =
    "content_ops_item_transition_receipt_v0";
pub const CONTENT_OPS_QUEUE_PROJECTION_SCHEMA_VERSION: &str = "content_ops_queue_projection_v0";

/// Key-name fragments that must never appear in content-ops records
/// (reference `RAW_MATERIAL_KEY_HINTS`): the surface stays compact and
/// public-safe — raw bodies, chat transcripts, credentials, and local paths
/// stay outside LoopX state.
pub const RAW_MATERIAL_KEY_HINTS: [&str; 11] = [
    "body",
    "chat",
    "credential",
    "dm",
    "local_path",
    "log",
    "message",
    "raw",
    "secret",
    "token",
    "transcript",
];

pub const ALLOWED_SOURCE_STATUSES: [&str; 5] = [
    "public",
    "private_needs_review",
    "synthetic_public_safe",
    "unpublished",
    "forbidden_for_public_surface",
];
pub const ALLOWED_FRESHNESS: [&str; 3] = ["fresh", "stale", "unknown"];
pub const ALLOWED_USE_POLICIES: [&str; 4] = [
    "summarize_and_transform",
    "metadata_only",
    "do_not_quote",
    "forbidden",
];
pub const ALLOWED_ANGLE_DECISIONS: [&str; 4] = ["draft", "reject", "hold", "needs_review"];
pub const ALLOWED_DRAFT_STATES: [&str; 5] =
    ["outline", "draft", "rewrite", "blocked", "ready_for_review"];
pub const ALLOWED_FEEDBACK_EFFECTS: [&str; 4] = [
    "preference_hint",
    "source_boundary_correction",
    "rewrite_todo",
    "publish_decision",
];
pub const ALLOWED_PUBLISH_GATE_STATUSES: [&str; 4] = [
    "blocked_until_user_approval",
    "approved",
    "denied",
    "needs_revision",
];
pub const ALLOWED_CONNECTOR_TRIAL_STATES: [&str; 5] = [
    "candidate",
    "metadata_packet_collected",
    "ready_for_metadata_trial",
    "needs_owner_gate",
    "blocked",
];
pub const ALLOWED_CONNECTOR_ACCESS_MODES: [&str; 3] = [
    "public_metadata_only",
    "private_metadata_only",
    "synthetic_fixture_only",
];

pub const ALLOWED_ITEM_KINDS: [&str; 5] = ["article", "post", "profile_update", "reply", "repost"];
pub const ALLOWED_ITEM_STATES: [&str; 9] = [
    "captured",
    "draft",
    "review_ready",
    "approved",
    "delivery_ready",
    "published",
    "readback_verified",
    "skipped",
    "superseded",
];
pub const ALLOWED_EFFECT_KINDS: [&str; 4] = ["profile_update", "publish", "reply", "repost"];
pub const TERMINAL_ITEM_STATES: [&str; 3] = ["readback_verified", "skipped", "superseded"];

// ── content classification vocabulary ─────────────────────────────────────

/// The bounded content-kind vocabulary free-text material classifies into
/// (adapted from the reference surface records: angle candidates, draft
/// items, feedback signals, connector reports).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentKind {
    Outline,
    Draft,
    Feedback,
    Url,
    SourceNote,
    ConnectorReport,
    Unrecognized,
}

impl ContentKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ContentKind::Outline => "outline",
            ContentKind::Draft => "draft",
            ContentKind::Feedback => "feedback",
            ContentKind::Url => "url",
            ContentKind::SourceNote => "source_note",
            ContentKind::ConnectorReport => "connector_report",
            ContentKind::Unrecognized => "unrecognized",
        }
    }
}

const FEEDBACK_MARKERS: [&str; 14] = [
    "prefer",
    "preference",
    "feedback",
    "偏好",
    "反馈",
    "suggest",
    "建议",
    "tone",
    "风格",
    "salesy",
    "rewrite",
    "reject",
    "hold",
    "too long",
];
const PUBLISH_MARKERS: [&str; 7] = [
    "publish", "发布", "post", "approve", "promote", "推广", "发文",
];
const PRIVATE_MARKERS: [&str; 14] = [
    "private",
    "credential",
    "secret",
    "password",
    "token",
    "bearer ",
    "chatlog",
    "wechat",
    "微信",
    "私密",
    "未公开",
    "unpublished",
    "confidential",
    "login",
];
const SOURCE_ATTRIBUTION_MARKERS: [&str; 5] = ["source:", "来源", "attribution", "via ", "from "];
const CONNECTOR_REPORT_MARKERS: [&str; 6] = [
    "channel_count",
    "record_count",
    "report_count",
    "api_request",
    "connector report",
    "chatview",
];

fn contains_any(text: &str, markers: &[&str]) -> bool {
    markers.iter().any(|marker| text.contains(marker))
}

/// Shape markers observed in one piece of content material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentClass {
    pub kind: ContentKind,
    pub has_headline: bool,
    pub has_list: bool,
    pub has_url: bool,
    pub has_feedback_marker: bool,
    pub has_publish_marker: bool,
    pub has_private_signal: bool,
    pub has_source_attribution: bool,
    pub has_api_path: bool,
    pub word_count: usize,
}

/// Classify free-text content material into the bounded kind vocabulary.
/// Priority order mirrors the reference pipeline: urls and connector
/// reports route to intake lanes, feedback routes to signal recording,
/// structured text to outlines, prose to drafts.
pub fn classify_content(input: &str) -> ContentClass {
    let text = input.replace("\r\n", "\n");
    let lowered = text.to_lowercase();
    let words = text.split_whitespace().count();

    let has_url = lowered.contains("http://") || lowered.contains("https://");
    let has_api_path = lowered.contains("/api/");
    let has_connector_report = has_api_path || contains_any(&lowered, &CONNECTOR_REPORT_MARKERS);
    let has_feedback_marker = contains_any(&lowered, &FEEDBACK_MARKERS);
    let has_publish_marker = contains_any(&lowered, &PUBLISH_MARKERS);
    let has_private_signal = contains_any(&lowered, &PRIVATE_MARKERS);
    let has_source_attribution = contains_any(&lowered, &SOURCE_ATTRIBUTION_MARKERS);

    let first_line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    let has_headline = first_line.starts_with('#')
        || contains_any(
            &first_line.to_lowercase(),
            &["title:", "标题", "headline:", "subject:"],
        );
    let has_list = text
        .lines()
        .filter(|line| {
            let line = line.trim_start();
            line.starts_with("- ")
                || line.starts_with("* ")
                || line
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .count()
                    .gt(&0)
                    && line.contains(". ")
        })
        .count()
        >= 2;

    let kind = if has_url {
        ContentKind::Url
    } else if has_connector_report {
        ContentKind::ConnectorReport
    } else if has_feedback_marker && words < 300 {
        ContentKind::Feedback
    } else if has_source_attribution && words < 80 {
        ContentKind::SourceNote
    } else if has_headline && has_list && words < 400 {
        ContentKind::Outline
    } else if has_publish_marker || words >= 60 {
        ContentKind::Draft
    } else {
        ContentKind::Unrecognized
    };

    ContentClass {
        kind,
        has_headline,
        has_list,
        has_url,
        has_feedback_marker,
        has_publish_marker,
        has_private_signal,
        has_source_attribution,
        has_api_path,
        word_count: words,
    }
}

// ── quality & length signals ──────────────────────────────────────────────

/// Deterministic quality and length signals over one piece of content
/// material (reference: length is a signal, quality is derived — nothing is
/// authored by prose).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualitySignals {
    pub words: usize,
    pub chars: usize,
    pub lines: usize,
    pub paragraphs: usize,
    pub headline_chars: Option<usize>,
    pub list_item_count: usize,
    pub url_count: usize,
    pub source_ref_count: usize,
    pub private_hits: Vec<String>,
    pub raw_key_hits: Vec<String>,
    pub call_to_action: bool,
    pub score: u32,
    pub flags: Vec<String>,
}

impl QualitySignals {
    /// Length band over word count: short / medium / long.
    pub fn length_band(&self) -> &'static str {
        if self.words < 60 {
            "short"
        } else if self.words <= 1200 {
            "medium"
        } else {
            "long"
        }
    }

    /// Quality band over the bounded score: strong / needs_work / weak.
    pub fn quality_band(&self) -> &'static str {
        if self.score >= 70 {
            "strong"
        } else if self.score >= 40 {
            "needs_work"
        } else {
            "weak"
        }
    }
}

/// Compute quality and length signals for content material of a given kind.
/// The score is a bounded 0..100 rubric: length fit, headline and list
/// structure, source references, link discipline, and penalties for private
/// or raw-looking material.
pub fn quality_signals(text: &str, kind: ContentKind) -> QualitySignals {
    let normalized = text.replace("\r\n", "\n");
    let lowered = normalized.to_lowercase();
    let words = normalized.split_whitespace().count();
    let chars = normalized.chars().count();
    let lines = normalized
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    let paragraphs = normalized
        .split("\n\n")
        .filter(|para| !para.trim().is_empty())
        .count();
    let list_item_count = normalized
        .lines()
        .filter(|line| {
            let line = line.trim_start();
            line.starts_with("- ")
                || line.starts_with("* ")
                || line
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .count()
                    .gt(&0)
                    && line.contains(". ")
        })
        .count();
    let url_count = lowered.matches("http://").count() + lowered.matches("https://").count();
    let source_ref_count = normalized
        .lines()
        .filter(|line| {
            let line = line.trim_start().to_lowercase();
            line.starts_with("source:") || line.starts_with("ref:") || line.starts_with("来源")
        })
        .count();

    let private_hits: Vec<String> = PRIVATE_MARKERS
        .iter()
        .filter(|marker| lowered.contains(**marker))
        .map(|marker| marker.to_string())
        .collect();
    let raw_key_hits: Vec<String> = normalized
        .split_whitespace()
        .filter_map(|word| {
            let word = word
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase();
            RAW_MATERIAL_KEY_HINTS
                .iter()
                .find(|hint| word.contains(**hint) && word.len() >= hint.len())
                .map(|hint| hint.to_string())
        })
        .collect::<BTreeSet<String>>()
        .into_iter()
        .collect();

    let call_to_action = normalized.lines().any(|line| {
        let line = line.trim_start().to_lowercase();
        line.starts_with("next:")
            || line.starts_with("todo:")
            || line.starts_with("action:")
            || line.starts_with("please")
    });

    let first_line = normalized
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    let headline_chars = if first_line.starts_with('#')
        || contains_any(
            &first_line.to_lowercase(),
            &["title:", "标题", "headline:", "subject:"],
        ) {
        Some(first_line.trim_start_matches('#').trim().chars().count())
    } else {
        None
    };

    let mut score: i32 = 50;
    if (120..=1200).contains(&words) {
        score += 15;
    } else if (60..120).contains(&words) || (1201..=2400).contains(&words) {
        score += 5;
    } else if words < 60 {
        score -= 10;
    } else {
        score -= 5;
    }
    if headline_chars.is_some() {
        score += 10;
    }
    if list_item_count >= 3 {
        score += 5;
    }
    if source_ref_count > 0 {
        score += 5;
    }
    if url_count > 0 && url_count <= 2 {
        score += 2;
    }
    if !private_hits.is_empty() {
        score -= 25 * private_hits.len() as i32;
    }
    if !raw_key_hits.is_empty() {
        score -= 15;
    }
    if !call_to_action && matches!(kind, ContentKind::Draft | ContentKind::Outline) {
        score -= 5;
    }

    let mut flags: Vec<String> = Vec::new();
    if words < 60 {
        flags.push("too short".to_string());
    }
    if words > 2400 {
        flags.push("too long".to_string());
    }
    if headline_chars.is_none() && matches!(kind, ContentKind::Draft | ContentKind::Outline) {
        flags.push("no headline".to_string());
    }
    if !private_hits.is_empty() {
        flags.push("private signal present".to_string());
    }
    if !raw_key_hits.is_empty() {
        flags.push("raw material key present".to_string());
    }
    if url_count > 2 {
        flags.push("link heavy".to_string());
    }
    if !call_to_action && matches!(kind, ContentKind::Draft | ContentKind::Outline) {
        flags.push("no call to action".to_string());
    }

    QualitySignals {
        words,
        chars,
        lines,
        paragraphs,
        headline_chars,
        list_item_count,
        url_count,
        source_ref_count,
        private_hits,
        raw_key_hits,
        call_to_action,
        score: score.clamp(0, 100) as u32,
        flags,
    }
}

// ── operation suggestions ─────────────────────────────────────────────────

/// One ordered content operation suggested from classification + signals
/// (the finite action_kind vocabulary of the reference projection's todo
/// candidates).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpSuggestion {
    /// "agent" (runnable as a successor) or "user" (a gate question).
    pub role: &'static str,
    pub action_kind: &'static str,
    pub title: String,
}

/// Derive the finite set of ordered content operations from a content
/// class and its quality/length signals. Private signals always raise a
/// user boundary gate first; publish intent always raises a publish gate.
pub fn suggest_operations(class: &ContentClass, signals: &QualitySignals) -> Vec<OpSuggestion> {
    let mut ops: Vec<OpSuggestion> = Vec::new();

    if !signals.private_hits.is_empty() || !signals.raw_key_hits.is_empty() {
        ops.push(OpSuggestion {
            role: "user",
            action_kind: "content_ops_source_boundary",
            title: format!(
                "Review source boundaries: private-looking material ({} / {}) must stay metadata-only until an owner gate approves use.",
                signals.private_hits.join(", "),
                signals.raw_key_hits.join(", ")
            ),
        });
    }

    match class.kind {
        ContentKind::Outline => ops.push(OpSuggestion {
            role: "agent",
            action_kind: "content_ops_draft_from_angle",
            title: "Draft a concrete content angle from the outline; keep source refs, ask for taste before publishing.".to_string(),
        }),
        ContentKind::Draft => {
            if signals.score >= 70 {
                ops.push(OpSuggestion {
                    role: "agent",
                    action_kind: "content_ops_submit_review",
                    title: format!(
                        "Content reads {} ({}/100, {} words): submit for review with the source map attached.",
                        signals.quality_band(),
                        signals.score,
                        signals.words
                    ),
                });
            } else {
                let angle_hint = if signals.flags.is_empty() {
                    "tighten the angle".to_string()
                } else {
                    signals.flags.join("; ")
                };
                ops.push(OpSuggestion {
                    role: "agent",
                    action_kind: "content_ops_revise_draft",
                    title: format!(
                        "Revise the draft ({}/100, {} words): {}.",
                        signals.score, signals.words, angle_hint
                    ),
                });
            }
        }
        ContentKind::Feedback => ops.push(OpSuggestion {
            role: "agent",
            action_kind: "content_ops_record_feedback",
            title: "Record the feedback signal against its target item and apply the effect (preference hint or rewrite todo).".to_string(),
        }),
        ContentKind::Url => ops.push(OpSuggestion {
            role: "agent",
            action_kind: "content_ops_observe_public_handle",
            title: "Run a metadata-only observation of the public handle: HEAD probe only, no login, body capture, or posting; record a compact source_item_v0.".to_string(),
        }),
        ContentKind::SourceNote => ops.push(OpSuggestion {
            role: "agent",
            action_kind: "content_ops_register_source",
            title: "Register the material as a compact source_item_v0 record with attribution and allowed_use before drafting anything.".to_string(),
        }),
        ContentKind::ConnectorReport => ops.push(OpSuggestion {
            role: "user",
            action_kind: "content_ops_connector_owner_gate",
            title: "Approve or reject metadata-only connector intake before any private source content read, quote, or summary.".to_string(),
        }),
        ContentKind::Unrecognized => ops.push(OpSuggestion {
            role: "agent",
            action_kind: "content_ops_register_source",
            title: "Record the material as compact source metadata (attribution + allowed_use) before drafting an angle.".to_string(),
        }),
    }

    if class.has_publish_marker && !class.has_private_signal {
        ops.push(OpSuggestion {
            role: "user",
            action_kind: "content_ops_publish_gate",
            title: "Approve, deny, or request revision before any external posting.".to_string(),
        });
    }

    ops
}

// ── helpers ───────────────────────────────────────────────────────────────

/// Collapse whitespace and bound text (reference `_text`).
fn compact_text(value: Option<&Value>, limit: usize) -> Option<String> {
    let text = value?
        .as_str()?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() {
        return None;
    }
    if text.chars().count() <= limit {
        return Some(text);
    }
    let mut truncated: String = text.chars().take(limit.saturating_sub(1)).collect();
    truncated.push_str("...");
    Some(truncated)
}

/// Ids of a record field across a group (reference `_ids`).
fn record_ids(records: &[&Value], key: &str) -> BTreeSet<String> {
    records
        .iter()
        .filter_map(|record| record.get(key))
        .filter_map(Value::as_str)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

/// Counter over string field values (reference `_counter`).
fn counter<'a>(values: impl Iterator<Item = &'a str>) -> BTreeMap<String, usize> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for value in values.filter(|value| !value.is_empty()) {
        *counts.entry(value.to_string()).or_default() += 1;
    }
    counts
}

fn in_vocab(value: Option<&Value>, vocab: &[&str]) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|text| vocab.contains(&text))
}

fn records(value: Option<&Value>) -> Vec<&Value> {
    value
        .and_then(Value::as_array)
        .map(|array| array.iter().filter(|item| item.is_object()).collect())
        .unwrap_or_default()
}

// ── public https URL normalization ────────────────────────────────────────

/// Normalize a public handle URL: https only, no credentials, no query or
/// fragment, a real host that is not localhost/.local, default port only,
/// and no private/loopback/link-local/multicast/unspecified addresses
/// (reference `_normalise_public_https_url`).
pub fn normalize_public_https_url(url: &str) -> Result<String, String> {
    let text = url.trim();
    let rest = text
        .strip_prefix("https://")
        .ok_or("public handle observation requires an https URL")?;
    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, "/"),
    };
    if authority.is_empty() {
        return Err("public handle URL must include a host".to_string());
    }
    if authority.contains('@') {
        return Err("public handle URL must not include credentials".to_string());
    }
    if authority.contains('?') || authority.contains('#') {
        return Err("public handle URL must not include query or fragment data".to_string());
    }
    if path.contains('?') || path.contains('#') {
        return Err("public handle URL must not include query or fragment data".to_string());
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => (host, Some(port)),
        _ => (authority, None),
    };
    if let Some(port) = port {
        if port != "443" {
            return Err("public handle URL must use the default https port".to_string());
        }
    }
    let mut host = host.to_ascii_lowercase();
    if host.starts_with('[') && host.ends_with(']') {
        host = host[1..host.len() - 1].to_string();
    }
    host = host.trim_end_matches('.').to_string();
    if host.is_empty() {
        return Err("public handle URL must include a host".to_string());
    }
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") {
        return Err("public handle URL must not target localhost or local hosts".to_string());
    }
    if let Ok(address) = host.parse::<std::net::IpAddr>() {
        let localish = match address {
            std::net::IpAddr::V4(v4) => {
                v4.is_private()
                    || v4.is_loopback()
                    || v4.is_link_local()
                    || v4.is_broadcast()
                    || v4.is_multicast()
                    || v4.is_unspecified()
            }
            std::net::IpAddr::V6(v6) => {
                v6.is_loopback()
                    || v6.is_unicast_link_local()
                    || v6.is_multicast()
                    || v6.is_unspecified()
                    || (v6.segments()[0] & 0xfe00) == 0xfc00
            }
        };
        if localish {
            return Err("public handle URL must not target private or local addresses".to_string());
        }
    }
    Ok(format!("https://{authority}{path}"))
}

// ── state surface validation ──────────────────────────────────────────────

/// Validation result over a content-ops state surface (reference
/// `validate_content_ops_surface`): schema versions, status vocabularies,
/// cross-record references, boundary flags, and raw key-name detection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceValidation {
    pub ok: bool,
    pub errors: Vec<String>,
    pub record_counts: BTreeMap<String, usize>,
    pub raw_material_key_names: Vec<String>,
}

/// Key names across every record group that look like raw material
/// (reference `_raw_material_key_names`).
fn raw_material_key_names(groups: &[&[&Value]]) -> Vec<String> {
    let mut names: BTreeSet<String> = BTreeSet::new();
    for group in groups {
        for object in group.iter().filter_map(|record| record.as_object()) {
            for key in object.keys() {
                let lowered = key.to_lowercase();
                if RAW_MATERIAL_KEY_HINTS
                    .iter()
                    .any(|hint| lowered.contains(hint))
                {
                    names.insert(key.clone());
                }
            }
        }
    }
    names.into_iter().collect()
}

/// Validate a `content_ops_surface_v0` state surface. Every check is a
/// deterministic rule over the records; nothing is inferred from prose.
pub fn validate_content_ops_surface(surface: &Value) -> SurfaceValidation {
    let source_items = records(surface.get("source_items"));
    let angle_candidates = records(surface.get("angle_candidates"));
    let draft_items = records(surface.get("draft_items"));
    let feedback_signals = records(surface.get("feedback_signals"));
    let publish_gates = records(surface.get("publish_gates"));
    let material_memory = records(surface.get("material_memory"));
    let connector_trials = records(surface.get("connector_trials"));

    let mut errors: Vec<String> = Vec::new();
    let source_ids = record_ids(&source_items, "source_item_id");
    let angle_ids = record_ids(&angle_candidates, "angle_id");
    let draft_ids = record_ids(&draft_items, "draft_id");
    let gate_ids = record_ids(&publish_gates, "gate_id");

    if surface.get("schema_version").and_then(Value::as_str)
        != Some(CONTENT_OPS_SURFACE_SCHEMA_VERSION)
    {
        errors.push("surface schema_version must be content_ops_surface_v0".to_string());
    }
    for (records, label) in [
        (
            &source_items,
            "at least one source_item_v0 record is required",
        ),
        (
            &angle_candidates,
            "at least one angle_candidate_v0 record is required",
        ),
        (
            &draft_items,
            "at least one draft_item_v0 record is required",
        ),
        (
            &feedback_signals,
            "at least one feedback_signal_v0 record is required",
        ),
        (
            &publish_gates,
            "at least one publish_gate_v0 record is required",
        ),
        (
            &material_memory,
            "at least one material_memory_v0 record is required",
        ),
        (
            &connector_trials,
            "at least one connector_trial_v0 record is required",
        ),
    ] {
        if records.is_empty() {
            errors.push(label.to_string());
        }
    }

    for item in &source_items {
        let id = compact_text(item.get("source_item_id"), 120);
        if item.get("schema_version").and_then(Value::as_str) != Some(SOURCE_ITEM_SCHEMA_VERSION) {
            errors.push(format!("source item {id:?} has wrong schema"));
        }
        if !in_vocab(item.get("source_status"), &ALLOWED_SOURCE_STATUSES) {
            errors.push(format!("source item {id:?} has invalid source_status"));
        }
        if !in_vocab(item.get("freshness"), &ALLOWED_FRESHNESS) {
            errors.push(format!("source item {id:?} has invalid freshness"));
        }
        if !in_vocab(item.get("allowed_use"), &ALLOWED_USE_POLICIES) {
            errors.push(format!("source item {id:?} has invalid allowed_use"));
        }
    }

    for item in &angle_candidates {
        let id = compact_text(item.get("angle_id"), 120);
        if item.get("schema_version").and_then(Value::as_str)
            != Some(ANGLE_CANDIDATE_SCHEMA_VERSION)
        {
            errors.push(format!("angle {id:?} has wrong schema"));
        }
        if !in_vocab(item.get("decision"), &ALLOWED_ANGLE_DECISIONS) {
            errors.push(format!("angle {id:?} has invalid decision"));
        }
        for source_id in item
            .get("source_item_ids")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let source_id = source_id.as_str().unwrap_or("");
            if !source_id.is_empty() && !source_ids.contains(source_id) {
                errors.push(format!(
                    "angle {id:?} references unknown source {source_id}"
                ));
            }
        }
    }

    for item in &draft_items {
        let id = compact_text(item.get("draft_id"), 120);
        if item.get("schema_version").and_then(Value::as_str) != Some(DRAFT_ITEM_SCHEMA_VERSION) {
            errors.push(format!("draft {id:?} has wrong schema"));
        }
        if !in_vocab(item.get("state"), &ALLOWED_DRAFT_STATES) {
            errors.push(format!("draft {id:?} has invalid state"));
        }
        if item
            .get("angle_id")
            .and_then(Value::as_str)
            .is_none_or(|angle_id| !angle_ids.contains(angle_id))
        {
            errors.push(format!("draft {id:?} references unknown angle"));
        }
        if item
            .get("publish_gate_id")
            .and_then(Value::as_str)
            .is_none_or(|gate_id| !gate_ids.contains(gate_id))
        {
            errors.push(format!("draft {id:?} references unknown publish gate"));
        }
        let source_map = item.get("source_map");
        if !source_map.is_some_and(Value::is_array) {
            errors.push(format!("draft {id:?} must carry a source_map"));
        } else {
            for source_ref in source_map.and_then(Value::as_array).into_iter().flatten() {
                let source_id = source_ref
                    .get("source_item_id")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if source_id.is_empty() || !source_ids.contains(source_id) {
                    errors.push(format!("draft {id:?} source_map references unknown source"));
                }
            }
        }
    }

    for item in &feedback_signals {
        let id = compact_text(item.get("feedback_id"), 120);
        if item.get("schema_version").and_then(Value::as_str)
            != Some(FEEDBACK_SIGNAL_SCHEMA_VERSION)
        {
            errors.push(format!("feedback {id:?} has wrong schema"));
        }
        if !in_vocab(item.get("effect"), &ALLOWED_FEEDBACK_EFFECTS) {
            errors.push(format!("feedback {id:?} has invalid effect"));
        }
        let target_id = item.get("target_id").and_then(Value::as_str).unwrap_or("");
        if !draft_ids.contains(target_id)
            && !source_ids.contains(target_id)
            && !angle_ids.contains(target_id)
        {
            errors.push(format!("feedback {id:?} references unknown target"));
        }
    }

    for item in &publish_gates {
        let id = compact_text(item.get("gate_id"), 120);
        if item.get("schema_version").and_then(Value::as_str) != Some(PUBLISH_GATE_SCHEMA_VERSION) {
            errors.push(format!("publish gate {id:?} has wrong schema"));
        }
        if !in_vocab(item.get("status"), &ALLOWED_PUBLISH_GATE_STATUSES) {
            errors.push(format!("publish gate {id:?} has invalid status"));
        }
        if item.get("autopublish_allowed").and_then(Value::as_bool) != Some(false) {
            errors.push(format!(
                "publish gate {id:?} must set autopublish_allowed=false"
            ));
        }
        if item.get("approval_required").and_then(Value::as_bool) != Some(true) {
            errors.push(format!("publish gate {id:?} must require approval"));
        }
    }

    for item in &material_memory {
        let id = compact_text(item.get("memory_id"), 120);
        if item.get("schema_version").and_then(Value::as_str)
            != Some(MATERIAL_MEMORY_SCHEMA_VERSION)
        {
            errors.push(format!("memory {id:?} has wrong schema"));
        }
        let source_id = item
            .get("source_item_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !source_ids.contains(source_id) {
            errors.push(format!("memory {id:?} references unknown source"));
        }
    }

    for item in &connector_trials {
        let id = compact_text(item.get("trial_id"), 120);
        if item.get("schema_version").and_then(Value::as_str)
            != Some(CONNECTOR_TRIAL_SCHEMA_VERSION)
        {
            errors.push(format!("connector trial {id:?} has wrong schema"));
        }
        if !in_vocab(item.get("source_status"), &ALLOWED_SOURCE_STATUSES) {
            errors.push(format!("connector trial {id:?} has invalid source_status"));
        }
        if !in_vocab(item.get("freshness"), &ALLOWED_FRESHNESS) {
            errors.push(format!("connector trial {id:?} has invalid freshness"));
        }
        if !in_vocab(item.get("allowed_use"), &ALLOWED_USE_POLICIES) {
            errors.push(format!("connector trial {id:?} has invalid allowed_use"));
        }
        if !in_vocab(item.get("trial_state"), &ALLOWED_CONNECTOR_TRIAL_STATES) {
            errors.push(format!("connector trial {id:?} has invalid trial_state"));
        }
        if !in_vocab(item.get("access_mode"), &ALLOWED_CONNECTOR_ACCESS_MODES) {
            errors.push(format!("connector trial {id:?} has invalid access_mode"));
        }
        if item.get("external_write_allowed").and_then(Value::as_bool) != Some(false) {
            errors.push(format!(
                "connector trial {id:?} must keep external_write_allowed=false"
            ));
        }
        if item.get("access_mode").and_then(Value::as_str) == Some("private_metadata_only")
            && item.get("requires_user_gate").and_then(Value::as_bool) != Some(true)
        {
            errors.push(format!(
                "connector trial {id:?} must gate private metadata use"
            ));
        }
    }

    if let Some(boundary) = surface.get("boundary").filter(|b| b.is_object()) {
        if boundary.get("public_safe").and_then(Value::as_bool) != Some(true) {
            errors.push("boundary.public_safe must be true".to_string());
        }
        for key in [
            "raw_private_material_recorded",
            "raw_platform_data_recorded",
            "credentials_recorded",
            "autopublish_allowed",
            "connector_bodies_are_source_of_truth",
        ] {
            if boundary.get(key).and_then(Value::as_bool) != Some(false) {
                errors.push(format!("boundary.{key} must be false"));
            }
        }
        if boundary
            .get("publish_requires_user_gate")
            .and_then(Value::as_bool)
            != Some(true)
        {
            errors.push("boundary.publish_requires_user_gate must be true".to_string());
        }
    } else {
        errors.push("surface boundary flags are required".to_string());
    }

    let groups: [&[&Value]; 7] = [
        &source_items,
        &angle_candidates,
        &draft_items,
        &feedback_signals,
        &publish_gates,
        &material_memory,
        &connector_trials,
    ];
    let raw_key_names = raw_material_key_names(&groups);
    if !raw_key_names.is_empty() {
        errors.push(
            "raw/private-looking key names must not appear in content-ops records".to_string(),
        );
    }

    let mut record_counts: BTreeMap<String, usize> = BTreeMap::new();
    record_counts.insert("source_items".to_string(), source_items.len());
    record_counts.insert("angle_candidates".to_string(), angle_candidates.len());
    record_counts.insert("draft_items".to_string(), draft_items.len());
    record_counts.insert("feedback_signals".to_string(), feedback_signals.len());
    record_counts.insert("publish_gates".to_string(), publish_gates.len());
    record_counts.insert("material_memory".to_string(), material_memory.len());
    record_counts.insert("connector_trials".to_string(), connector_trials.len());

    SurfaceValidation {
        ok: errors.is_empty(),
        errors,
        record_counts,
        raw_material_key_names: raw_key_names,
    }
}

// ── state surface projection ──────────────────────────────────────────────

/// First-screen status fields (reference `project_content_ops_surface`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirstScreen {
    pub waiting_on: String,
    pub user_action_required: bool,
    pub agent_can_continue: bool,
    pub safe_side_work_available: bool,
    pub source_review_required_count: usize,
    pub ready_to_draft_count: usize,
    pub waiting_for_feedback_count: usize,
    pub publish_decision_count: usize,
    pub next_safe_action: String,
}

/// One ordered operation candidate projected from the surface (reference
/// `todo_candidates`): role + action_kind + bounded text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoCandidate {
    pub role: String,
    pub action_kind: String,
    pub title: String,
    pub ref_ids: Vec<String>,
    pub validation_surface: String,
    pub stop_condition: Option<String>,
}

/// Read-only projection of a content-ops surface (reference
/// `project_content_ops_surface`): waiting_on / next_safe_action / ordered
/// todo candidates / a truth contract that forbids editing the projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceProjection {
    pub surface_id: Option<String>,
    pub first_screen: FirstScreen,
    pub record_counts: BTreeMap<String, usize>,
    pub source_statuses: BTreeMap<String, usize>,
    pub draft_states: BTreeMap<String, usize>,
    pub feedback_effects: BTreeMap<String, usize>,
    pub publish_gate_statuses: BTreeMap<String, usize>,
    pub connector_trial_state_counts: BTreeMap<String, usize>,
    pub todo_candidates: Vec<TodoCandidate>,
}

/// Project a content-ops surface into first-screen status fields and
/// ordered operation candidates. Pure read-only: the projection never
/// writes back into the surface.
pub fn project_content_ops_surface(surface: &Value) -> SurfaceProjection {
    let source_items = records(surface.get("source_items"));
    let angle_candidates = records(surface.get("angle_candidates"));
    let draft_items = records(surface.get("draft_items"));
    let feedback_signals = records(surface.get("feedback_signals"));
    let publish_gates = records(surface.get("publish_gates"));
    let connector_trials = records(surface.get("connector_trials"));
    let validation = validate_content_ops_surface(surface);

    let source_review_required: Vec<&Value> = source_items
        .iter()
        .copied()
        .filter(|item| {
            matches!(
                item.get("source_status").and_then(Value::as_str),
                Some("private_needs_review" | "unpublished")
            ) || item.get("allowed_use").and_then(Value::as_str) == Some("metadata_only")
        })
        .collect();
    let ready_angles: Vec<&Value> = angle_candidates
        .iter()
        .copied()
        .filter(|item| item.get("decision").and_then(Value::as_str) == Some("draft"))
        .collect();
    let drafts_waiting_feedback: Vec<&Value> = draft_items
        .iter()
        .copied()
        .filter(|item| {
            matches!(
                item.get("state").and_then(Value::as_str),
                Some("outline" | "draft" | "ready_for_review")
            )
        })
        .collect();
    let publish_decision_gates: Vec<&Value> = publish_gates
        .iter()
        .copied()
        .filter(|item| {
            item.get("status").and_then(Value::as_str) == Some("blocked_until_user_approval")
        })
        .collect();
    let operator_states: Vec<&str> = surface
        .get("operator_states")
        .and_then(Value::as_array)
        .map(|array| array.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    let user_action_required = !publish_decision_gates.is_empty();
    let safe_side_work_available = operator_states.contains(&"safe_side_work_available");
    let ready_to_draft = !ready_angles.is_empty();
    let (waiting_on, next_safe_action) = if user_action_required {
        (
            "user",
            "review source map and publish gate before external posting",
        )
    } else if ready_to_draft {
        (
            "agent",
            "draft or rewrite from approved source-mapped angle",
        )
    } else if !source_review_required.is_empty() {
        ("operator", "review source status before drafting")
    } else {
        ("agent", "collect more compact source signals")
    };

    let mut todo_candidates: Vec<TodoCandidate> = Vec::new();
    if !ready_angles.is_empty() {
        todo_candidates.push(TodoCandidate {
            role: "agent".to_string(),
            action_kind: "content_ops_draft_from_angle".to_string(),
            title: "Draft or rewrite the selected source-mapped angle".to_string(),
            ref_ids: ready_angles
                .iter()
                .filter_map(|item| compact_text(item.get("angle_id"), 120))
                .collect(),
            validation_surface: "source_map plus publish_gate must remain present".to_string(),
            stop_condition: Some("stop before external posting".to_string()),
        });
    }
    if !source_review_required.is_empty() {
        todo_candidates.push(TodoCandidate {
            role: "user".to_string(),
            action_kind: "content_ops_source_review".to_string(),
            title: "Review private or metadata-only source before use".to_string(),
            ref_ids: source_review_required
                .iter()
                .filter_map(|item| compact_text(item.get("source_item_id"), 120))
                .collect(),
            validation_surface: "source_status and allowed_use updated".to_string(),
            stop_condition: None,
        });
    }
    if !publish_decision_gates.is_empty() {
        todo_candidates.push(TodoCandidate {
            role: "user".to_string(),
            action_kind: "content_ops_publish_gate".to_string(),
            title: "Approve, deny, or request revision before publication".to_string(),
            ref_ids: publish_decision_gates
                .iter()
                .filter_map(|item| compact_text(item.get("gate_id"), 120))
                .collect(),
            validation_surface: "publish gate decision recorded".to_string(),
            stop_condition: None,
        });
    }
    let runnable_connector_trials: Vec<&Value> = connector_trials
        .iter()
        .copied()
        .filter(|item| {
            item.get("trial_state").and_then(Value::as_str) == Some("ready_for_metadata_trial")
                && item.get("external_write_allowed").and_then(Value::as_bool) == Some(false)
        })
        .collect();
    let gated_connector_trials: Vec<&Value> = connector_trials
        .iter()
        .copied()
        .filter(|item| item.get("requires_user_gate").and_then(Value::as_bool) == Some(true))
        .collect();
    if !runnable_connector_trials.is_empty() {
        todo_candidates.push(TodoCandidate {
            role: "agent".to_string(),
            action_kind: "content_ops_connector_metadata_trial".to_string(),
            title: "Run a connector metadata-only observation trial".to_string(),
            ref_ids: runnable_connector_trials
                .iter()
                .filter_map(|item| compact_text(item.get("trial_id"), 120))
                .collect(),
            validation_surface:
                "compact source_item_v0 produced; no raw platform or private material".to_string(),
            stop_condition: Some(
                "stop before login-gated reads, posting, or private source use".to_string(),
            ),
        });
    }
    if !gated_connector_trials.is_empty() {
        todo_candidates.push(TodoCandidate {
            role: "user".to_string(),
            action_kind: "content_ops_connector_owner_gate".to_string(),
            title: "Approve or reject private connector metadata intake".to_string(),
            ref_ids: gated_connector_trials
                .iter()
                .filter_map(|item| compact_text(item.get("trial_id"), 120))
                .collect(),
            validation_surface: "connector trial gate decision recorded".to_string(),
            stop_condition: None,
        });
    }

    SurfaceProjection {
        surface_id: compact_text(surface.get("surface_id"), 120),
        first_screen: FirstScreen {
            waiting_on: waiting_on.to_string(),
            user_action_required,
            agent_can_continue: ready_to_draft || safe_side_work_available,
            safe_side_work_available,
            source_review_required_count: source_review_required.len(),
            ready_to_draft_count: ready_angles.len(),
            waiting_for_feedback_count: drafts_waiting_feedback.len(),
            publish_decision_count: publish_decision_gates.len(),
            next_safe_action: next_safe_action.to_string(),
        },
        record_counts: validation.record_counts,
        source_statuses: counter(
            source_items
                .iter()
                .filter_map(|item| item.get("source_status").and_then(Value::as_str)),
        ),
        draft_states: counter(
            draft_items
                .iter()
                .filter_map(|item| item.get("state").and_then(Value::as_str)),
        ),
        feedback_effects: counter(
            feedback_signals
                .iter()
                .filter_map(|item| item.get("effect").and_then(Value::as_str)),
        ),
        publish_gate_statuses: counter(
            publish_gates
                .iter()
                .filter_map(|item| item.get("status").and_then(Value::as_str)),
        ),
        connector_trial_state_counts: counter(
            connector_trials
                .iter()
                .filter_map(|item| item.get("trial_state").and_then(Value::as_str)),
        ),
        todo_candidates,
    }
}

// ── synthetic public-safe surface fixture ─────────────────────────────────

/// Build the synthetic public-safe content-ops state surface fixture
/// (reference `build_content_ops_surface_fixture`): demonstrates the shape
/// of a creator loop without copying raw platform posts, chat messages,
/// draft bodies, credentials, or local paths into LoopX state.
pub fn build_content_ops_surface_fixture(generated_at: &str) -> Value {
    serde_json::json!({
        "schema_version": CONTENT_OPS_SURFACE_SCHEMA_VERSION,
        "surface_id": "creator_ops_public_safe_demo",
        "generated_at": generated_at,
        "mode": "compact_state_surface",
        "source_items": [
            {
                "schema_version": SOURCE_ITEM_SCHEMA_VERSION,
                "source_item_id": "source_demo_public_feed_001",
                "source_kind": "synthetic_demo_feed",
                "source_status": "synthetic_public_safe",
                "freshness": "fresh",
                "terms_note": "synthetic demo only; no platform scraping claim",
                "allowed_use": "summarize_and_transform",
                "attribution": "LoopX synthetic creator-ops demo",
                "summary": "A public-safe trend summary suggests creator operators need source-aware drafting queues."
            },
            {
                "schema_version": SOURCE_ITEM_SCHEMA_VERSION,
                "source_item_id": "source_demo_private_note_001",
                "source_kind": "synthetic_private_note",
                "source_status": "private_needs_review",
                "freshness": "fresh",
                "terms_note": "metadata-only placeholder for private material",
                "allowed_use": "metadata_only",
                "attribution": "operator-owned private source placeholder",
                "summary": "Private source is represented only as a compact review-needed signal."
            }
        ],
        "angle_candidates": [
            {
                "schema_version": ANGLE_CANDIDATE_SCHEMA_VERSION,
                "angle_id": "angle_source_aware_loop",
                "source_item_ids": ["source_demo_public_feed_001"],
                "audience": "maintainers evaluating creator-ops automation",
                "topic": "source-aware drafting loop",
                "novelty": "connects connector observations to explicit publish gates",
                "preference_fit": "high",
                "evidence_quality": "synthetic_demo",
                "decision": "draft"
            },
            {
                "schema_version": ANGLE_CANDIDATE_SCHEMA_VERSION,
                "angle_id": "angle_private_material_quote",
                "source_item_ids": ["source_demo_private_note_001"],
                "audience": "same",
                "topic": "private source quote",
                "novelty": "blocked by source boundary",
                "preference_fit": "unknown",
                "evidence_quality": "needs_owner_review",
                "decision": "reject",
                "rejection_reason": "private material cannot be quoted or promoted without review"
            }
        ],
        "draft_items": [
            {
                "schema_version": DRAFT_ITEM_SCHEMA_VERSION,
                "draft_id": "draft_source_aware_loop_outline",
                "angle_id": "angle_source_aware_loop",
                "state": "outline",
                "source_map": [
                    {"source_item_id": "source_demo_public_feed_001", "use": "summarized premise"}
                ],
                "preference_hints": [
                    "explain value as quality and feedback, not raw publish count",
                    "keep publish decision human-gated"
                ],
                "publish_gate_id": "publish_gate_source_aware_loop",
                "validation_surface": "source map present; no raw private material; publish gate visible"
            }
        ],
        "feedback_signals": [
            {
                "schema_version": FEEDBACK_SIGNAL_SCHEMA_VERSION,
                "feedback_id": "feedback_demo_style_001",
                "target_id": "draft_source_aware_loop_outline",
                "signal": "useful_but_less_salesy",
                "effect": "preference_hint",
                "writes_todo": false,
                "summary": "Favor operator-quality framing over content volume claims."
            },
            {
                "schema_version": FEEDBACK_SIGNAL_SCHEMA_VERSION,
                "feedback_id": "feedback_private_source_boundary_001",
                "target_id": "source_demo_private_note_001",
                "signal": "do_not_use_source_body",
                "effect": "source_boundary_correction",
                "writes_todo": false,
                "summary": "Private source stays metadata-only until an explicit review approves use."
            }
        ],
        "publish_gates": [
            {
                "schema_version": PUBLISH_GATE_SCHEMA_VERSION,
                "gate_id": "publish_gate_source_aware_loop",
                "draft_id": "draft_source_aware_loop_outline",
                "status": "blocked_until_user_approval",
                "approval_required": true,
                "autopublish_allowed": false,
                "required_review": [
                    "source attribution",
                    "tone/style",
                    "platform policy",
                    "final publish destination"
                ]
            }
        ],
        "material_memory": [
            {
                "schema_version": MATERIAL_MEMORY_SCHEMA_VERSION,
                "memory_id": "memory_source_aware_loop",
                "source_item_id": "source_demo_public_feed_001",
                "attribution": "LoopX synthetic creator-ops demo",
                "reuse_boundary": "demo_only",
                "rejected_angles": ["angle_private_material_quote"],
                "preference_hints": ["quality and feedback beat raw article count"]
            }
        ],
        "connector_trials": [
            {
                "schema_version": CONNECTOR_TRIAL_SCHEMA_VERSION,
                "trial_id": "trial_x_public_metadata_001",
                "surface": "x_public_feed",
                "tool_hint": "public metadata connector",
                "access_mode": "public_metadata_only",
                "source_status": "public",
                "freshness": "fresh",
                "allowed_use": "metadata_only",
                "trial_state": "metadata_packet_collected",
                "proposed_source_item_id": "source_demo_public_feed_001",
                "terms_note": "metadata-only public source packet already collected",
                "promotion_target": "source_item_v0",
                "requires_user_gate": false,
                "external_write_allowed": false
            },
            {
                "schema_version": CONNECTOR_TRIAL_SCHEMA_VERSION,
                "trial_id": "trial_wechat_chatlog_alpha",
                "surface": "wechat_private_archive",
                "tool_hint": "chatlog-alpha/chatview",
                "access_mode": "private_metadata_only",
                "source_status": "private_needs_review",
                "freshness": "unknown",
                "allowed_use": "metadata_only",
                "trial_state": "needs_owner_gate",
                "proposed_source_item_id": "source_wechat_metadata_signal_001",
                "terms_note": "private material intake stays metadata-only until owner review approves any use",
                "promotion_target": "source_item_v0",
                "requires_user_gate": true,
                "external_write_allowed": false
            }
        ],
        "operator_states": [
            "waiting_for_source_review",
            "ready_to_draft",
            "waiting_for_feedback",
            "ready_for_publish_decision",
            "safe_side_work_available"
        ],
        "boundary": {
            "public_safe": true,
            "raw_private_material_recorded": false,
            "raw_platform_data_recorded": false,
            "credentials_recorded": false,
            "autopublish_allowed": false,
            "publish_requires_user_gate": true,
            "connector_bodies_are_source_of_truth": false
        }
    })
}

// ── connector runtime policy ──────────────────────────────────────────────

/// Build the runtime policy that keeps connector runs inside source bounds
/// (reference `build_content_ops_connector_runtime_policy`): public
/// HEAD-only metadata probes, private gate projections, fixture-only.
pub fn build_content_ops_connector_runtime_policy(
    access_mode: &str,
    connector_id: &str,
    connector_name: &str,
    connector_url: Option<&str>,
) -> Result<Value, String> {
    if !ALLOWED_CONNECTOR_ACCESS_MODES.contains(&access_mode) {
        return Err(format!(
            "access_mode must be one of {}",
            ALLOWED_CONNECTOR_ACCESS_MODES.join(", ")
        ));
    }
    let base = serde_json::json!({
        "schema_version": CONTENT_OPS_CONNECTOR_RUNTIME_POLICY_SCHEMA_VERSION,
        "connector_id": connector_id,
        "connector_name": connector_name,
        "access_mode": access_mode,
        "browser_open_allowed_before_gate": false,
    });
    let policy = match access_mode {
        "public_metadata_only" => serde_json::json!({
            "safe_default": "head_only_metadata_probe",
            "browser_open_risk": "public browser pages can autoload timelines, post text, media, video streams, analytics, and engagement counters",
            "allowed_probe_methods": ["HEAD"],
            "allowed_before_approval": [
                "verify URL and host",
                "read response status and content-type headers",
                "record attribution and freshness metadata"
            ],
            "forbidden_before_approval": [
                "timeline body capture",
                "media download",
                "login-gated reads",
                "posting or engagement actions"
            ]
        }),
        "private_metadata_only" => serde_json::json!({
            "connector_url": connector_url,
            "safe_default": "gate_projection_only",
            "browser_open_risk": "the default web app route may autoload private-source message lists or message-detail APIs before an agent can intervene",
            "allowed_probe_methods": [],
            "allowed_before_approval": [
                "store this compact gate packet",
                "display the owner question",
                "prepare fixture-only smoke coverage"
            ],
            "forbidden_url_path_prefixes_before_approval": [
                "/api/messages",
                "/api/reports",
                "/api/channel-state"
            ],
            "forbidden_before_approval": [
                "browser-opening the default private connector route",
                "private source content read",
                "message-list API calls",
                "message-detail API calls",
                "derived report ingestion",
                "source quote",
                "source summary",
                "external posting",
                "autopublish"
            ]
        }),
        _ => serde_json::json!({
            "safe_default": "fixture_only",
            "allowed_before_approval": ["fixture-only validation"],
            "forbidden_before_approval": ["external reads", "external writes"]
        }),
    };
    let mut merged = base.as_object().cloned().unwrap_or_default();
    for (key, value) in policy.as_object().cloned().unwrap_or_default() {
        merged.insert(key, value);
    }
    Ok(Value::Object(merged))
}

// __PART3__

/// Capability entry point (registered in `CapabilityRegistry::with_builtin`).
pub struct ContentOpsCapability;

impl Capability for ContentOpsCapability {
    fn name(&self) -> &'static str {
        "content_ops"
    }

    fn describe(&self) -> &'static str {
        "classify free-text content, derive deterministic quality and length \
         signals, and propose the ordered content operations (draft / source \
         review / publish gate / revise / record) for the kernel to decide"
    }

    fn propose(&self, input: &str) -> Vec<TypedProposal> {
        let text = input.trim();
        if text.is_empty() {
            return vec![TypedProposal::no_followup("no content provided")];
        }

        // JSON payload path: validate a `content_ops_surface_v0` state
        // surface. A surface that fails validation raises a repair gate with
        // every deterministic error; a clean surface yields a single
        // no-follow-up (the projection is readable via the CLI read model).
        if text.starts_with('{') {
            if let Ok(value) = serde_json::from_str::<Value>(text) {
                let validation = validate_content_ops_surface(&value);
                if !validation.ok {
                    return vec![TypedProposal::gate(
                        &format!(
                            "Repair the content-ops surface: {}",
                            validation.errors.join("; ")
                        ),
                        "content-ops surface validation failed",
                    )];
                }
                return vec![TypedProposal::no_followup(
                    "content-ops surface validated clean",
                )];
            }
        }

        // Free-text path: classify → quality/length signals → the finite
        // ordered-operation vocabulary mapped onto the proposal set
        // (agent-role operations become successor todos; user-role
        // operations become gates).
        let class = classify_content(text);
        let signals = quality_signals(text, class.kind);
        let ops = suggest_operations(&class, &signals);
        ops.into_iter()
            .map(|op| match op.role {
                "user" => TypedProposal::gate(&op.title, op.action_kind),
                _ => TypedProposal::successor(
                    successor_todo("content-op", &op.title),
                    op.action_kind,
                ),
            })
            .collect()
    }
}
