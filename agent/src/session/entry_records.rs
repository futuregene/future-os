//! Relational transcript encoding. JSON is reserved for extensible metadata;
//! message bodies live in ordered blocks, never in the entry envelope.
use anyhow::{anyhow, Result};
use rusqlite::{params, Connection};
use serde_json::Value;

use crate::session::entry::{ENTRY_TYPE_ASSISTANT, ENTRY_TYPE_TOOL, ENTRY_TYPE_USER};

/// Does an entry of this kind count as conversation activity — the fact
/// `sessions.updated_at_ms` records? See [`insert_entry_record`].
fn advances_activity(kind: Option<&str>) -> bool {
    matches!(
        kind,
        Some(ENTRY_TYPE_USER | ENTRY_TYPE_ASSISTANT | ENTRY_TYPE_TOOL)
    )
}

/// The entry kinds [`advances_activity`] accepts, as a SQL list.
fn activity_kinds_sql() -> String {
    format!("'{ENTRY_TYPE_USER}','{ENTRY_TYPE_ASSISTANT}','{ENTRY_TYPE_TOOL}'")
}

/// Recompute every stored activity stamp as the session's newest message.
///
/// Recovery for older databases: until 2026-10-27 a metadata snapshot moved
/// `updated_at_ms` forward, so one bulk rewrite restamped hundreds of
/// conversations at once and their age became "when their metadata was
/// written" (a week-idle conversation read "5h"). The entries were never
/// touched, so the true stamp is still in the transcript.
///
/// Idempotent, and a no-op for a database written by the current rule: the
/// stamp can no longer exceed the newest message.
pub(crate) fn repair_activity_stamps(db: &Connection) -> Result<()> {
    db.execute_batch(&format!(
        "WITH activity(session_id,ts) AS (
             SELECT session_id,max(timestamp_ms) FROM entries
              WHERE entry_type IN ({}) GROUP BY session_id
         )
         UPDATE sessions
            SET updated_at_ms=(SELECT ts FROM activity a WHERE a.session_id=sessions.id)
          WHERE updated_at_ms > (SELECT ts FROM activity a WHERE a.session_id=sessions.id);",
        activity_kinds_sql()
    ))?;
    Ok(())
}

pub(crate) fn normalize_entry(mut value: Value) -> Value {
    if !matches!(
        value["type"].as_str(),
        Some("user" | "assistant" | "tool" | "system")
    ) {
        return value;
    }
    let Some(object) = value.as_object_mut() else {
        return value;
    };
    let mut blocks = match object.get("content") {
        Some(Value::Array(values)) => values.clone(),
        Some(Value::String(text)) => vec![serde_json::json!({"type":"text","text":text})],
        _ => Vec::new(),
    };
    if !blocks.iter().any(|b| b["type"] == "reasoning") {
        if let Some(thinking) = object
            .get("thinking")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            blocks.insert(0, serde_json::json!({"type":"reasoning","text":thinking}));
        }
    }
    if !blocks.iter().any(|b| b["type"] == "tool_call") {
        if let Some(calls) = object.get("tool_calls").and_then(Value::as_array) {
            blocks.extend(calls.iter().map(|call|serde_json::json!({"type":"tool_call","id":call["id"],"name":call["function"]["name"],"args":call["function"]["arguments"]})));
        }
    }
    if object["type"] == "tool" && !blocks.iter().any(|b| b["type"] == "tool_result") {
        let text = blocks
            .iter()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("");
        let error = object
            .get("tool_result_is_error")
            .and_then(Value::as_bool)
            .unwrap_or_else(|| text.starts_with("Error:"));
        blocks.retain(|b| b["type"] != "text");
        let mut block = serde_json::json!({"type":"tool_result","tool_call_id":object.get("tool_call_id").and_then(Value::as_str).unwrap_or(""),"content":text});
        if error {
            block["is_error"] = Value::Bool(true);
        }
        blocks.push(block);
    }
    if !blocks.is_empty() {
        object.insert("content".into(), Value::Array(blocks));
    }
    for key in [
        "thinking",
        "tool_calls",
        "tool_call_id",
        "name",
        "tool_args",
        "tool_result_is_error",
    ] {
        object.remove(key);
    }
    value
}

