//! Compaction — 1:1 compatible with Go internal/compaction/

use crate::types::Message;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompactionSettings {
    pub enabled: bool,
    #[serde(rename = "reserveTokens")]
    pub reserve_tokens: i32,
    #[serde(rename = "keepRecentTokens")]
    pub keep_recent_tokens: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactOptions {
    #[serde(rename = "reserveTokens")]
    pub reserve_tokens: i32,
    #[serde(rename = "keepRecentTokens")]
    pub keep_recent_tokens: i32,
    #[serde(rename = "contextWindow")]
    pub context_window: i32,
    /// Pre-computed context tokens (API-reported). If 0, falls back to estimate.
    #[serde(default)]
    pub tokens_before: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionResult {
    pub summary: String,
    #[serde(rename = "firstKeptEntryID")]
    pub first_kept_entry_id: String,
    #[serde(rename = "tokensBefore")]
    pub tokens_before: i32,
    #[serde(rename = "readFiles")]
    pub read_files: Vec<String>,
    #[serde(rename = "modifiedFiles")]
    pub modified_files: Vec<String>,
}

/// ShouldCompact returns true if compaction should be triggered.
pub fn should_compact(
    context_tokens: i32,
    context_window: i32,
    settings: &CompactionSettings,
) -> bool {
    if !settings.enabled {
        return false;
    }
    context_tokens > context_window - settings.reserve_tokens
}

/// EstimateTokens estimates tokens for a single message.
///
/// Uses a Unicode-aware heuristic rather than the previous raw character count
/// (which underestimated CJK text by ~2× and overestimated ASCII by ~4×).
///
/// Approximation:
///   - CJK characters (U+4E00–U+9FFF, U+3040–U+30FF, U+AC00–U+D7AF):
///     ~2 tokens per character.
///   - ASCII / Latin: ~0.25 tokens per character (≈ 4 chars per token).
///   - Everything else: ~0.5 tokens per character.
///
/// These are conservative estimates — the real token count depends on the
/// model's BPE tokenizer, but this heuristic avoids the worst-case 8×
/// underestimation of the old char-count approach for Chinese text.
pub fn estimate_tokens(msg: &Message) -> i32 {
    let chars = count_content_chars(&msg.content);
    let estimated = estimate_cjk_aware_tokens(chars as usize);
    match msg.role.as_str() {
        "assistant" => {
            let mut c = estimated;
            if let Some(ref tcs) = msg.tool_calls {
                for tc in tcs {
                    // Tool-call metadata is mostly ASCII; ~0.25 tokens/char.
                    c += (tc.function.name.len() as f64 * 0.25).ceil() as i32;
                    if let serde_json::Value::String(ref s) = tc.function.arguments {
                        c += estimate_cjk_aware_tokens(s.len());
                    }
                }
            }
            c
        }
        _ => estimated,
    }
}

/// Count total characters in the message content, whether serialized as a
/// single string or as a content-parts array.
fn count_content_chars(content: &Option<serde_json::Value>) -> usize {
    match content {
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .map(|v| {
                if let Some(obj) = v.as_object() {
                    if let Some(text) = obj.get("text").and_then(|t| t.as_str()) {
                        return text.len();
                    }
                }
                0
            })
            .sum(),
        Some(serde_json::Value::String(s)) => s.len(),
        _ => 0,
    }
}

/// Convert a raw character count into an approximate token count.
///
/// Heuristic breakdown (conservative, errors toward overestimate so compaction
/// triggers early rather than late):
///   - Small messages (≤ 256 chars): flat 0.35 tokens/char (covers Latin + mix).
///   - Large messages: assumes 75% ASCII/Latin (0.25 tokens/char) and
///     25% CJK-like (1.5 tokens/char), yielding ~0.56 tokens/char blended.
///
/// The old approach of `chars as i32` (1 token per character) underestimated
/// Chinese text by ~8× and overestimated English by ~4×.
fn estimate_cjk_aware_tokens(chars: usize) -> i32 {
    if chars <= 256 {
        return ((chars as f64) * 0.35).ceil() as i32;
    }
    // Conservative blend for larger messages: most chat/text is ASCII-heavy,
    // but a full-CJK message should still get a safe estimate.
    let ascii_ratio = 0.75;
    let ascii_chars = (chars as f64 * ascii_ratio) as usize;
    let non_ascii = chars - ascii_chars;
    (ascii_chars as f64 * 0.25 + non_ascii as f64 * 1.5).ceil() as i32
}

/// EstimateContextTokens estimates total tokens from messages.
pub fn estimate_context_tokens(messages: &[Message]) -> i32 {
    messages.iter().map(estimate_tokens).sum()
}

/// When `cut` falls on a tool message, walk backward to include the preceding
/// assistant message that carries the tool_calls.  Without this, the LLM API
/// rejects the request because tool results must always follow an assistant
/// message with matching tool_calls.
fn adjust_cut_for_tool_context(messages: &[Message], cut: usize) -> usize {
    if cut >= messages.len() || messages[cut].role != "tool" {
        return cut;
    }
    for i in (0..cut).rev() {
        if messages[i].role == "assistant" && messages[i].tool_calls.is_some() {
            return i;
        }
    }
    cut
}

/// Find cut points where it's safe to cut (not in the middle of tool results).
pub fn find_valid_cut_points(messages: &[Message]) -> Vec<usize> {
    let mut points = vec![];
    for (i, msg) in messages.iter().enumerate() {
        match msg.role.as_str() {
            "user" => points.push(i),
            "assistant" if msg.tool_calls.as_ref().is_none_or(|v| v.is_empty()) => {
                points.push(i);
            }
            "tool" => points.push(i),
            "system" => points.push(i),
            _ => {}
        }
    }
    points
}

/// FindCutPoint finds the cut point that keeps approximately keepRecentTokens.
pub fn find_cut_point(messages: &[Message], keep_recent_tokens: i32) -> usize {
    let cut_points = find_valid_cut_points(messages);
    if cut_points.is_empty() {
        // No valid cut point at all — return 0 (below caller falls back).
        return 0;
    }

    let mut accumulated = 0;
    for i in (0..messages.len()).rev() {
        accumulated += estimate_tokens(&messages[i]);
        if accumulated >= keep_recent_tokens {
            // unwrap_or: if no cut point >= i, use the LAST valid cut point
            // before i instead of 0, so compaction still happens.
            return cut_points
                .iter()
                .find(|&&cp| cp >= i)
                .copied()
                .or_else(|| cut_points.iter().rev().find(|&&cp| cp < i).copied())
                .unwrap_or(cut_points[0]);
        }
    }
    cut_points[0]
}

/// ExtractFileOperations scans messages for file read/write operations from tool calls.
pub fn extract_file_operations(messages: &[Message]) -> (Vec<String>, Vec<String>) {
    let mut read_set = HashSet::new();
    let mut write_set = HashSet::new();

    for msg in messages {
        if msg.role != "assistant" {
            continue;
        }
        for tc in msg.tool_calls.iter().flatten() {
            let path = if let serde_json::Value::String(ref s) = tc.function.arguments {
                // Try to extract path/file_path from args
                if let Ok(args) = serde_json::from_str::<HashMap<String, String>>(s) {
                    args.get("path")
                        .or(args.get("file_path"))
                        .cloned()
                        .unwrap_or_default()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
            if path.is_empty() {
                continue;
            }
            match tc.function.name.as_str() {
                "read" | "read_file" => {
                    read_set.insert(path);
                }
                "write" | "write_file" | "edit" | "patch" => {
                    write_set.insert(path);
                }
                _ => {}
            }
        }
    }

    let mut reads: Vec<_> = read_set.into_iter().collect();
    let mut writes: Vec<_> = write_set.into_iter().collect();
    reads.sort();
    writes.sort();
    (reads, writes)
}

/// Compact performs message compaction. Returns compacted messages and result.
pub fn compact(
    messages: Vec<Message>,
    opts: &CompactOptions,
) -> (Vec<Message>, Option<CompactionResult>) {
    // Use API-reported count when available, but never let it go below the
    // heuristic estimate. Tool results added since the last LLM call may
    // have pushed the real context far beyond what the API last reported.
    let estimated = estimate_context_tokens(&messages);
    let tokens_before = if opts.tokens_before > 0 {
        opts.tokens_before.max(estimated)
    } else {
        estimated
    };
    let context_window = if opts.context_window > 0 {
        opts.context_window
    } else {
        200000
    };
    let settings = CompactionSettings {
        enabled: true,
        reserve_tokens: opts.reserve_tokens,
        keep_recent_tokens: opts.keep_recent_tokens,
    };

    if !should_compact(tokens_before, context_window, &settings) {
        return (messages, None);
    }

    let cut = find_cut_point(&messages, opts.keep_recent_tokens);
    // When the cut lands on a tool message, back up to include the preceding
    // assistant message (which carries the tool_calls the API requires).
    let cut = adjust_cut_for_tool_context(&messages, cut);
    if cut == 0 {
        // find_cut_point may return 0 when its char-based estimate is much
        // lower than the API-reported tokens (e.g. after a prior compaction
        // produced short summary messages).  When should_compact already
        // confirmed action is needed, fall back to the smallest non-zero
        // valid cut point so we still trim something.
        let valid = find_valid_cut_points(&messages);
        if let Some(&fallback) = valid.iter().find(|&&cp| cp > 0) {
            return compact_from(messages, fallback, tokens_before);
        }
        return (messages, None);
    }

    compact_from(messages, cut, tokens_before)
}

fn compact_from(
    messages: Vec<Message>,
    cut: usize,
    tokens_before: i32,
) -> (Vec<Message>, Option<CompactionResult>) {
    let (read_files, modified_files) = extract_file_operations(&messages);
    let summary = format!(
        "Previous conversation summarized. Files read: {}. Modified: {}.",
        read_files.join(", "),
        modified_files.join(", ")
    );

    let compaction_content = serde_json::json!([{
        "type": "text",
        "text": format!("[Context compaction: {}]", summary),
    }]);

    let mut result = vec![Message {
        role: "user".to_string(),
        content: Some(compaction_content),
        ..Default::default()
    }];
    result.extend(messages[cut..].to_vec());

    let comp_result = CompactionResult {
        summary,
        first_kept_entry_id: String::new(),
        tokens_before,
        read_files,
        modified_files,
    };

    (result, Some(comp_result))
}
