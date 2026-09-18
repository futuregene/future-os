//! Compaction: the bounded projection a session sends in place of its full history.
//!
//! Two strategies, both built from the same protected originals and deterministic
//! tool-evidence index — see `semantic::evidence`:
//!
//!   * `prepare_evidence` — no model call at all;
//!   * `prepare_evidence_with_summary` — the same projection plus a model-written
//!     handoff summary, and the runtime default.

pub(super) use crate::types::ConvertToLLM;
use crate::types::{AgentMessage, ContentBlock, Message};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

mod budget;
mod durable;
mod semantic;
pub use budget::{set_request_budget, trigger_tokens, TARGET_HISTORY};
pub(crate) use durable::prepare_with_journal_and_summary;
pub use durable::CompactionJournal;
pub(crate) use semantic::evidence::remap_evidence_references;

pub(super) const INTERNAL_ANCHOR_METADATA_KEY: &str = "internal_context_anchor";

pub(super) const INTERNAL_CHECKPOINT_METADATA_KEY: &str = "internal_context_checkpoint";

/// Preserve the legacy (reserve, recent) interface while deriving the trigger
/// from 80% of the model window (`budget::trigger_tokens`, which has no absolute
/// cap). `reserve` is threshold headroom, not the model's output cap. Recent
/// history is bounded independently at 8K. Degenerate windows do not invent
/// capacity.
pub fn context_token_budgets(context_window: i32) -> (i32, i32) {
    let window = context_window.max(1);
    if window <= 1 {
        return (0, 0);
    }
    let threshold = trigger_tokens(window as u64).max(1) as i32;
    let reserve_tokens = window - threshold;
    let keep_recent_tokens = (budget::RECENT_HISTORY as i32).min(threshold / 4).max(1);
    (reserve_tokens, keep_recent_tokens)
}

#[cfg(test)]
#[test]
fn degenerate_windows_do_not_invent_reserved_capacity() {
    for window in [-1, 0, 1] {
        assert_eq!(context_token_budgets(window), (0, 0));
    }
    assert_eq!(context_token_budgets(2), (1, 1));
}

pub(super) fn stamp_internal_checkpoint_message(message: &mut AgentMessage, entry_id: &str) {
    let metadata = message.metadata.get_or_insert_with(serde_json::Map::new);
    metadata.insert(
        AgentMessage::JOURNAL_ENTRY_ID_KEY.to_string(),
        serde_json::Value::String(entry_id.to_string()),
    );
    metadata.insert(
        INTERNAL_CHECKPOINT_METADATA_KEY.to_string(),
        serde_json::Value::Bool(true),
    );
}

