//! Structured context-checkpoint persistence.
//!
//! Only the current schema is read. A checkpoint written by a retired algorithm (schema 2,
//! or the released string-protocol marker) is **not** recognised: `latest_context_checkpoint`
//! skips it and the session's own journal is projected in full, so the next compaction
//! re-covers that history with the current algorithm. That is deliberate — carrying a
//! legacy checkpoint forward would mean carrying its lossy summary and its author's idea of
//! what to protect, which is exactly what the current algorithms exist to redo.

use super::{SessionEntry, ENTRY_TYPE_COMPACTION, ENTRY_TYPE_SYSTEM};
use crate::compaction::ContextCheckpoint;
use crate::types::ContentBlock;
use chrono::Local;

/// The only checkpoint schema this build writes and reads.
const SCHEMA_VERSION: u64 = 3;

pub fn checkpoint_to_entry(checkpoint: &ContextCheckpoint) -> SessionEntry {
    let mut content = serde_json::json!({
        "schema_version": SCHEMA_VERSION,
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
    // Always present, even when empty: the schema says "this range protected nothing",
    // which is different from an older schema that had no such field. (Deriving the version
    // from whether the list happened to be empty once wrote schema 2 for a current
    // algorithm, which then read back as a legacy checkpoint.)
    content.as_object_mut().expect("checkpoint object").insert(
        "protected_entry_ids".into(),
        serde_json::json!(checkpoint.protected_entry_ids),
    );
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
}

/// Reject a torn, partially copied, or otherwise dangling checkpoint. The
/// caller scans newest-to-oldest, so returning false naturally falls back to
/// the previous valid checkpoint instead of expanding the prompt from a bad
/// cutoff.
fn checkpoint_is_valid(entries: &[SessionEntry], checkpoint: &ContextCheckpoint) -> bool {
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
    let mut seen = std::collections::HashSet::new();
    covered_index <= cutoff_index
        && cutoff_index < checkpoint_index
        && checkpoint.protected_entry_ids.iter().all(|id| {
            seen.insert(id)
                && entries
                    .iter()
                    .position(|e| {
                        &e.id == id && matches!(e.entry_type.as_str(), "user" | "assistant")
                    })
                    .is_some_and(|index| index <= cutoff_index)
        })
}

/// Read a checkpoint entry. Anything that is not the current schema — an older
/// `schema_version`, a missing `protected_entry_ids`, or a retired algorithm's string
/// marker — is not a checkpoint as far as this build is concerned, so the caller keeps
/// looking (and, finding none, projects the whole journal).
pub fn entry_to_checkpoint(entry: &SessionEntry) -> Option<ContextCheckpoint> {
    (entry.entry_type == ENTRY_TYPE_COMPACTION)
        .then(|| compaction_entry_to_checkpoint(entry))
        .flatten()
}

fn compaction_entry_to_checkpoint(entry: &SessionEntry) -> Option<ContextCheckpoint> {
    let content = entry.content.as_ref()?.as_object()?;
    if content
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        != Some(SCHEMA_VERSION)
    {
        return None;
    }
    let summary: Vec<ContentBlock> =
        serde_json::from_value(content.get("summary")?.clone()).ok()?;
    let trigger = serde_json::from_value(
        content
            .get("trigger")
            .cloned()
            .unwrap_or_else(|| serde_json::json!("automatic")),
    )
    .ok()?;
    Some(ContextCheckpoint {
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
        protected_entry_ids: serde_json::from_value(content.get("protected_entry_ids")?.clone())
            .ok()?,
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
            .and_then(serde_json::Value::as_str)?
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compaction::CompactionTrigger;
    use chrono::TimeZone;

    #[test]
    fn checkpoint_round_trips_through_existing_session_envelope() {
        let checkpoint = ContextCheckpoint {
            entry_id: "entry-cp".into(),
            protected_entry_ids: vec!["entry-a".into(), "entry-b".into()],
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
        };
        let entry = checkpoint_to_entry(&checkpoint);
        let parsed = entry_to_checkpoint(&entry).unwrap();
        assert_eq!(parsed.checkpoint_id, checkpoint.checkpoint_id);
        assert_eq!(parsed.cutoff_entry_id, checkpoint.cutoff_entry_id);
        assert_eq!(parsed.tokens_after, 20);
        assert_eq!(parsed.phase, checkpoint.phase);
        assert_eq!(parsed.protected_entry_ids, checkpoint.protected_entry_ids);
        assert_eq!(entry.content.as_ref().unwrap()["schema_version"], 3);
    }

    #[test]
    fn optional_phase_is_omitted_instead_of_serialized_as_null() {
        let checkpoint = ContextCheckpoint {
            entry_id: "entry-no-phase".into(),
            protected_entry_ids: Vec::new(),
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
        };
        let entry = checkpoint_to_entry(&checkpoint);
        assert!(entry.content.unwrap().get("phase").is_none());
    }

    #[test]
    fn a_retired_schema_is_not_read_as_a_checkpoint() {
        // Schema 2 predates `protected_entry_ids`. Ignoring it is deliberate: the covered
        // range stays in the journal, so the next compaction re-covers it with the current
        // algorithm instead of inheriting the retired one's summary and its idea of what to
        // protect.
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
            "algorithm_version": "semantic-v1",
            "model": "model",
            "context_window": 200
        }));
        assert!(entry_to_checkpoint(&entry).is_none());
        assert!(latest_context_checkpoint(&[first, entry]).is_none());
    }

    #[test]
    fn a_schema_2_entry_with_no_protected_list_is_not_a_checkpoint() {
        // The current schema always carries the list, even when empty; its absence marks
        // an older writer.
        let mut entry = SessionEntry::new_user("user", serde_json::json!(null));
        entry.entry_type = ENTRY_TYPE_COMPACTION.into();
        entry.content = Some(serde_json::json!({
            "schema_version": 3,
            "checkpoint_id": "cp",
            "summary": [{"type": "text", "text": "summary"}],
            "trigger": "automatic",
            "algorithm_version": "deterministic-s2-evidence-v1"
        }));
        assert!(entry_to_checkpoint(&entry).is_none());
    }

    #[test]
    fn a_released_string_protocol_entry_is_not_a_checkpoint() {
        // The released protocol stored its summary as a plain entry with no range, so
        // there is nothing to project against. Treating it as ordinary content is what the
        // journal shows; nothing may silently claim the covered prefix back.
        let mut entry = SessionEntry::new_user("user", serde_json::json!("ignored"));
        entry.entry_type = ENTRY_TYPE_COMPACTION.into();
        entry.content = Some(serde_json::json!({
            "summary": "[Context compaction: old summary]",
            "tokens_in": 99
        }));
        assert!(entry_to_checkpoint(&entry).is_none());
        assert!(latest_context_checkpoint(&[entry]).is_none());
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
            protected_entry_ids: Vec::new(),
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

    #[test]
    fn v3_rejects_missing_or_out_of_range_protected_references() {
        let first = SessionEntry::new_user("user", serde_json::json!("first"));
        let later = SessionEntry::new_user("user", serde_json::json!("later"));
        let mut entry = SessionEntry::new_user("system", serde_json::json!(null));
        entry.entry_type = ENTRY_TYPE_COMPACTION.into();
        entry.content = Some(
            serde_json::json!({"schema_version":3,"checkpoint_id":"cp","covered_from_entry_id":first.id,"cutoff_entry_id":first.id,"summary":[{"type":"text","text":"summary"}],"protected_entry_ids":[later.id],"algorithm_version":"deterministic-s2-evidence-v1"}),
        );
        assert!(
            latest_context_checkpoint(&[first.clone(), later.clone(), entry.clone()]).is_none()
        );
        entry.content.as_mut().unwrap()["protected_entry_ids"] = serde_json::json!([first.id]);
        assert!(latest_context_checkpoint(&[first, later, entry.clone()]).is_some());
        entry
            .content
            .as_mut()
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("protected_entry_ids");
        assert!(entry_to_checkpoint(&entry).is_none());
    }

    #[test]
    fn latest_checkpoint_rejects_a_checkpoint_without_range_refs() {
        let checkpoint = ContextCheckpoint {
            entry_id: "entry-no-range".into(),
            protected_entry_ids: Vec::new(),
            checkpoint_id: "cp-no-range".into(),
            covered_from_entry_id: None,
            cutoff_entry_id: None,
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
        assert!(latest_context_checkpoint(&[checkpoint_to_entry(&checkpoint)]).is_none());
    }

    #[test]
    fn latest_checkpoint_rejects_a_checkpoint_with_dangling_covered_from() {
        let first = SessionEntry::new_user("user", serde_json::json!("first"));
        let checkpoint = ContextCheckpoint {
            entry_id: "entry-dangling-covered".into(),
            protected_entry_ids: Vec::new(),
            checkpoint_id: "cp-dangling-covered".into(),
            covered_from_entry_id: Some("missing-covered".into()),
            cutoff_entry_id: Some(first.id.clone()),
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
        let entries = vec![first, checkpoint_to_entry(&checkpoint)];
        assert!(latest_context_checkpoint(&entries).is_none());
    }

    #[test]
    fn only_a_compaction_entry_can_be_a_checkpoint() {
        let assistant = SessionEntry::new_assistant(serde_json::json!("hi"), vec![]);
        assert!(entry_to_checkpoint(&assistant).is_none());
        // A user message that merely looks like the released marker is content, not a
        // checkpoint: the writer that produced those markers is gone, and inferring a
        // checkpoint from a string would let ordinary text claim a covered prefix.
        let user = SessionEntry::new_user(
            "user",
            serde_json::json!("[Context compaction: legacy text]"),
        );
        assert!(entry_to_checkpoint(&user).is_none());
        assert!(latest_context_checkpoint(&[user]).is_none());
    }

    #[test]
    fn non_string_non_array_user_content_is_not_a_legacy_checkpoint() {
        let user = SessionEntry::new_user("user", serde_json::json!({"type": "text"}));
        assert!(entry_to_checkpoint(&user).is_none());
    }
}