pub(crate) fn insert_entry_record(
    db: &Connection,
    session: &str,
    position: i64,
    value: &Value,
) -> Result<()> {
    let mut metadata = value
        .as_object()
        .cloned()
        .ok_or_else(|| anyhow!("entry must be an object"))?;
    let id = metadata
        .remove("id")
        .ok_or_else(|| anyhow!("missing entry identity"))?;
    let kind = metadata
        .remove("type")
        .ok_or_else(|| anyhow!("missing entry type"))?;
    let role = metadata.remove("role");
    if let Some(role) = role.as_ref().filter(|role| !role.is_string()) {
        metadata.insert("role".into(), role.clone());
    }
    let content = metadata.remove("content");
    let timestamp = metadata.remove("timestamp");
    let timestamp_ms = timestamp
        .as_ref()
        .map(|value| {
            chrono::DateTime::parse_from_rfc3339(
                value
                    .as_str()
                    .ok_or_else(|| anyhow!("invalid entry timestamp"))?,
            )
            .map(|time| time.timestamp_millis())
            .map_err(|_| anyhow!("invalid entry timestamp"))
        })
        .transpose()?;
    let run_id = metadata
        .get("meta")
        .and_then(|meta| meta.get("run_id"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            if matches!(kind.as_str(), Some("run_started" | "run_terminal")) {
                content
                    .as_ref()
                    .and_then(|content| content.get("run_id"))
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned)
            } else {
                None
            }
        });
    if let Some(meta) = metadata.get_mut("meta").and_then(Value::as_object_mut) {
        meta.remove("run_id");
    }
    // Preserve the source's exact timestamp representation only in the internal
    // reconstruction envelope. Queries and the public contract use timestamp_ms.
    // This also preserves legacy identity-conflict detection across re-imports.
    if let Some(timestamp) = timestamp {
        metadata.insert("timestamp".into(), timestamp);
    }
    let blocks = content.as_ref().and_then(Value::as_array);
    if let Some(timestamp_ms) = timestamp_ms {
        // `updated_at_ms` answers "when did this conversation last move", which
        // every client shows as the session's age (`list_sessions` →
        // `updatedAtMs`, and the rail's `5m` / `3h` next to the title).
        //
        // Only a message moves it. `user`, `assistant` and `tool` entries are
        // the conversation; `session_info` is a metadata snapshot (a rename, a
        // cwd change, a model reconcile) and the run markers / compaction
        // records are bookkeeping written around it. Counting bookkeeping as
        // activity is not a rounded-off detail: one bulk model reconcile on
        // 2026-10-27 wrote 311 sessions' snapshots inside three seconds, and
        // conversations untouched for a week began reporting "5h" — the list
        // said every conversation had been used that afternoon. Those entries
        // therefore only *seed* an empty stamp (so a session whose only row is
        // a snapshot still has a time) and never move one forward.
        //
        // `created_at_ms` stays the earliest entry of any kind: the snapshot
        // written when a session is created is that moment.
        let stamp = if advances_activity(kind.as_str()) {
            "updated_at_ms=CASE WHEN updated_at_ms IS NULL THEN ?2 ELSE max(updated_at_ms,?2) END"
        } else {
            "updated_at_ms=CASE WHEN updated_at_ms IS NULL THEN ?2 ELSE updated_at_ms END"
        };
        db.execute(
            &format!(
                "UPDATE sessions SET created_at_ms=CASE WHEN created_at_ms IS NULL THEN ?2 \
                 ELSE min(created_at_ms,?2) END,{stamp} WHERE id=?1"
            ),
            params![session, timestamp_ms],
        )?;
    }
    if kind == "session_info" {
        db.execute(
            "UPDATE sessions SET current_metadata_json=?2 WHERE id=?1",
            params![session, content.as_ref().map(Value::to_string)],
        )?;
    }
    if kind == "model_change" {
        if let Some(model) = content
            .as_ref()
            .and_then(|c| c.get("model"))
            .and_then(Value::as_str)
        {
            db.execute("UPDATE sessions SET current_metadata_json=json_set(COALESCE(current_metadata_json,'{}'),'$.model',?2) WHERE id=?1",params![session,model])?;
            db.execute("UPDATE history_shapes SET payload=json_set(payload,'$.content.model',?2) WHERE session_id=?1 AND position IN (SELECT position FROM entries WHERE session_id=?1 AND entry_type='session_info')",params![session,model])?;
        }
    }
    db.execute("INSERT INTO entries(session_id,position,entry_id,entry_type,role,run_id,timestamp_ms,metadata_json,content_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![session,position,id.as_str(),kind.as_str(),role.as_ref().and_then(Value::as_str),run_id,timestamp_ms,
            Value::Object(metadata.clone()).to_string(),content.as_ref().filter(|_| blocks.is_none() && kind != "session_info").map(Value::to_string)])?;
    if let Some(blocks) = blocks {
        for (ordinal, block) in blocks.iter().enumerate() {
            let mut extra = block.as_object().cloned().unwrap_or_default();
            let kind = extra.get("type").filter(|value| value.is_string()).cloned();
            if kind.is_some() {
                extra.remove("type");
            }
            let block_kind = kind.as_ref().and_then(Value::as_str);
            let text_key = if block_kind == Some("tool_result") {
                "content"
            } else {
                "text"
            };
            let text = extra
                .get(text_key)
                .and_then(Value::as_str)
                .map(str::to_owned);
            if text.is_some() {
                extra.remove(text_key);
            }
            let call_key = if block_kind == Some("tool_result") {
                "tool_call_id"
            } else {
                "id"
            };
            let call = if matches!(block_kind, Some("tool_result" | "tool_call")) {
                extra.get(call_key).filter(|v| v.is_string()).cloned()
            } else {
                None
            };
            if call.is_some() {
                extra.remove(call_key);
            }
            let name = if block_kind == Some("tool_call") {
                extra.get("name").filter(|v| v.is_string()).cloned()
            } else {
                None
            };
            if name.is_some() {
                extra.remove("name");
            }
            let args = if block_kind == Some("tool_call") {
                extra.remove("args")
            } else {
                None
            };
            let error = if block_kind == Some("tool_result") {
                extra.get("is_error").filter(|v| v.is_boolean()).cloned()
            } else {
                None
            };
            if error.is_some() {
                extra.remove("is_error");
            }
            db.execute("INSERT INTO message_blocks(session_id,entry_position,ordinal,kind,text,tool_call_id,tool_name,arguments_json,is_error,metadata_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![session,position,ordinal as i64,block_kind,text,call.as_ref().and_then(Value::as_str),name.as_ref().and_then(Value::as_str),args.map(|v|v.to_string()),error.as_ref().and_then(Value::as_bool),
                if block.is_object() { Value::Object(extra).to_string() } else { block.to_string() }])?;
        }
        // Distinguish an explicitly empty block array from absent content.
        db.execute(
            "UPDATE entries SET content_json='[]' WHERE session_id=?1 AND position=?2",
            params![session, position],
        )?;
    }
    Ok(())
}

