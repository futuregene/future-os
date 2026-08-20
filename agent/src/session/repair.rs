//! In-memory healing of a loaded session so it stays API-valid on resume.
//!
//! These are pure functions over `Vec<SessionEntry>` — no file access — and are
//! applied by `Manager::load_path` only in memory. The healed entries are
//! deliberately NOT written back there: persisting a placeholder for a dangling
//! tool call while the owning agent is still mid-tool would corrupt the file
//! when the real result later lands with the same `tool_call_id`.

use super::entry::{
    SessionEntry, ENTRY_TYPE_ASSISTANT, ENTRY_TYPE_RUN_STARTED, ENTRY_TYPE_RUN_TERMINAL,
    ENTRY_TYPE_TOOL,
};

/// Content prefix of the placeholder tool-result entries written by
/// `repair_dangling_tool_calls`. Used to recognise placeholders so a
/// later-arriving REAL tool result with the same tool_call_id can
/// replace them (see `dedupe_tool_entries`).
pub(crate) const TOOL_LOST_PLACEHOLDER_PREFIX: &str = "[Tool execution lost —";

/// Strip assistant entries that have neither content nor tool_calls —
/// the LLM API rejects these with HTTP 400.  Returns true if any were removed.
pub(crate) fn strip_empty_assistants(entries: &mut Vec<SessionEntry>) -> bool {
    let before = entries.len();
    entries.retain(|e| {
        e.entry_type != ENTRY_TYPE_ASSISTANT || e.content.is_some() || !e.tool_calls.is_empty()
    });
    entries.len() != before
}

pub(crate) fn entry_text_starts_with(entry: &SessionEntry, prefix: &str) -> bool {
    match &entry.content {
        Some(serde_json::Value::String(s)) => s.starts_with(prefix),
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .find_map(|block| {
                block
                    .get("text")
                    .or_else(|| block.get("content"))
                    .and_then(|text| text.as_str())
            })
            .is_some_and(|text| text.starts_with(prefix)),
        _ => false,
    }
}

/// Remove duplicate tool-result entries that share the same tool_call_id.
///
/// Duplicates arise when a load-time repair wrote a "tool execution lost"
/// placeholder for a dangling tool_call while the original agent process
/// was still mid-tool, and the real tool result was appended afterwards.
/// Two tool messages with the same tool_call_id make the LLM API reject
/// the request with HTTP 400 ("Messages with role 'tool' must be a
/// response to a preceding message with 'tool_calls'").
///
/// When one of the duplicates is a placeholder and the other is a real
/// result, the placeholder is dropped; otherwise the first entry wins.
pub(crate) fn dedupe_tool_entries(entries: &mut Vec<SessionEntry>) -> bool {
    use std::collections::{HashMap, HashSet};
    // tool_call_id -> index of the entry currently kept for that id
    let mut kept: HashMap<String, usize> = HashMap::new();
    let mut drop_idx: Vec<usize> = vec![];
    for (i, e) in entries.iter().enumerate() {
        if e.entry_type != ENTRY_TYPE_TOOL || e.tool_call_id.is_empty() {
            continue;
        }
        match kept.get(&e.tool_call_id) {
            None => {
                kept.insert(e.tool_call_id.clone(), i);
            }
            Some(&prev) => {
                let prev_is_placeholder =
                    entry_text_starts_with(&entries[prev], TOOL_LOST_PLACEHOLDER_PREFIX);
                let cur_is_placeholder = entry_text_starts_with(e, TOOL_LOST_PLACEHOLDER_PREFIX);
                if prev_is_placeholder && !cur_is_placeholder {
                    // The real result arrived after the placeholder was
                    // written — drop the placeholder, keep the real one.
                    drop_idx.push(prev);
                    kept.insert(e.tool_call_id.clone(), i);
                } else {
                    drop_idx.push(i);
                }
            }
        }
    }
    if drop_idx.is_empty() {
        return false;
    }
    tracing::warn!(
        "Removing {} duplicate tool-result entries from session (shared tool_call_id)",
        drop_idx.len()
    );
    let drop: HashSet<usize> = drop_idx.into_iter().collect();
    let mut i = 0;
    entries.retain(|_| {
        let keep = !drop.contains(&i);
        i += 1;
        keep
    });
    true
}

