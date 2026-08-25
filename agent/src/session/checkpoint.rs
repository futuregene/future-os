//! Structured context-checkpoint persistence and legacy read compatibility.

use super::{SessionEntry, ENTRY_TYPE_COMPACTION, ENTRY_TYPE_SYSTEM};
use crate::compaction::{CompactionTrigger, ContextCheckpoint};
use crate::types::ContentBlock;
use chrono::Local;

pub fn checkpoint_to_entry(checkpoint: &ContextCheckpoint) -> SessionEntry {
    let mut content = serde_json::json!({
        "schema_version": 2,
        "checkpoint_id": checkpoint.checkpoint_id,
        "covered_from_entry_id": checkpoint.covered_from_entry_id,
        "cutoff_entry_id": checkpoint.cutoff_entry_id,
        "summary": checkpoint.summary,
        "tokens_before": checkpoint.tokens_before,
        "tokens_after": checkpoint.tokens_after,
        "trigger": checkpoint.trigger,
        "algorithm_version": checkpoint.algorithm_version,
        "model": checkpoint.model,
        "context_window": checkpoint.context_window,
    });
    if let Some(phase) = checkpoint.phase {
        content
            .as_object_mut()
            .expect("checkpoint content is an object")
            .insert("phase".to_string(), serde_json::json!(phase));
    }
    SessionEntry {
        id: checkpoint.entry_id.clone(),
        entry_type: ENTRY_TYPE_COMPACTION.to_string(),
        role: ENTRY_TYPE_SYSTEM.to_string(),
        content: Some(content),
        tool_calls: Vec::new(),
        timestamp: checkpoint.created_at.with_timezone(&Local),
        tool_call_id: String::new(),
        name: String::new(),
        tool_args: String::new(),
        thinking: String::new(),
        meta: None,
    }
}

pub fn latest_context_checkpoint(entries: &[SessionEntry]) -> Option<ContextCheckpoint> {
    entries
        .iter()
        .rev()
        .filter(|entry| entry.entry_type == ENTRY_TYPE_COMPACTION)
        .filter_map(compaction_entry_to_checkpoint)
        .find(|checkpoint| checkpoint_is_valid(entries, checkpoint))
        .or_else(|| {
            // The released string protocol only ever placed its marker at the
            // beginning of the surviving message history. Restrict the
            // compatibility heuristic to that first message so a real later
            // user prompt beginning with the same text is never reclassified.
            entries
                .iter()
                .find(|entry| matches!(entry.entry_type.as_str(), "user" | "assistant" | "tool"))
                .and_then(legacy_string_checkpoint)
        })
}

/// Reject a torn, partially copied, or otherwise dangling v2 checkpoint. The
/// caller scans newest-to-oldest, so returning false naturally falls back to
/// the previous valid checkpoint instead of expanding the prompt from a bad
/// cutoff. Legacy checkpoints have no range references and remain readable.
fn checkpoint_is_valid(entries: &[SessionEntry], checkpoint: &ContextCheckpoint) -> bool {
    if checkpoint.legacy_without_cutoff {
        return true;
    }
    let (Some(covered_from), Some(cutoff)) = (
        checkpoint.covered_from_entry_id.as_deref(),
        checkpoint.cutoff_entry_id.as_deref(),
    ) else {
        return false;
    };
    let Some(checkpoint_index) = entries
        .iter()
        .position(|entry| entry.id == checkpoint.entry_id)
    else {
        return false;
    };
    let Some(covered_index) = entries.iter().position(|entry| entry.id == covered_from) else {
        return false;
    };
    let Some(cutoff_index) = entries.iter().position(|entry| entry.id == cutoff) else {
        return false;
    };
    covered_index <= cutoff_index && cutoff_index < checkpoint_index
}

pub fn entry_to_checkpoint(entry: &SessionEntry) -> Option<ContextCheckpoint> {
    if entry.entry_type == ENTRY_TYPE_COMPACTION {
        return compaction_entry_to_checkpoint(entry);
    }
    legacy_string_checkpoint(entry)
}