pub(crate) const ENTRY_RECORD_VIEWS_SQL: &str = "
DROP VIEW IF EXISTS entry_records;
DROP VIEW IF EXISTS block_records;
CREATE VIEW block_records AS
WITH b0 AS (SELECT *,CASE WHEN kind IS NULL THEN metadata_json ELSE json_set(metadata_json,'$.' || 'type',kind) END AS j0 FROM message_blocks),
b1 AS (SELECT *,CASE WHEN text IS NULL THEN j0 ELSE json_set(j0,'$.' || CASE WHEN kind='tool_result' THEN 'content' ELSE 'text' END,text) END AS j1 FROM b0),
b2 AS (SELECT *,CASE WHEN tool_call_id IS NULL THEN j1 ELSE json_set(j1,'$.' || CASE WHEN kind='tool_result' THEN 'tool_call_id' ELSE 'id' END,tool_call_id) END AS j2 FROM b1),
b3 AS (SELECT *,CASE WHEN tool_name IS NULL THEN j2 ELSE json_set(j2,'$.' || 'name',tool_name) END AS j3 FROM b2),
b4 AS (SELECT *,CASE WHEN arguments_json IS NULL THEN j3 ELSE json_set(j3,'$.' || 'args',json(arguments_json)) END AS j4 FROM b3),
b5 AS (SELECT *,CASE WHEN is_error IS NULL THEN j4 ELSE json_set(j4,'$.' || 'is_error',json(CASE WHEN is_error THEN 'true' ELSE 'false' END)) END AS j5 FROM b4)
SELECT session_id,entry_position,ordinal,j5 AS payload FROM b5;
CREATE VIEW entry_records AS
WITH headers AS (
 SELECT e.*,json_set(metadata_json,'$.id',entry_id,'$.type',entry_type) AS header FROM entries e
), roles AS (
 SELECT *,CASE WHEN role IS NULL THEN header ELSE json_set(header,'$.role',role) END AS with_role FROM headers
), ownership AS (
 SELECT *,CASE WHEN run_id IS NULL OR entry_type IN ('run_started','run_terminal') THEN with_role ELSE json_set(with_role,'$.meta.run_id',run_id) END AS envelope FROM roles
)
SELECT e.session_id,e.position,e.entry_id,e.entry_type,
CASE WHEN e.entry_type='session_info' THEN json_set(envelope,'$.content',json((SELECT current_metadata_json FROM sessions WHERE id=e.session_id)))
 WHEN e.content_json IS NULL THEN envelope ELSE json_set(envelope,'$.content',json(
 CASE WHEN e.content_json='[]' THEN COALESCE((SELECT json_group_array(json(payload)) FROM
 (SELECT payload FROM block_records b WHERE b.session_id=e.session_id AND b.entry_position=e.position ORDER BY ordinal)),'[]')
 ELSE e.content_json END)) END AS payload