/// Find every assistant entry with tool_calls that lacks matching tool
/// responses in the entries that follow it (up to the next non-tool entry
/// or run-marker boundary).  An orphaned assistant can appear anywhere in
/// the journal — not just at the end — when a crash happens mid-run and a
/// later restart appends run markers + new user messages ahead of it.
/// Insert placeholder tool-result entries immediately after each orphaned
/// assistant so the conversation stays API-valid.
pub(crate) fn repair_dangling_tool_calls(entries: &mut Vec<SessionEntry>) -> bool {
    use std::collections::HashSet;
    if entries.is_empty() {
        return false;
    }

    // True for entry types that end a tool-response window: after one of
    // these, any pending tool_call_ids are orphaned and need placeholders.
    fn ends_tool_window(entry_type: &str) -> bool {
        matches!(
            entry_type,
            "user" | "system" | "assistant" | ENTRY_TYPE_RUN_STARTED | ENTRY_TYPE_RUN_TERMINAL
        )
    }

    // Collect (insertion_index, Vec<placeholder_entries>) pairs.
    // Process later so earlier insertions don't invalidate indices.
    let now = chrono::Local::now();
    let mut repairs: Vec<(usize, Vec<SessionEntry>)> = Vec::new();
    let mut i = 0;
    while i < entries.len() {
        let entry = &entries[i];
        if entry.entry_type != ENTRY_TYPE_ASSISTANT || entry.tool_calls.is_empty() {
            i += 1;
            continue;
        }
        let pending: HashSet<String> = entry.tool_calls.iter().map(|tc| tc.id.clone()).collect();

        // Walk forward to find which tool_call_ids already have responses.
        let mut matched: HashSet<String> = HashSet::new();
        let mut j = i + 1;
        while j < entries.len() {
            let next = &entries[j];
            if next.entry_type == ENTRY_TYPE_TOOL && pending.contains(&next.tool_call_id) {
                matched.insert(next.tool_call_id.clone());
            }
            if ends_tool_window(&next.entry_type) {
                break;
            }
            j += 1;
        }

        let missing: Vec<_> = pending.difference(&matched).cloned().collect();
        if !missing.is_empty() {
            let placeholders: Vec<SessionEntry> = entry
                .tool_calls
                .iter()
                .filter(|tc| missing.contains(&tc.id))
                .map(|tc| {
                    let placeholder = format!(
                        "{} {} was not executed before the session \
                         was interrupted]",
                        TOOL_LOST_PLACEHOLDER_PREFIX, tc.function.name,
                    );
                    SessionEntry {
                        id: crate::utils::generate_id(),
                        entry_type: ENTRY_TYPE_TOOL.to_string(),
                        role: "tool".to_string(),
                        content: Some(serde_json::Value::String(placeholder)),
                        tool_calls: vec![],
                        timestamp: now,
                        tool_call_id: tc.id.clone(),
                        name: tc.function.name.clone(),
                        tool_args: String::new(),
                        thinking: String::new(),
                        meta: None,
                    }
                })
                .collect();
            // Insert right after the assistant (i + 1).
            repairs.push((i + 1, placeholders));
            // Skip past what we already scanned.
            i = j;
            continue;
        }
        i += 1;
    }

    if repairs.is_empty() {
        return false;
    }

    // Apply insertions in reverse index order to keep positions valid.
    repairs.sort_unstable_by_key(|b| std::cmp::Reverse(b.0));
    for (idx, placeholders) in repairs {
        for placeholder in placeholders.into_iter().rev() {
            entries.insert(idx, placeholder);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_text_starts_with_non_textual_content_is_false() {
        let entry = SessionEntry::new_user("user", serde_json::json!(42));
        assert!(!entry_text_starts_with(&entry, "4"));
    }

    #[test]
    fn repair_dangling_tool_calls_empty_entries_is_noop() {
        assert!(!repair_dangling_tool_calls(&mut Vec::new()));
    }
}