pub(super) fn is_protected(message: &AgentMessage) -> bool {
    message
        .metadata
        .as_ref()
        .and_then(|m| m.get(INTERNAL_ANCHOR_METADATA_KEY))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

pub(super) fn protected_text(original: &AgentMessage) -> Option<AgentMessage> {
    if !matches!(original.role.as_str(), "user" | "assistant")
        || original
            .metadata
            .as_ref()
            .and_then(|m| m.get(INTERNAL_CHECKPOINT_METADATA_KEY))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    {
        return None;
    }
    let id = original.journal_entry_id()?;
    let content = original
        .content
        .iter()
        .filter(|b| matches!(b, ContentBlock::Text { text } if !text.trim().is_empty()))
        .cloned()
        .collect::<Vec<_>>();
    if content.is_empty() {
        return None;
    }
    let mut metadata = serde_json::Map::new();
    metadata.insert(
        AgentMessage::JOURNAL_ENTRY_ID_KEY.into(),
        serde_json::json!(id),
    );
    metadata.insert(INTERNAL_ANCHOR_METADATA_KEY.into(), serde_json::json!(true));
    Some(AgentMessage {
        role: original.role.clone(),
        content,
        metadata: Some(metadata),
        ..Default::default()
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompactionSettings {
    pub enabled: bool,
    #[serde(rename = "reserveTokens")]
    pub reserve_tokens: i32,
    #[serde(rename = "keepRecentTokens")]
    pub keep_recent_tokens: i32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompactionTrigger {
    Automatic,
    ProviderContextLimit,
    Manual,
    ModelContextDownshift,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompactionPhase {
    PreTurn,
    MidTurn,
    Standalone,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ContextUsage {
    pub input_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub estimated_input_tokens: u64,
    pub context_window: u64,
    #[serde(default)]
    pub fixed_input_tokens: u64,
    #[serde(default)]
    pub output_reserve_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextCheckpoint {
    /// Journal entry that stores this checkpoint. Separate from checkpoint_id
    /// so later checkpoints can use this entry as provenance when chaining.
    pub entry_id: String,
    pub checkpoint_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub covered_from_entry_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cutoff_entry_id: Option<String>,
    pub summary: Vec<ContentBlock>,
    /// Original user/assistant text to retain from the covered prefix. Bodies
    /// remain in the immutable journal, not duplicated inside this checkpoint.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protected_entry_ids: Vec<String>,
    pub tokens_before: u64,
    pub tokens_after: u64,
    pub trigger: CompactionTrigger,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<CompactionPhase>,
    pub algorithm_version: String,
    pub model: String,
    pub context_window: u64,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct ProjectedMessage {
    pub message: AgentMessage,
    pub source_entry_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PromptContext {
    pub messages: Vec<ProjectedMessage>,
    pub usage: ContextUsage,
}

#[derive(Debug, Clone)]
pub enum ContextPreparation {
    Unchanged {
        prompt: PromptContext,
    },
    Compacted {
        prompt: PromptContext,
        checkpoint: Box<ContextCheckpoint>,
    },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ContextError {
    #[error("context compaction found no valid journal boundary")]
    NoValidBoundary,
    #[error("context compaction produced an empty summary")]
    InvalidSummary,
    #[error("context compaction summary failed: {0}")]
    SummaryFailed(String),
    #[error("context compaction was cancelled")]
    Cancelled,
    #[error("context compaction budget exceeded: {0}")]
    BudgetExceeded(String),
    #[error("context compaction made no token progress")]
    NoProgress,
    #[error("context compaction durability: {0}")]
    PersistenceFailed(String),
}

#[derive(Debug, Clone)]
pub struct ContextManager {
    pub enabled: bool,
    pub reserve_tokens: i32,
    pub keep_recent_tokens: i32,
    pub context_window: i32,
    pub model: String,
}

impl ContextManager {
    /// Default runtime compaction (C). The raw journal is used only for bounded
    /// evidence selection; no LLM provider is accepted or called by this path.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_evidence(
        &self,
        prompt: PromptContext,
        raw: &[AgentMessage],
        trigger: CompactionTrigger,
        phase: CompactionPhase,
        instructions: Option<&str>,
        interrupted: &std::sync::atomic::AtomicBool,
        on_started: Option<&(dyn Fn() + Sync)>,
    ) -> Result<ContextPreparation, ContextError> {
        semantic::evidence::prepare(
            self,
            prompt,
            raw,
            trigger,
            phase,
            instructions,
            interrupted,
            on_started,
        )
    }

    /// Summarised: the deterministic projection plus a model-written handoff summary.
    ///
    /// The summary is generated from the material being compressed and receives the
    /// previous summary, so facts accumulate across successive compactions instead of
    /// being rewritten from scratch. When the provider is absent or the call fails,
    /// this commits the deterministic projection; the fallback is reported through
    /// `on_fallback` rather than failing the compaction.
    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_evidence_with_summary(
        &self,
        prompt: PromptContext,
        raw: &[AgentMessage],
        trigger: CompactionTrigger,
        phase: CompactionPhase,
        instructions: Option<&str>,
        interrupted: &std::sync::atomic::AtomicBool,
        on_started: Option<&(dyn Fn() + Sync)>,
        provider: Option<&dyn crate::types::LLMProvider>,
        system_prompt: Option<&str>,
        tools: &[crate::types::ToolDef],
        on_usage: Option<&(dyn Fn(&crate::types::Usage) + Sync)>,
        on_fallback: Option<&(dyn Fn(&str) + Sync)>,
    ) -> Result<ContextPreparation, ContextError> {
        semantic::evidence::prepare_with_handoff_summary(
            self,
            prompt,
            raw,
            trigger,
            phase,
            instructions,
            interrupted,
            on_started,
            provider,
            system_prompt,
            tools,
            on_usage,
            on_fallback,
        )
        .await
    }

    // The legacy semantic entry points (`prepare_semantic`, its `_with_phase`,
    // `_with_phase_and_fallback`, `_with_lifecycle` and `_observed` wrappers, and the
    // synchronous `prepare`) were removed: the runtime has used `prepare_evidence` /
    // `prepare_evidence_with_summary` since `6a83a34a`, and nothing in the workspace
    // called them. The implementation they reached is retained under `#[cfg(test)]` in
    // `semantic.rs` so the tests that characterise legacy A still run; it is not part of
    // a production build.
}

/// The phase a trigger implies. Only the legacy-A tests need this now: the runtime
/// passes the phase explicitly at each call site.
#[cfg(test)]
fn default_phase(trigger: CompactionTrigger) -> CompactionPhase {
    match trigger {
        CompactionTrigger::Manual => CompactionPhase::Standalone,
        CompactionTrigger::ProviderContextLimit => CompactionPhase::MidTurn,
        CompactionTrigger::Automatic | CompactionTrigger::ModelContextDownshift => {
            CompactionPhase::PreTurn
        }
    }
}

pub fn project_prompt_context(
    messages: &[AgentMessage],
    checkpoint: Option<&ContextCheckpoint>,
    input_tokens: Option<u64>,
    context_window: u64,
) -> PromptContext {
    let cutoff_index = checkpoint.and_then(|checkpoint| {
        checkpoint.cutoff_entry_id.as_deref().and_then(|cutoff| {
            messages
                .iter()
                .position(|message| message.journal_entry_id() == Some(cutoff))
        })
    });
    let tail_start = cutoff_index.map_or(0, |index| index.saturating_add(1));
    let mut projected = Vec::with_capacity(messages.len().saturating_sub(tail_start) + 1);
    if let Some(checkpoint) = checkpoint {
        let protected: HashSet<&str> = checkpoint
            .protected_entry_ids
            .iter()
            .map(String::as_str)
            .collect();
        for original in &messages[..tail_start] {
            if original
                .journal_entry_id()
                .is_some_and(|id| protected.contains(id))
            {
                if let Some(message) = protected_text(original) {
                    projected.push(ProjectedMessage {
                        source_entry_ids: vec![message.journal_entry_id().unwrap().to_string()],
                        message,
                    });
                }
            }
        }
        let summary = checkpoint
            .summary
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        if !summary.is_empty() {
            let mut message = AgentMessage::new_user(
                "user",
                serde_json::json!([{
                    "type": "text",
                    "text": format!("[Context compaction: {summary}]")
                }]),
            );
            stamp_internal_checkpoint_message(&mut message, &checkpoint.entry_id);
            projected.push(ProjectedMessage {
                message,
                source_entry_ids: vec![checkpoint.entry_id.clone()],
            });
        }
    }
    projected.extend(messages[tail_start..].iter().cloned().map(|message| {
        let source_entry_ids = message
            .journal_entry_id()
            .map(|id| vec![id.to_string()])
            .unwrap_or_default();
        ProjectedMessage {
            message,
            source_entry_ids,
        }
    }));
    let estimated_input_tokens = projected
        .iter()
        .flat_map(|projected| ConvertToLLM(std::slice::from_ref(&projected.message)))
        .map(|message| estimate_tokens(&message).max(0) as u64)
        .sum();
    PromptContext {
        messages: projected,
        usage: ContextUsage {
            input_tokens,
            estimated_input_tokens,
            context_window,
            ..Default::default()
        },
    }
}

/// EstimateTokens estimates tokens for a single message.
///
/// Uses a Unicode-aware per-character heuristic rather than the previous raw
/// character count (which underestimated CJK text by ~2× and overestimated
/// ASCII by ~4×):
///   - CJK characters (U+4E00–U+9FFF, U+3400–U+4DBF, U+3040–U+30FF,
///     U+AC00–U+D7AF, U+F900–U+FAFF, U+20000–U+2A6DF): ~1.5 tokens/char.
///     Modern BPE tokenizers average ~1 token per common CJK char; 1.5 keeps
///     a conservative margin so compaction triggers early rather than late.
///   - ASCII: ~0.25 tokens per character (≈ 4 chars per token).
///   - Everything else: ~0.5 tokens per character.
///
/// The real token count depends on the model's BPE tokenizer, but classifying
/// each character avoids the worst-case 8× underestimation of the old
/// char-count approach for Chinese text.
pub fn estimate_tokens(msg: &Message) -> i32 {
    let mut estimated: i32 = content_text_pieces(&msg.content)
        .iter()
        .map(|s| estimate_text_tokens(s))
        .sum();
    if msg.role.as_str() == "assistant" {
        if let Some(ref tcs) = msg.tool_calls {
            for tc in tcs {
                estimated += estimate_text_tokens(&tc.function.name);
                if let serde_json::Value::String(ref s) = tc.function.arguments {
                    estimated += estimate_text_tokens(s);
                }
            }
        }
    }
    estimated
}

/// Collect the text pieces of a message's content, whether serialized as a
/// single string or as a content-parts array.
fn content_text_pieces(content: &Option<serde_json::Value>) -> Vec<&str> {
    match content {
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_object()?.get("text")?.as_str())
            .collect(),
        Some(serde_json::Value::String(s)) => vec![s.as_str()],
        _ => Vec::new(),
    }
}

/// Whether `c` falls in a CJK Unicode range whose characters typically
/// tokenize to ~1 token each (rather than ~4 chars/token for ASCII).
fn is_cjk(c: char) -> bool {
    matches!(
        c as u32,
        0x4E00..=0x9FFF     // CJK Unified Ideographs
        | 0x3400..=0x4DBF   // CJK Extension A
        | 0x3040..=0x30FF   // Hiragana + Katakana
        | 0xAC00..=0xD7AF   // Hangul Syllables
        | 0xF900..=0xFAFF   // CJK Compatibility Ideographs
        | 0x20000..=0x2A6DF // CJK Extension B
    )
}

/// Estimate tokens for a text by classifying each character: CJK ~1.5
/// tokens/char, ASCII ~0.25, everything else ~0.5. Errors toward
/// overestimate so compaction triggers early rather than late.
fn char_token_quarters(c: char) -> u64 {
    if is_cjk(c) {
        6
    } else if c.is_ascii() {
        1
    } else {
        2
    }
}

fn estimate_text_tokens(text: &str) -> i32 {
    let quarters = text
        .chars()
        .fold(0_u64, |n, c| n.saturating_add(char_token_quarters(c)));
    quarters
        .saturating_add(3)
        .div_euclid(4)
        .min(i32::MAX as u64) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_msg(role: &str, text: &str) -> Message {
        Message {
            role: role.to_string(),
            content: Some(serde_json::Value::String(text.to_string())),
            ..Default::default()
        }
    }

    #[test]
    fn cjk_text_estimates_higher_than_ascii_of_same_length() {
        // 100 CJK chars ≈ 150 tokens; 100 ASCII chars ≈ 25 tokens.
        let cjk = estimate_tokens(&text_msg("user", &"汉".repeat(100)));
        let ascii = estimate_tokens(&text_msg("user", &"a".repeat(100)));
        assert_eq!(cjk, 150);
        assert_eq!(ascii, 25);
        assert!(cjk > ascii * 3, "CJK must weigh far more than ASCII");
    }

    #[test]
    fn is_cjk_covers_every_listed_range() {
        for c in [
            '\u{4E00}',  // CJK Unified Ideographs
            '\u{3400}',  // Extension A
            '\u{3040}',  // Hiragana
            '\u{30FF}',  // Katakana
            '\u{AC00}',  // Hangul Syllables
            '\u{F900}',  // CJK Compatibility Ideographs
            '\u{20000}', // Extension B
        ] {
            assert!(is_cjk(c), "{c:?} must classify as CJK");
        }
        assert!(!is_cjk('a'));
        assert!(!is_cjk('\u{0400}')); // Cyrillic is not CJK
    }

    #[test]
    fn mixed_content_and_tool_args_are_classified_per_char() {
        // Content-parts array form.
        let msg = Message {
            role: "user".to_string(),
            content: Some(serde_json::json!([
                {"type": "text", "text": "你好"},        // 2 CJK ≈ 3
                {"type": "text", "text": "abcd"},        // 4 ASCII ≈ 1
            ])),
            ..Default::default()
        };
        assert_eq!(estimate_tokens(&msg), 4);

        // Assistant tool-call arguments are estimated with the same
        // per-char classifier (a CJK-heavy args payload is not undercounted).
        let args = serde_json::Value::String("命令".to_string());
        let tool = estimate_text_tokens(args.as_str().unwrap());
        assert_eq!(tool, 3); // 2 CJK chars × 1.5
    }

    #[test]
    fn non_cjk_non_ascii_falls_back_to_half_token() {
        // Cyrillic: not CJK, not ASCII → 0.5 tokens/char.
        assert_eq!(estimate_tokens(&text_msg("user", &"Привет".repeat(2))), 6);
    }

    // ─── should_compact ────────────────────────────────────────────────────

    #[test]
    fn context_budgets_leave_room_for_small_model_windows() {
        for window in [4_096, 8_192, 16_384, 64_000, 1_000_000] {
            let (reserve, keep_recent) = context_token_budgets(window);
            assert!(reserve > 0, "window {window} must retain a reserve");
            assert!(
                reserve < window,
                "window {window} must retain usable prompt space"
            );
            assert!(keep_recent > 0);
            assert!(
                keep_recent <= window - reserve,
                "recent tail must fit beside the reserve for window {window}"
            );
        }
    }

    // ─── estimate_context_tokens ───────────────────────────────────────────

    #[test]
    fn checkpoint_projection_preserves_recent_provider_and_tool_metadata() {
        let mut covered = AgentMessage::new_user("user", serde_json::json!("old"));
        covered.ensure_journal_entry_id();
        let covered_id = covered.journal_entry_id().unwrap().to_string();
        let mut provider_metadata = crate::types::ProviderMetadata::new();
        provider_metadata.insert("openai".into(), serde_json::json!({"id": "rs_1"}));
        let mut recent = AgentMessage {
            role: "assistant".into(),
            content: vec![
                ContentBlock::reasoning("reason", provider_metadata.clone()),
                ContentBlock::tool_call(
                    "call-1",
                    "read",
                    serde_json::json!({"path": "a.rs"}),
                    provider_metadata,
                ),
            ],
            ..Default::default()
        };
        recent.ensure_journal_entry_id();
        let checkpoint = ContextCheckpoint {
            entry_id: "checkpoint-entry".into(),
            protected_entry_ids: Vec::new(),
            checkpoint_id: "cp-1".into(),
            covered_from_entry_id: Some(covered_id.clone()),
            cutoff_entry_id: Some(covered_id),
            summary: vec![ContentBlock::text("summary")],
            tokens_before: 100,
            tokens_after: 10,
            trigger: CompactionTrigger::Automatic,
            phase: None,
            algorithm_version: "v2".into(),
            model: "model".into(),
            context_window: 200,
            created_at: chrono::Utc::now(),
        };

        let projected =
            project_prompt_context(&[covered, recent.clone()], Some(&checkpoint), None, 200);
        assert_eq!(projected.messages.len(), 2);
        assert_eq!(
            serde_json::to_value(&projected.messages[1].message).unwrap(),
            serde_json::to_value(&recent).unwrap()
        );
    }

    // ─── content_text_pieces ───────────────────────────────────────────────

    #[test]
    fn content_text_pieces_array() {
        let content = Some(serde_json::json!([
            {"type": "text", "text": "hello"},
            {"type": "text", "text": " world"}
        ]));
        let pieces = content_text_pieces(&content);
        assert_eq!(pieces, vec!["hello", " world"]);
    }

    #[test]
    fn content_text_pieces_string() {
        let content = Some(serde_json::json!("plain string"));
        let pieces = content_text_pieces(&content);
        assert_eq!(pieces, vec!["plain string"]);
    }

    #[test]
    fn content_text_pieces_none() {
        assert!(content_text_pieces(&None).is_empty());
    }

    // ─── is_cjk ────────────────────────────────────────────────────────────

    #[test]
    fn is_cjk_detects_all_ranges() {
        assert!(is_cjk('汉')); // U+6C49 — CJK Unified
        assert!(is_cjk('あ')); // U+3042 — Hiragana
        assert!(is_cjk('が')); // U+304C — Hiragana
        assert!(!is_cjk('a'));
        assert!(!is_cjk('@'));
    }

    // ─── estimate_text_tokens ──────────────────────────────────────────────

    #[test]
    fn estimate_text_tokens_empty() {
        assert_eq!(estimate_text_tokens(""), 0);
    }

    // ─── adjust_cut_for_tool_context ───────────────────────────────────────

    // ─── find_valid_cut_points ─────────────────────────────────────────────

    // ─── find_cut_point ────────────────────────────────────────────────────

    // ─── extract_file_operations ───────────────────────────────────────────

    // ─── compact / compact_from ────────────────────────────────────────────

    #[test]
    fn checkpoint_projection_ignores_non_text_summary_blocks() {
        let checkpoint = ContextCheckpoint {
            entry_id: "e".to_string(),
            protected_entry_ids: Vec::new(),
            checkpoint_id: "cp".to_string(),
            covered_from_entry_id: None,
            cutoff_entry_id: None,
            summary: vec![
                ContentBlock::text("visible"),
                ContentBlock::reasoning("hidden", Default::default()),
            ],
            tokens_before: 100,
            tokens_after: 10,
            trigger: CompactionTrigger::Automatic,
            phase: None,
            algorithm_version: "v1".to_string(),
            model: "model".to_string(),
            context_window: 200,
            created_at: chrono::Utc::now(),
        };

        let projected = project_prompt_context(&[], Some(&checkpoint), None, 200);
        assert_eq!(projected.messages.len(), 1);
        let text = projected.messages[0].message.text();
        assert!(text.contains("visible"));
        assert!(!text.contains("hidden"));
    }
}
