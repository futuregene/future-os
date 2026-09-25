//! Shared display projection for full exports and indexed history pages.
use super::SessionEntry;

pub(crate) fn project_entries(entries: &[SessionEntry]) -> Vec<serde_json::Value> {
    // SQLite retains one current metadata record. Accept in-memory snapshots
    // as well, projecting only their final metadata state.
    let authoritative_info = entries
        .iter()
        .rev()
        .find(|e| e.entry_type == crate::session::ENTRY_TYPE_SESSION_INFO)
        .and_then(|e| e.content.clone());
    let mut emitted_session_info = false;
    use future_rpc::message::{MessageRun, MessageUsage};
    let mut final_assistants = std::collections::HashMap::new();
    for entry in entries
        .iter()
        .filter(|entry| entry.entry_type == "assistant")
    {
        if let Some(run) = entry
            .meta
            .as_ref()
            .and_then(|meta| meta.get("run_id"))
            .and_then(|id| id.as_str())
        {
            final_assistants.insert(run.to_owned(), entry.id.clone());
        }
    }
    let mut run_stats: std::collections::HashMap<String, MessageUsage> =
        std::collections::HashMap::new();
    let mut run_outcomes: std::collections::HashMap<String, MessageRun> =
        std::collections::HashMap::new();
    for marker in entries
        .iter()
        .filter(|entry| entry.entry_type == "run_terminal")
    {
        let Some(content) = marker.content.as_ref() else {
            continue;
        };
        let Some(run_id) = content["run_id"].as_str() else {
            continue;
        };
        let status = content["state"].as_str().map(|status| {
            match status {
                "error" => "failed",
                "incomplete" | "interrupted_by_restart" => "interrupted",
                other => other,
            }
            .to_owned()
        });
        run_outcomes.insert(
            run_id.to_owned(),
            MessageRun {
                status,
                error: content["error"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
                duration_ms: content["run_duration_ms"].as_i64(),
            },
        );
        if let Some(assistant) = final_assistants.get(run_id) {
            run_stats.insert(
                assistant.clone(),
                MessageUsage {
                    input_tokens: content["input_tokens"].as_i64(),
                    output_tokens: content["run_tokens"].as_i64(),
                    cache_read_tokens: content["cache_read_tokens"].as_i64(),
                    cache_write_tokens: content["cache_write_tokens"].as_i64(),
                },
            );
        }
    }
    entries
        .iter()
        .filter(|e| {
            if !matches!(
                e.entry_type.as_str(),
                "user" | "assistant" | "tool" | "session_info" | "compaction"
            ) {
                return false;
            }
            // Keep only the first session_info slot; its content is
            // replaced with the authoritative (last) snapshot below, and
            // the stale later snapshots are dropped.
            if e.entry_type == crate::session::ENTRY_TYPE_SESSION_INFO {
                if emitted_session_info {
                    return false;
                }
                emitted_session_info = true;
            }
            true
        })
        .map(|e| {
            use crate::rpc::payloads::SessionEntryPayload;
            use future_rpc::message::MessageBlock;
            let mut metadata = e.meta.clone();
            let run_id = metadata
                .as_mut()
                .and_then(serde_json::Value::as_object_mut)
                .and_then(|meta| meta.remove("run_id"))
                .and_then(|id| id.as_str().filter(|id| !id.is_empty()).map(str::to_owned));
            let outcome = run_id.as_ref().and_then(|id| run_outcomes.get(id));
            let stats = run_stats.get(&e.id);
            let blocks = if matches!(e.entry_type.as_str(), "user" | "assistant" | "tool") {
                let raw: serde_json::Value = serde_json::from_str(
                    &super::Manager::serialize_entry(e).expect("serializable entry"),
                )
                .expect("serialized entry");
                raw["content"]
                    .as_array()
                    .map(|blocks| blocks.iter().map(MessageBlock::from_model).collect())
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            let payload = SessionEntryPayload {
                id: e.id.clone(),
                kind: e.entry_type.clone(),
                role: e.role.clone(),
                created_at_ms: e.timestamp.timestamp_millis(),
                run_id,
                blocks,
                metadata: metadata.filter(|value| !metadata_is_redundant(value)),
                usage: stats.cloned(),
                run: outcome.cloned(),
                session: (e.entry_type == "session_info")
                    .then(|| {
                        authoritative_info
                            .as_ref()
                            .map(future_rpc::message::session_metadata)
                    })
                    .flatten(),
                checkpoint: (e.entry_type == "compaction")
                    .then(|| {
                        e.content
                            .as_ref()
                            .map(future_rpc::message::checkpoint_metadata)
                    })
                    .flatten(),
            };
            serde_json::to_value(payload).expect("serializable history payload")
        })
        .collect()
}

/// Whether an entry's projected `metadata` carries nothing a client can use, so
/// the payload can omit it entirely.
///
/// The value here is `SessionEntry.meta` — the entry's own metadata object, with
/// `run_id` already hoisted into the payload's own field — not the raw
/// `entries.metadata_json` column, whose `{"meta":…,"timestamp":…}` wrapper is
/// loader plumbing that never reaches the wire. Measured on a 6,300-entry real
/// page: 6,292 entries carry exactly `{}` and one carries `{"attachments":…}`.
///
/// So the only redundant shape is the empty object, and ~57 bytes per entry of
/// DB-side wrapper is already projected away upstream. Dropping the empty object
/// saves its 14 bytes on the wire; anything non-empty is kept verbatim, because
/// `attachments` lives here and *is* rendered (the phone shows the attached
/// images and files).
fn metadata_is_redundant(metadata: &serde_json::Value) -> bool {
    matches!(metadata.as_object(), Some(object) if object.is_empty())
}

#[cfg(test)]
mod metadata_tests {
    use super::metadata_is_redundant;
    use serde_json::json;

    /// The measured shape of almost every entry: `e.meta` minus the hoisted
    /// `run_id`, which leaves an object with nothing in it.
    #[test]
    fn the_empty_object_is_redundant() {
        assert!(metadata_is_redundant(&json!({})));
    }

    /// The one that must never regress: attached images/files are rendered on
    /// the phone, and losing them would be silent (the bubble simply shows no
    /// image).
    #[test]
    fn attachments_always_survive() {
        assert!(!metadata_is_redundant(&json!({
            "attachments": [{"path": "/tmp/a.png", "kind": "image"}]
        })));
        // Even an empty attachment list is a deliberate value, not absence.
        assert!(!metadata_is_redundant(&json!({"attachments": []})));
    }

    /// Fail closed: an unrecognized key keeps the whole object rather than being
    /// dropped by a rule written before that key existed, and a non-object
    /// metadata value is passed through untouched.
    #[test]
    fn unknown_keys_and_shapes_are_kept() {
        assert!(!metadata_is_redundant(&json!({"future_key": 1})));
        assert!(!metadata_is_redundant(&json!({"nested": {}})));
        assert!(!metadata_is_redundant(&json!("text")));
        assert!(!metadata_is_redundant(&json!(null)));
        assert!(!metadata_is_redundant(&json!([1, 2])));
        assert!(!metadata_is_redundant(&json!(0)));
    }
}
