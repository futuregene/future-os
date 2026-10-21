//! Body-free history index. Ordered message blocks remain the body store.
//! Reconciliation runs inside the transcript transaction, never on page reads.
use super::display_projection::project_entries;
use crate::session::{repair, SessionEntry};
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

/// Retain only the fields needed for visibility, pairing and run annotations.
/// In particular, do not duplicate text, reasoning, arguments or attachments.
///
/// `compaction` keeps its whole `content` because that object *is* the
/// checkpoint the projection turns into the divider's identity and counts
/// (`checkpointId`, `tokensBefore`, `tokensAfter`, `trigger`, `phase`). Blanking
/// it — as every other kind's body is blanked — left `payload.checkpoint` as the
/// empty string those rows were projected from, which the client cannot read a
/// field off: a body-less compaction row then served a divider with no numbers
/// and no checkpoint identity. The entry it copies is bounded and rare, and a
/// partial copy would make an indexed page disagree with the direct projection
/// of the same journal.
fn entry_shape(value: &Value) -> Value {
    let kind = value["type"].as_str().unwrap_or("");
    let mut result = json!({
        "id": value["id"], "type": kind,
        "role": value["role"].as_str().unwrap_or(""),
        "timestamp": value.get("timestamp").cloned().unwrap_or(json!("1970-01-01T00:00:00Z")),
    });
    if let Some(run) = value.get("meta").and_then(|v| v.get("run_id")) {
        result["meta"] = json!({"run_id": run});
    }
    let blocks = value.get("content").and_then(Value::as_array);
    let mut calls = Vec::new();
    if let Some(legacy) = value.get("tool_calls").and_then(Value::as_array) {
        for call in legacy {
            calls.push(json!({"id":call["id"], "type":"function", "function":{"name":call["function"]["name"], "arguments":{}}}));
        }
    }
    if calls.is_empty() {
        if let Some(blocks) = blocks {
            for block in blocks.iter().filter(|b| b["type"] == "tool_call") {
                if let Ok(crate::types::ContentBlock::ToolCall { id, name, .. }) =
                    serde_json::from_value::<crate::types::ContentBlock>(block.clone())
                {
                    calls.push(json!({"id":id, "type":"function", "function":{"name":name, "arguments":{}}}));
                }
            }
        }
    }
    if !calls.is_empty() {
        result["tool_calls"] = json!(calls);
    }
    if matches!(
        kind,
        // Kinds whose `content` is what the projection emits: the session
        // snapshot, a marker's terminal state, and a checkpoint's metadata.
        "session_info" | "model_change" | "run_started" | "run_terminal" | "compaction"
    ) {
        if let Some(content) = value.get("content") {
            result["content"] = content.clone();
        }
    } else if value.get("content").is_some_and(|v| !v.is_null()) {
        result["content"] = json!("");
    }
    if kind == "tool" {
        let canonical_call_id = blocks.and_then(|blocks| {
            blocks
                .iter()
                .filter(|b| b["type"] == "tool_result")
                .find_map(|block| {
                    match serde_json::from_value::<crate::types::ContentBlock>(block.clone())
                        .ok()?
                    {
                        crate::types::ContentBlock::ToolResult { tool_call_id, .. } => {
                            Some(tool_call_id)
                        }
                        _ => None,
                    }
                })
        });
        let call_id = value
            .get("tool_call_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .or(canonical_call_id.as_deref())
            .unwrap_or("");
        result["tool_call_id"] = json!(call_id);
        let text = value["content"].as_str().or_else(|| {
            blocks.and_then(|blocks| {
                blocks.iter().find_map(|b| {
                    b.get("text")
                        .or_else(|| b.get("content"))
                        .and_then(Value::as_str)
                })
            })
        });
        if text.is_some_and(|text| text.starts_with(repair::TOOL_LOST_PLACEHOLDER_PREFIX)) {
            result["content"] = json!(repair::TOOL_LOST_PLACEHOLDER_PREFIX);
        }
    }
    result
}

pub(crate) fn insert_entry_shape(
    db: &Connection,
    session: &str,
    position: i64,
    value: &Value,
) -> Result<()> {
    db.execute(
        "INSERT INTO history_shapes(session_id,position,payload) VALUES (?1,?2,?3)",
        params![session, position, entry_shape(value).to_string()],
    )?;
    Ok(())
}

/// Expand the dirty tail to include every run/tool dependency crossing its
/// boundary. Repairs can move late results and replace old placeholders, so a
/// simple "last N rows" update would leave stale historical projections.
fn affected_history_start(db: &Connection, session: &str, mut start: i64) -> Result<i64> {
    loop {
        let run: Option<i64> = db.query_row(
            "SELECT min((SELECT min(position) FROM entries old WHERE old.session_id=?1
             AND old.run_id=tail.run_id AND old.position<?2))
             FROM (SELECT run_id FROM entries WHERE session_id=?1 AND position>=?2
                   ) tail",
            params![session, start],
            |r| r.get(0),
        )?;
        let tool: Option<i64> = db.query_row(
            "SELECT min((SELECT min(entry_position) FROM message_blocks old WHERE old.session_id=?1
             AND old.tool_call_id=tail.tool_call_id AND old.kind IN ('tool_call','tool_result')
             AND old.entry_position<?2))
             FROM (SELECT tool_call_id FROM message_blocks WHERE session_id=?1
                   AND entry_position>=?2 AND kind IN ('tool_call','tool_result')
                   ) tail",
            params![session, start],
            |r| r.get(0),
        )?;
        let expanded = run.into_iter().chain(tool).fold(start, i64::min);
        // Include the user boundary and its complete tool window. This also
        // gives the suffix a stable display ordinal before synthetic repairs.
        let boundary: i64 = db.query_row(
            "SELECT coalesce(max(position),0) FROM entries
             WHERE session_id=?1 AND entry_type='user' AND position<=?2",
            params![session, expanded],
            |r| r.get(0),
        )?;
        if boundary == start {
            return Ok(start);
        }
        start = boundary;
    }
}