FROM ownership e;
";

#[cfg(test)]
mod tests {
    use serde_json::json;
    #[test]
    fn empty_legacy_run_identity_is_assigned_once() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            super::super::sqlite_store::SqliteStore::open(&dir.path().join("agent.db")).unwrap();
        let user = json!({"id":"u","type":"user","role":"user","timestamp":"2026-01-01T00:00:00Z","content":"question","meta":{"run_id":""}});
        let assistant = json!({"id":"a","type":"assistant","role":"assistant","timestamp":"2026-01-01T00:00:01Z","content":"answer","meta":{"run_id":""}});
        store.replace("s", vec![user, assistant.clone()]).unwrap();
        store.append("s", vec![assistant]).unwrap();
        let entries = store.entries("s").unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["meta"]["run_id"], "history:s:u");
        assert_eq!(entries[1]["meta"]["run_id"], "history:s:u");
    }

    #[test]
    fn session_info_entries_persist_their_metadata_content() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            super::super::sqlite_store::SqliteStore::open(&dir.path().join("agent.db")).unwrap();
        store
            .replace(
                "s",
                vec![json!({"id":"u","type":"user","role":"user","timestamp":"2026-01-01T00:00:00Z","content":"question"})],
            )
            .unwrap();
        // A session_info row is not a message: its content is the session's
        // metadata snapshot, so it must round-trip as content rather than as a
        // block list.
        let info = json!({"id":"i","type":"session_info","timestamp":"2026-01-01T00:00:01Z","content":{"cwd":"/tmp","model":"provider-a/m"}});
        store.append("s", vec![info]).unwrap();
        let entries = store.entries("s").unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1]["type"], "session_info");
        assert_eq!(entries[1]["content"]["cwd"], "/tmp");
        assert_eq!(entries[1]["content"]["model"], "provider-a/m");
    }

    #[test]
    fn relational_blocks_preserve_order_and_provider_extensions() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            super::super::sqlite_store::SqliteStore::open(&dir.path().join("agent.db")).unwrap();
        let value = json!({"meta":{"run_id":"synthetic-run"},"id":"m","type":"assistant","role":"assistant","timestamp":"2026-01-01T00:00:00Z","content":[
            {"type":"reasoning","text":"synthetic reasoning","provider_metadata":{"vendor":{"signature":"opaque"}}},
            {"type":"text","text":"before"},
            {"type":"tool_call","id":"call","name":"read","args":null,"provider_metadata":{"nullable":null}},
            {"type":"text","text":"after"},
            {"type":"future_block","unknown":{"keep":true}}
        ]});
        store.replace("s", vec![value.clone()]).unwrap();
        assert_eq!(store.entries("s").unwrap(), vec![value]);
        store
            .db
            .call(|db| {
                let (body, count): (Option<String>, i64) = db.query_row(
                    "SELECT content_json,(SELECT count(*) FROM message_blocks) FROM entries",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                assert_eq!(body.as_deref(), Some("[]"));
                assert_eq!(count, 5);
                Ok(())
            })
            .unwrap();
    }

    /// The stamp the session list shows as "how long ago was this used".
    fn activity_stamp(store: &super::super::sqlite_store::SqliteStore, id: &str) -> i64 {
        let id = id.to_owned();
        store
            .db
            .call(move |db| {
                Ok(db.query_row(
                    "SELECT updated_at_ms FROM sessions WHERE id=?1",
                    [id],
                    |row| row.get::<_, i64>(0),
                )?)
            })
            .unwrap()
    }

    fn at(rfc3339: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(rfc3339)
            .unwrap()
            .timestamp_millis()
    }

    /// A rename, a cwd change or a model reconcile writes a `session_info`
    /// snapshot. That is bookkeeping, not use — letting it move the stamp made
    /// every session a bulk reconcile touched read "5h" while its last message
    /// was a week old.
    #[test]
    fn a_metadata_snapshot_seeds_the_activity_stamp_without_moving_it() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            super::super::sqlite_store::SqliteStore::open(&dir.path().join("agent.db")).unwrap();
        store
            .replace(
                "s",
                vec![json!({"id":"u","type":"user","role":"user","timestamp":"2026-01-01T00:00:00Z","content":"question"})],
            )
            .unwrap();
        assert_eq!(activity_stamp(&store, "s"), at("2026-01-01T00:00:00Z"));
        store
            .append(
                "s",
                vec![json!({"id":"i","type":"session_info","timestamp":"2026-01-05T00:00:00Z","content":{"session_name":"renamed"}})],
            )
            .unwrap();
        assert_eq!(
            activity_stamp(&store, "s"),
            at("2026-01-01T00:00:00Z"),
            "a snapshot must leave the activity stamp where the last message put it"
        );
        // A real message still moves it, forwards only.
        store
            .append(
                "s",
                vec![json!({"id":"a","type":"assistant","role":"assistant","timestamp":"2026-01-06T00:00:00Z","content":"answer"})],
            )
            .unwrap();
        assert_eq!(activity_stamp(&store, "s"), at("2026-01-06T00:00:00Z"));
    }

    /// A session whose only entry is its creation snapshot still needs a
    /// stamp: the list would otherwise have nothing to show.
    #[test]
    fn the_first_snapshot_seeds_an_empty_activity_stamp() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            super::super::sqlite_store::SqliteStore::open(&dir.path().join("agent.db")).unwrap();
        store
            .replace(
                "s",
                vec![json!({"id":"i","type":"session_info","timestamp":"2026-01-02T00:00:00Z","content":{"cwd":"/tmp"}})],
            )
            .unwrap();
        assert_eq!(activity_stamp(&store, "s"), at("2026-01-02T00:00:00Z"));
        assert_eq!(activity_stamp(&store, "s"), at("2026-01-02T00:00:00Z"));
    }

    /// Databases written before the rule above carry snapshots as their
    /// activity stamp. The transcript still holds the answer, so the repair
    /// recovers it — and leaves a session with no messages alone.
    #[test]
    fn the_repair_recovers_a_stamp_pushed_forward_by_a_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            super::super::sqlite_store::SqliteStore::open(&dir.path().join("agent.db")).unwrap();
        store
            .replace(
                "restamped",
                vec![
                    json!({"id":"u","type":"user","role":"user","timestamp":"2026-01-01T00:00:00Z","content":"question"}),
                    json!({"id":"a","type":"assistant","role":"assistant","timestamp":"2026-01-01T00:05:00Z","content":"answer"}),
                    json!({"id":"i","type":"session_info","timestamp":"2026-01-09T00:00:00Z","content":{"session_name":"renamed"}}),
                ],
            )
            .unwrap();
        store
            .replace(
                "snapshot-only",
                vec![json!({"id":"i","type":"session_info","timestamp":"2026-01-03T00:00:00Z","content":{"cwd":"/tmp"}})],
            )
            .unwrap();
        // The old writer let the snapshot set the stamp.
        store
            .db
            .call(|db: &mut rusqlite::Connection| {
                db.execute(
                    "UPDATE sessions SET updated_at_ms=?1 WHERE id='restamped'",
                    [at("2026-01-09T00:00:00Z")],
                )?;
                Ok(())
            })
            .unwrap();
        store
            .db
            .call(|db| super::repair_activity_stamps(db))
            .unwrap();
        assert_eq!(
            activity_stamp(&store, "restamped"),
            at("2026-01-01T00:05:00Z")
        );
        assert_eq!(
            activity_stamp(&store, "snapshot-only"),
            at("2026-01-03T00:00:00Z"),
            "a session with no message keeps the stamp only its snapshot can give it"
        );
        // Idempotent: a second pass changes nothing.
        store
            .db
            .call(|db| super::repair_activity_stamps(db))
            .unwrap();
        assert_eq!(
            activity_stamp(&store, "restamped"),
            at("2026-01-01T00:05:00Z")
        );
    }

    #[test]
    fn run_markers_normalize_identity_without_changing_their_envelope() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            super::super::sqlite_store::SqliteStore::open(&dir.path().join("agent.db")).unwrap();
        let started = json!({
            "id":"started",
            "type":"run_started",
            "role":"system",
            "timestamp":"2026-01-01T00:00:00Z",
            "content":{"run_id":"run-1","epoch":1,"run_sequence":1}
        });
        let terminal = json!({
            "id":"terminal",
            "type":"run_terminal",
            "role":"system",
            "timestamp":"2026-01-01T00:00:01Z",
            "content":{"run_id":"run-1","state":"completed","run_tokens":7,"run_duration_ms":1000}
        });
        store
            .replace("s", vec![started.clone(), terminal.clone()])
            .unwrap();

        assert_eq!(store.entries("s").unwrap(), vec![started, terminal]);
        store
            .db
            .call(|db| {
                let run_ids = db
                    .prepare("SELECT run_id FROM entries ORDER BY position")?
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                assert_eq!(run_ids, vec!["run-1", "run-1"]);
                Ok(())
            })
            .unwrap();
    }
}

