//! Operator inbox (G-15) — LoopX
//! `control_plane/work_items/operator_inbox.py`, natively (compact set). A
//! content-free control-plane read model over provider-owned inbox events:
//! pending operator messages are classified into attention kinds
//! (`direct_question` / `direct_mention` / `reply_to_operator`) and the
//! urgency projection drives the operator triage surface. Event content is
//! never returned in the projection (local_private_content_returned: false).

use std::path::{Path, PathBuf};

use serde::Serialize;

pub const OPERATOR_INBOX_URGENCY_SCHEMA_VERSION: &str = "operator_inbox_urgency_v0";
pub const CAPTURE_SCOPES: [&str; 2] = ["addressed_only", "configured_chat_all"];

/// Inbox capture config (LoopX operator inbox config).
#[derive(Debug, Clone, Serialize)]
pub struct OperatorInboxConfig {
    pub enabled: bool,
    pub capture_scope: String,
    pub inbox_dir: String,
    pub operator_display_name: String,
    pub reply_enabled: bool,
}

/// One pending inbox event (content held locally, never projected).
#[derive(Debug, Clone)]
pub struct OperatorInboxEvent {
    pub message_id: String,
    pub create_time: String,
    pub content: String,
    pub reply_context_verified: bool,
    pub reply_to_operator: bool,
}

/// Attention kind of a pending event (LoopX operator_inbox_attention_kind).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatorAttentionKind {
    DirectQuestion,
    DirectMention,
    ReplyToOperator,
}

impl OperatorAttentionKind {
    pub fn label(&self) -> &'static str {
        match self {
            OperatorAttentionKind::DirectQuestion => "direct_question",
            OperatorAttentionKind::DirectMention => "direct_mention",
            OperatorAttentionKind::ReplyToOperator => "reply_to_operator",
        }
    }
}

/// Classify a pending event's attention kind. An explicit reply to the
/// operator always counts; otherwise the message must be addressed to the
/// operator (mention/question signals) depending on the capture scope.
pub fn operator_inbox_attention_kind(
    event: &OperatorInboxEvent,
    operator_display_name: &str,
    capture_scope: &str,
) -> Option<OperatorAttentionKind> {
    if event.reply_context_verified && event.reply_to_operator {
        return Some(OperatorAttentionKind::ReplyToOperator);
    }
    let content = event.content.to_lowercase();
    let operator_name = operator_display_name.trim().to_lowercase();
    let explicit_mention =
        !operator_name.is_empty() && content.contains('@') && content.contains(&operator_name);
    let loop_mention = content.contains('@') && content.contains("future-loop");
    if capture_scope != "addressed_only" && !explicit_mention && !loop_mention {
        return None;
    }
    if content.contains('?') || content.contains('？') {
        return Some(OperatorAttentionKind::DirectQuestion);
    }
    if explicit_mention || loop_mention {
        return Some(OperatorAttentionKind::DirectMention);
    }
    None
}

/// The urgency projection (LoopX project_operator_inbox_urgency).
#[derive(Debug, Clone, Serialize)]
pub struct OperatorInboxUrgency {
    pub schema_version: String,
    pub enabled: bool,
    pub pending_count: usize,
    pub direct_question_count: usize,
    pub direct_mention_count: usize,
    pub reply_to_operator_count: usize,
    pub attention_required_count: usize,
    pub reply_due: bool,
    pub local_private_content_returned: bool,
}

/// Project the inbox urgency from config + pending events.
pub fn project_operator_inbox_urgency(
    config: &OperatorInboxConfig,
    pending: &[OperatorInboxEvent],
) -> OperatorInboxUrgency {
    if !config.enabled {
        return OperatorInboxUrgency {
            schema_version: OPERATOR_INBOX_URGENCY_SCHEMA_VERSION.to_string(),
            enabled: false,
            pending_count: 0,
            direct_question_count: 0,
            direct_mention_count: 0,
            reply_to_operator_count: 0,
            attention_required_count: 0,
            reply_due: false,
            local_private_content_returned: false,
        };
    }
    let kinds: Vec<Option<OperatorAttentionKind>> = pending
        .iter()
        .map(|e| {
            operator_inbox_attention_kind(e, &config.operator_display_name, &config.capture_scope)
        })
        .collect();
    let direct_question_count = kinds
        .iter()
        .filter(|k| **k == Some(OperatorAttentionKind::DirectQuestion))
        .count();
    let direct_mention_count = kinds
        .iter()
        .filter(|k| **k == Some(OperatorAttentionKind::DirectMention))
        .count();
    let reply_to_operator_count = kinds
        .iter()
        .filter(|k| **k == Some(OperatorAttentionKind::ReplyToOperator))
        .count();
    let attention_required_count =
        direct_question_count + direct_mention_count + reply_to_operator_count;
    OperatorInboxUrgency {
        schema_version: OPERATOR_INBOX_URGENCY_SCHEMA_VERSION.to_string(),
        enabled: true,
        pending_count: pending.len(),
        direct_question_count,
        direct_mention_count,
        reply_to_operator_count,
        attention_required_count,
        reply_due: attention_required_count > 0 && config.reply_enabled,
        local_private_content_returned: false,
    }
}