pub(crate) fn refresh_session_metadata(db: &Connection, session: &str) -> Result<()> {
    let row: Option<(i64, String)> = db
        .query_row(
            "SELECT h.position,h.payload FROM history_shapes h JOIN entries e
         ON e.session_id=h.session_id AND e.position=h.position
         WHERE e.session_id=?1 AND e.entry_type='session_info' ORDER BY e.position LIMIT 1",
            [session],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((position, raw)) = row {
        let entry: SessionEntry = serde_json::from_str(&raw)?;
        if let Some(payload) = project_entries(&[entry]).pop() {
            db.execute(
                "UPDATE history_display SET payload=?3 WHERE session_id=?1 AND source_position=?2",
                params![session, position, payload.to_string()],
            )?;
        }
    }
    Ok(())
}

pub(crate) fn update_display_index_from(
    db: &Connection,
    session: &str,
    changed_position: i64,
) -> Result<()> {
    let start = affected_history_start(db, session, changed_position)?;
    let ordinal: i64 = if start == 0 {
        0
    } else {
        db.query_row(
            "SELECT coalesce((SELECT min(ordinal) FROM history_display
            WHERE session_id=?1 AND source_position>=?2),
            (SELECT max(ordinal)+1 FROM history_display WHERE session_id=?1),0)",
            params![session, start],
            |r| r.get(0),
        )?
    };
    let start = if ordinal == 0 { 0 } else { start };
    let mut stmt = db.prepare(
        "SELECT position,payload FROM history_shapes WHERE session_id=?1 AND position>=?2 ORDER BY position",
    )?;
    let rows = stmt
        .query_map(params![session, start], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut positions = std::collections::HashMap::new();
    let mut entries = Vec::with_capacity(rows.len());
    for (position, row) in rows {
        let entry: SessionEntry = serde_json::from_str(&row)?;
        positions.insert(entry.id.clone(), position);
        entries.push(entry);
    }
    repair::strip_empty_assistants(&mut entries);
    repair::dedupe_tool_entries(&mut entries);
    repair::repair_dangling_tool_calls(&mut entries);
    let projected = project_entries(&entries);
    let mut upsert = db.prepare("INSERT INTO history_display(session_id,ordinal,source_position,is_user,payload) VALUES (?1,?2,?3,?4,?5)
        ON CONFLICT(session_id,ordinal) DO UPDATE SET source_position=excluded.source_position,is_user=excluded.is_user,payload=excluded.payload
        WHERE history_display.source_position IS NOT excluded.source_position OR history_display.payload<>excluded.payload")?;
    for (index, payload) in projected.iter().enumerate() {
        let source = payload["id"]
            .as_str()
            .and_then(|id| positions.get(id))
            .copied();
        upsert.execute(params![
            session,
            ordinal + index as i64,
            source,
            payload["role"] == "user",
            payload.to_string()
        ])?;
    }
    db.execute(
        "DELETE FROM history_display WHERE session_id=?1 AND ordinal>=?2",
        params![session, ordinal + projected.len() as i64],
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "incremental_index_tests.rs"]
mod incremental_tests;
#[cfg(test)]
#[path = "index_tests.rs"]
mod tests;