fn compaction_entry_to_checkpoint(entry: &SessionEntry) -> Option<ContextCheckpoint> {
    let content = entry.content.as_ref()?.as_object()?;
    if content
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        == Some(2)
    {
        let summary: Vec<ContentBlock> =
            serde_json::from_value(content.get("summary")?.clone()).ok()?;
        let trigger = serde_json::from_value(
            content
                .get("trigger")
                .cloned()
                .unwrap_or_else(|| serde_json::json!("automatic")),
        )
        .ok()?;
        return Some(ContextCheckpoint {
            entry_id: entry.id.clone(),
            checkpoint_id: content.get("checkpoint_id")?.as_str()?.to_string(),
            covered_from_entry_id: content
                .get("covered_from_entry_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            cutoff_entry_id: content
                .get("cutoff_entry_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            summary,
            tokens_before: content
                .get("tokens_before")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default(),
            tokens_after: content
                .get("tokens_after")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default(),
            trigger,
            phase: content
                .get("phase")
                .cloned()
                .and_then(|value| serde_json::from_value(value).ok()),
            algorithm_version: content
                .get("algorithm_version")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("v2")
                .to_string(),
            model: content
                .get("model")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            context_window: content
                .get("context_window")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default(),
            created_at: entry.timestamp.with_timezone(&chrono::Utc),
            legacy_without_cutoff: false,
        });
    }

    let raw_summary = content.get("summary").and_then(serde_json::Value::as_str)?;
    Some(legacy_checkpoint(entry, raw_summary))
}

fn legacy_string_checkpoint(entry: &SessionEntry) -> Option<ContextCheckpoint> {
    if entry.entry_type != "user" || entry.role != "user" {
        return None;
    }
    // Modern user entries carry their owning run id. Never reinterpret their
    // literal text as the released legacy compaction protocol; genuine legacy
    // marker entries predate run provenance and therefore have no such stamp.
    if entry
        .meta
        .as_ref()
        .and_then(|meta| meta.get("run_id"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|run_id| !run_id.is_empty())
    {
        return None;
    }
    let text = match entry.content.as_ref()? {
        serde_json::Value::String(text) => text.as_str(),
        serde_json::Value::Array(blocks) => blocks
            .iter()
            .find_map(|block| block.get("text").and_then(serde_json::Value::as_str))?,
        _ => return None,
    };
    text.starts_with("[Context compaction:")
        .then(|| legacy_checkpoint(entry, text))
}

fn legacy_checkpoint(entry: &SessionEntry, raw_summary: &str) -> ContextCheckpoint {
    let summary = raw_summary
        .strip_prefix("[Context compaction:")
        .and_then(|value| value.strip_suffix(']'))
        .map(str::trim)
        .unwrap_or(raw_summary)
        .to_string();
    ContextCheckpoint {
        entry_id: entry.id.clone(),
        checkpoint_id: format!("legacy_{}", entry.id),
        covered_from_entry_id: None,
        cutoff_entry_id: None,
        summary: vec![ContentBlock::text(summary)],
        tokens_before: entry
            .content
            .as_ref()
            .and_then(|content| content.get("tokens_in"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default(),
        tokens_after: 0,
        trigger: CompactionTrigger::Automatic,
        phase: None,
        algorithm_version: "legacy".to_string(),
        model: String::new(),
        context_window: 0,
        created_at: entry.timestamp.with_timezone(&chrono::Utc),
        legacy_without_cutoff: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn v2_checkpoint_round_trips_through_existing_session_envelope() {
        let checkpoint = ContextCheckpoint {
            entry_id: "entry-cp".into(),
            checkpoint_id: "cp-1".into(),
            covered_from_entry_id: Some("entry-a".into()),
            cutoff_entry_id: Some("entry-b".into()),
            summary: vec![ContentBlock::text("summary")],
            tokens_before: 120,
            tokens_after: 20,
            trigger: CompactionTrigger::ProviderContextLimit,
            phase: Some(crate::compaction::CompactionPhase::MidTurn),
            algorithm_version: "v2".into(),
            model: "model".into(),
            context_window: 200,
            created_at: chrono::Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            legacy_without_cutoff: false,
        };
        let entry = checkpoint_to_entry(&checkpoint);
        let parsed = entry_to_checkpoint(&entry).unwrap();
        assert_eq!(parsed.checkpoint_id, checkpoint.checkpoint_id);
        assert_eq!(parsed.cutoff_entry_id, checkpoint.cutoff_entry_id);
        assert_eq!(parsed.tokens_after, 20);
        assert_eq!(parsed.phase, checkpoint.phase);
    }

    #[test]
    fn optional_phase_is_omitted_instead_of_serialized_as_null() {
        let checkpoint = ContextCheckpoint {
            entry_id: "entry-no-phase".into(),
            checkpoint_id: "cp-no-phase".into(),
            covered_from_entry_id: Some("entry-a".into()),
            cutoff_entry_id: Some("entry-b".into()),
            summary: vec![ContentBlock::text("summary")],
            tokens_before: 120,
            tokens_after: 20,
            trigger: CompactionTrigger::Automatic,
            phase: None,
            algorithm_version: "v2".into(),
            model: "model".into(),
            context_window: 200,
            created_at: chrono::Utc::now(),
            legacy_without_cutoff: false,
        };
        let entry = checkpoint_to_entry(&checkpoint);
        assert!(entry.content.unwrap().get("phase").is_none());
    }

    #[test]
    fn released_v2_checkpoint_without_phase_remains_readable() {
        let first = SessionEntry::new_user("user", serde_json::json!("first"));
        let mut entry = SessionEntry::new_user("user", serde_json::json!(null));
        entry.id = "entry-cp-old-v2".into();
        entry.entry_type = ENTRY_TYPE_COMPACTION.into();
        entry.role = ENTRY_TYPE_SYSTEM.into();
        entry.content = Some(serde_json::json!({
            "schema_version": 2,
            "checkpoint_id": "cp-old-v2",
            "covered_from_entry_id": first.id,
            "cutoff_entry_id": first.id,
            "summary": [{"type": "text", "text": "summary"}],
            "tokens_before": 100,
            "tokens_after": 10,
            "trigger": "automatic",
            "algorithm_version": "v2",
            "model": "model",
            "context_window": 200
        }));
        let parsed = entry_to_checkpoint(&entry).unwrap();
        assert_eq!(parsed.checkpoint_id, "cp-old-v2");
        assert_eq!(parsed.phase, None);
    }

    #[test]
    fn legacy_compaction_entry_is_read_only_compatible() {
        let mut entry = SessionEntry::new_user("user", serde_json::json!("ignored"));
        entry.entry_type = ENTRY_TYPE_COMPACTION.into();
        entry.content = Some(serde_json::json!({
            "summary": "[Context compaction: old summary]",
            "tokens_in": 99
        }));
        let parsed = entry_to_checkpoint(&entry).unwrap();
        assert!(parsed.legacy_without_cutoff);
        assert_eq!(parsed.tokens_before, 99);
    }

    #[test]
    fn modern_user_text_that_looks_like_a_legacy_marker_stays_user_content() {
        let mut entry = SessionEntry::new_user(
            "user",
            serde_json::json!("[Context compaction: explain this literal syntax]"),
        );
        entry.meta = Some(serde_json::json!({"run_id": "run-modern"}));

        assert!(entry_to_checkpoint(&entry).is_none());
        assert!(latest_context_checkpoint(&[entry]).is_none());
    }

    #[test]
    fn latest_checkpoint_skips_dangling_newer_checkpoint() {
        let first = SessionEntry::new_user("user", serde_json::json!("first"));
        let cutoff = SessionEntry::new_user("user", serde_json::json!("cutoff"));
        let valid = ContextCheckpoint {
            entry_id: "entry-valid-cp".into(),
            checkpoint_id: "valid-cp".into(),
            covered_from_entry_id: Some(first.id.clone()),
            cutoff_entry_id: Some(cutoff.id.clone()),
            summary: vec![ContentBlock::text("valid")],
            tokens_before: 100,
            tokens_after: 10,
            trigger: CompactionTrigger::Automatic,
            phase: None,
            algorithm_version: "v2".into(),
            model: "model".into(),
            context_window: 200,
            created_at: chrono::Utc::now(),
            legacy_without_cutoff: false,
        };
        let dangling = ContextCheckpoint {
            entry_id: "entry-dangling-cp".into(),
            checkpoint_id: "dangling-cp".into(),
            cutoff_entry_id: Some("missing".into()),
            ..valid.clone()
        };
        let entries = vec![
            first,
            cutoff,
            checkpoint_to_entry(&valid),
            checkpoint_to_entry(&dangling),
        ];

        assert_eq!(
            latest_context_checkpoint(&entries)
                .expect("previous valid checkpoint")
                .checkpoint_id,
            "valid-cp"
        );
    }
}