/// Load pending inbox events from `<project>/.future/loop/inbox/*.json`
/// (project-local pending events). Path safety: the inbox must stay under
/// `.future/loop/inbox` inside the project; `..` components and absolute
/// paths are rejected; malformed files are skipped.
pub fn load_pending_inbox_events(
    project: &str,
    inbox_rel: &str,
) -> Result<Vec<OperatorInboxEvent>, String> {
    let root = Path::new(project);
    let raw = inbox_rel.trim();
    if raw.starts_with('/') || raw.starts_with('\\') {
        return Err("operator inbox path must stay under .future/loop/inbox".into());
    }
    let rel = raw.replace('\\', "/");
    let rel_path = Path::new(&rel);
    if rel_path.is_absolute()
        || rel_path
            .components()
            .any(|c| c == std::path::Component::ParentDir)
    {
        return Err("operator inbox path must stay under .future/loop/inbox".into());
    }
    let inbox = root.join(".future").join("loop").join(rel_path);
    let canonical_inbox = root.join(".future").join("loop").join("inbox");
    if !inbox.starts_with(&canonical_inbox) {
        return Err("operator inbox path must stay under .future/loop/inbox".into());
    }
    if !inbox.is_dir() {
        return Ok(vec![]);
    }
    let mut events = vec![];
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&inbox)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|e| e == "json").unwrap_or(false))
        .collect();
    paths.sort();
    for path in paths {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let message_id = value
            .get("message_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let content = value
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if message_id.is_empty() || content.is_empty() {
            continue;
        }
        let parent_id = value
            .get("parent_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        events.push(OperatorInboxEvent {
            message_id,
            create_time: value
                .get("create_time")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            content: content.chars().take(1200).collect(),
            reply_context_verified: value
                .get("reply_context_verified")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            reply_to_operator: value
                .get("reply_context_verified")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
                && !parent_id.is_empty()
                && value
                    .get("is_reply")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
        });
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(content: &str, reply: bool, verified: bool) -> OperatorInboxEvent {
        OperatorInboxEvent {
            message_id: "m1".into(),
            create_time: "2026-01-01T00:00:00Z".into(),
            content: content.into(),
            reply_context_verified: verified,
            reply_to_operator: reply,
        }
    }

    fn config(enabled: bool, scope: &str, name: &str, reply_enabled: bool) -> OperatorInboxConfig {
        OperatorInboxConfig {
            enabled,
            capture_scope: scope.into(),
            inbox_dir: "inbox".into(),
            operator_display_name: name.into(),
            reply_enabled,
        }
    }

    #[test]
    fn direct_question_is_classified() {
        let e = event("@operator 这个结论成立吗？", false, false);
        assert_eq!(
            operator_inbox_attention_kind(&e, "operator", "addressed_only"),
            Some(OperatorAttentionKind::DirectQuestion)
        );
    }

    #[test]
    fn unaddressed_message_ignored_in_addressed_scope() {
        let e = event("hello world", false, false);
        assert_eq!(
            operator_inbox_attention_kind(&e, "operator", "addressed_only"),
            None
        );
        // configured_chat_all captures non-addressed content too (mention).
        assert_eq!(
            operator_inbox_attention_kind(&e, "operator", "configured_chat_all"),
            None
        );
    }

    #[test]
    fn reply_to_operator_is_always_attention() {
        let e = event("ok done", true, true);
        assert_eq!(
            operator_inbox_attention_kind(&e, "operator", "addressed_only"),
            Some(OperatorAttentionKind::ReplyToOperator)
        );
    }

    #[test]
    fn urgency_projection_counts_and_never_leaks_content() {
        let cfg = config(true, "addressed_only", "operator", true);
        let pending = vec![
            event("@operator 怎么办？", false, false),
            event("@operator ping", false, false),
            event("done", true, true),
        ];
        let urgency = project_operator_inbox_urgency(&cfg, &pending);
        assert_eq!(urgency.pending_count, 3);
        assert_eq!(urgency.direct_question_count, 1);
        assert_eq!(urgency.direct_mention_count, 1);
        assert_eq!(urgency.reply_to_operator_count, 1);
        assert_eq!(urgency.attention_required_count, 3);
        assert!(urgency.reply_due);
        assert!(!urgency.local_private_content_returned);
    }

    #[test]
    fn disabled_inbox_is_empty() {
        let cfg = config(false, "addressed_only", "operator", false);
        let urgency = project_operator_inbox_urgency(&cfg, &[]);
        assert!(!urgency.enabled);
        assert_eq!(urgency.pending_count, 0);
    }

    #[test]
    fn inbox_path_cannot_escape_project() {
        assert!(load_pending_inbox_events("/tmp", "../../etc").is_err());
    }

    #[test]
    fn inbox_rel_must_stay_under_the_canonical_inbox_dir() {
        // A clean relative path that is not `inbox[..]` is still rejected.
        let err = load_pending_inbox_events("/tmp", "elsewhere").unwrap_err();
        assert!(err.contains("must stay under"), "{err}");
    }

    #[test]
    fn load_skips_unreadable_and_non_object_files() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = dir.path().join(".future/loop/inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        // Invalid JSON → skipped by the parse guard.
        std::fs::write(inbox.join("bad.json"), "not json").unwrap();
        // A directory named *.json → read_to_string fails → skipped.
        std::fs::create_dir(inbox.join("adir.json")).unwrap();
        let events = load_pending_inbox_events(&dir.path().to_string_lossy(), "inbox").unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn verified_reply_flags_reply_to_operator() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = dir.path().join(".future/loop/inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        std::fs::write(
            inbox.join("m1.json"),
            r#"{"message_id":"m1","content":"done","parent_id":"p1","reply_context_verified":true,"is_reply":true}"#,
        )
        .unwrap();
        std::fs::write(
            inbox.join("m2.json"),
            r#"{"message_id":"m2","content":"fyi","parent_id":"p1","reply_context_verified":true,"is_reply":false}"#,
        )
        .unwrap();
        let events = load_pending_inbox_events(&dir.path().to_string_lossy(), "inbox").unwrap();
        assert_eq!(events.len(), 2);
        assert!(events[0].reply_to_operator);
        assert!(!events[1].reply_to_operator);
    }
}