/// The legacy shapes the relational encoder must absorb: a value that is not an
/// entry at all, the older "Error:"-prefixed tool text, and an explicitly empty
/// block array (which differs from absent content).
#[cfg(test)]
mod canonical_paths {
    use super::*;
    use crate::session::sqlite_store::SqliteStore;
    use serde_json::json;

    #[test]
    fn a_non_entry_value_is_passed_through_unchanged() {
        assert_eq!(normalize_entry(json!(7)), json!(7));
    }

    #[test]
    fn error_prefixed_tool_text_becomes_an_error_result_block() {
        let canonical = normalize_entry(json!({
            "id":"t","type":"tool","role":"tool","timestamp":"2026-01-01T00:00:00Z",
            "content":"Error: synthetic failure"
        }));
        let blocks = canonical["content"].as_array().unwrap();
        assert_eq!(blocks[0]["type"], "tool_result");
        assert_eq!(blocks[0]["is_error"], true);
    }

    #[test]
    fn an_explicitly_empty_block_array_survives_a_storage_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
        store
            .replace(
                "s",
                vec![json!({"id":"e","type":"assistant","role":"assistant",
                    "timestamp":"2026-01-01T00:00:00Z","content":[]})],
            )
            .unwrap();
        assert_eq!(store.entries("s").unwrap()[0]["content"], json!([]));
    }
}
