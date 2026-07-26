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
fn estimate_text_tokens(text: &str) -> i32 {
    let mut tokens = 0.0f64;
    for c in text.chars() {
        tokens += if is_cjk(c) {
            1.5
        } else if c.is_ascii() {
            0.25
        } else {
            0.5
        };
    }
    tokens.ceil() as i32
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
    fn should_compact_disabled() {
        let settings = CompactionSettings {
            enabled: false,
            reserve_tokens: 1000,
            keep_recent_tokens: 5000,
        };
        assert!(!should_compact(200_000, 128_000, &settings));
    }

    #[test]
    fn should_compact_when_exceeding_threshold() {
        let settings = CompactionSettings {
            enabled: true,
            reserve_tokens: 8000,
            keep_recent_tokens: 5000,
        };
        // context_tokens (120_000) > context_window (128_000) - reserve (8000) = 120_000
        // 120_000 > 120_000 is false, so not triggered
        assert!(!should_compact(120_000, 128_000, &settings));
        // 120_001 > 120_000 → triggers
        assert!(should_compact(120_001, 128_000, &settings));
    }

    #[test]
    fn should_compact_under_threshold() {
        let settings = CompactionSettings {
            enabled: true,
            reserve_tokens: 8000,
            keep_recent_tokens: 5000,
        };
        assert!(!should_compact(100_000, 128_000, &settings));
    }

    // ─── estimate_context_tokens ───────────────────────────────────────────

    #[test]
    fn estimate_context_tokens_sums_messages() {
        let msgs = vec![
            text_msg("user", &"a".repeat(100)),     // 25 tokens
            text_msg("assistant", &"b".repeat(40)), // 10 tokens
        ];
        assert_eq!(estimate_context_tokens(&msgs), 35);
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

    #[test]
    fn adjust_cut_after_tool_backs_up_to_assistant_with_tool_calls() {
        let msgs = vec![
            text_msg("user", "hello"),
            Message {
                role: "assistant".to_string(),
                content: None,
                tool_calls: Some(vec![]),
                ..Default::default()
            },
            text_msg("tool", "result"),
        ];
        // Cut on the tool msg (index 2), should back up to assistant (index 1)
        assert_eq!(adjust_cut_for_tool_context(&msgs, 2), 1);
    }

    #[test]
    fn adjust_cut_on_user_unchanged() {
        let msgs = vec![text_msg("user", "hello")];
        assert_eq!(adjust_cut_for_tool_context(&msgs, 0), 0);
    }

    #[test]
    fn adjust_cut_out_of_bounds_returns_original() {
        let msgs: Vec<Message> = vec![];
        assert_eq!(adjust_cut_for_tool_context(&msgs, 5), 5);
    }

    // ─── find_valid_cut_points ─────────────────────────────────────────────

    #[test]
    fn find_valid_cut_points_all_types() {
        let msgs = vec![
            text_msg("system", "prompt"),
            text_msg("user", "question"),
            text_msg("assistant", "answer"),
            text_msg("tool", "output"),
        ];
        let points = find_valid_cut_points(&msgs);
        assert_eq!(points, vec![0, 1, 2, 3]);
    }

    #[test]
    fn find_valid_cut_points_excludes_assistant_with_tool_calls() {
        let msgs = vec![
            text_msg("user", "q"),
            Message {
                role: "assistant".to_string(),
                content: None,
                tool_calls: Some(vec![crate::types::ToolCall {
                    id: "t1".to_string(),
                    call_type: "function".to_string(),
                    function: crate::types::ToolCallFn {
                        name: "shell".to_string(),
                        arguments: serde_json::json!("{}"),
                    },
                }]),
                ..Default::default()
            },
            text_msg("tool", "result"),
        ];
        let points = find_valid_cut_points(&msgs);
        // Index 1 (assistant with tool_calls) is excluded
        assert_eq!(points, vec![0, 2]);
    }

    // ─── find_cut_point ────────────────────────────────────────────────────

    #[test]
    fn find_cut_point_empty_messages_returns_zero() {
        let msgs: Vec<Message> = vec![];
        assert_eq!(find_cut_point(&msgs, 5000), 0);
    }

    #[test]
    fn find_cut_point_returns_first_useful_cut() {
        let msgs = vec![
            text_msg("user", &"a".repeat(400)),     // ~100 tokens
            text_msg("assistant", &"b".repeat(40)), // ~10 tokens
            text_msg("user", &"c".repeat(100)),     // ~25 tokens
        ];
        // keep_recent 20 → keep last ~20 tokens → should cut before last msg
        let cut = find_cut_point(&msgs, 20);
        assert_eq!(cut, 2);
    }

    // ─── extract_file_operations ───────────────────────────────────────────

    #[test]
    fn extract_file_operations_finds_reads_and_writes() {
        let msgs = vec![
            text_msg("user", "read file"),
            Message {
                role: "assistant".to_string(),
                content: None,
                tool_calls: Some(vec![
                    crate::types::ToolCall {
                        id: "tc1".to_string(),
                        call_type: "function".to_string(),
                        function: crate::types::ToolCallFn {
                            name: "read".to_string(),
                            arguments: serde_json::json!(r#"{"path":"/tmp/a.txt"}"#),
                        },
                    },
                    crate::types::ToolCall {
                        id: "tc2".to_string(),
                        call_type: "function".to_string(),
                        function: crate::types::ToolCallFn {
                            name: "write".to_string(),
                            arguments: serde_json::json!(r#"{"file_path":"/tmp/b.txt"}"#),
                        },
                    },
                ]),
                ..Default::default()
            },
        ];
        let (reads, writes) = extract_file_operations(&msgs);
        assert_eq!(reads, vec!["/tmp/a.txt"]);
        assert_eq!(writes, vec!["/tmp/b.txt"]);
    }

    #[test]
    fn extract_file_operations_edit_and_patch_are_writes() {
        let msgs = vec![Message {
            role: "assistant".to_string(),
            content: None,
            tool_calls: Some(vec![crate::types::ToolCall {
                id: "tc1".to_string(),
                call_type: "function".to_string(),
                function: crate::types::ToolCallFn {
                    name: "edit".to_string(),
                    arguments: serde_json::json!(r#"{"path":"/tmp/edit.txt"}"#),
                },
            }]),
            ..Default::default()
        }];
        let (reads, writes) = extract_file_operations(&msgs);
        assert!(reads.is_empty());
        assert_eq!(writes, vec!["/tmp/edit.txt"]);
    }

    // ─── compact / compact_from ────────────────────────────────────────────

    #[test]
    fn compact_below_threshold_does_nothing() {
        let msgs = vec![text_msg("user", "hello")];
        let opts = CompactOptions {
            reserve_tokens: 8000,
            keep_recent_tokens: 4000,
            context_window: 128_000,
            tokens_before: 50,
        };
        let (result, compacted) = compact(msgs.clone(), &opts);
        assert!(compacted.is_none());
        assert_eq!(result.len(), msgs.len());
    }

    #[test]
    fn compact_triggers_and_keeps_recent() {
        // Build enough messages to trigger compaction
        let mut msgs = vec![];
        for i in 0..200 {
            msgs.push(text_msg("user", &format!("message {i} ")));
        }
        let opts = CompactOptions {
            reserve_tokens: 100,
            keep_recent_tokens: 100,
            context_window: 500,
            tokens_before: 0,
        };
        let (result, compacted) = compact(msgs, &opts);
        assert!(compacted.is_some());
        // First message should be the compaction marker
        assert_eq!(result[0].role, "user");
        let content = result[0].content.as_ref().unwrap();
        assert!(content.to_string().contains("compaction"));
        // Result should be shorter than original
        assert!(result.len() < 200);
    }

    #[test]
    fn compact_uses_tokens_before_when_larger_than_estimate() {
        // Need enough messages so that should_compact triggers
        let mut msgs = vec![];
        for i in 0..200 {
            msgs.push(text_msg(
                "user",
                &format!(
                    "message number {i} with extra text to push token count up higher and higher"
                ),
            ));
        }
        let opts = CompactOptions {
            reserve_tokens: 50,
            keep_recent_tokens: 50,
            context_window: 300,
            tokens_before: 500,
        };
        let (_, compact_opt) = compact(msgs, &opts);
        assert!(compact_opt.is_some(), "should trigger compaction");
        // tokens_before is max(opts.tokens_before, estimated), so it's at least 500
        assert!(compact_opt.unwrap().tokens_before >= 500);
    }
}
