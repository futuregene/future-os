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
    let stored_session = crate::session::identity::session_key(db, session)?;
    db.execute(
        "INSERT INTO history_shapes(session_id,position,payload) VALUES (?1,?2,?3)",
        params![&stored_session, position, entry_shape(value).to_string()],
    )?;
    Ok(())
}

/// Expand the dirty tail to include every run/tool dependency crossing its
/// boundary. Repairs can move late results and replace old placeholders, so a
/// simple "last N rows" update would leave stale historical projections.
fn affected_history_start(db: &Connection, session: &str, mut start: i64) -> Result<i64> {
    let stored_session = crate::session::identity::session_key(db, session)?;
    loop {
        // Set-based on purpose: the correlated form re-ran the tail scan once per
        // row, which is what made one lazy rebuild batch of a 14,000-entry
        // conversation cost seconds. Both halves read an index.
        let run: Option<i64> = db.query_row(
            "SELECT min(position) FROM entries
              WHERE session_id=?1 AND run_id IN (
                SELECT run_id FROM entries WHERE session_id=?1 AND position>=?2 AND run_id IS NOT NULL)",
            params![&stored_session, start],
            |r| r.get(0),
        )?;
        let tool: Option<i64> = db.query_row(
            "SELECT min(entry_position) FROM message_blocks
              WHERE session_id=?1 AND kind IN ('tool_call','tool_result')
                AND tool_call_id IN (
                  SELECT tool_call_id FROM message_blocks WHERE session_id=?1
                   AND entry_position>=?2 AND kind IN ('tool_call','tool_result')
                   AND tool_call_id IS NOT NULL)",
            params![&stored_session, start],
            |r| r.get(0),
        )?;
        let expanded = run.into_iter().chain(tool).fold(start, i64::min);
        // Include the user boundary and its complete tool window. This also
        // gives the suffix a stable display ordinal before synthetic repairs.
        let boundary: i64 = db.query_row(
            "SELECT coalesce(max(position),0) FROM entries
             WHERE session_id=?1 AND entry_type='user' AND position<=?2",
            params![&stored_session, expanded],
            |r| r.get(0),
        )?;
        if boundary == start {
            return Ok(start);
        }
        start = boundary;
    }
}

pub(crate) fn refresh_session_metadata(db: &Connection, session: &str) -> Result<()> {
    let stored_session = crate::session::identity::session_key(db, session)?;
    let row: Option<(i64, String)> = db
        .query_row(
            "SELECT h.position,h.payload FROM history_shapes h JOIN entries e
         ON e.session_id=h.session_id AND e.position=h.position
         WHERE e.session_id=?1 AND e.entry_type='session_info' ORDER BY e.position LIMIT 1",
            [&stored_session],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((position, raw)) = row {
        let entry: SessionEntry = serde_json::from_str(&raw)?;
        if let Some(payload) = project_entries(&[entry]).pop() {
            db.execute(
                "UPDATE history_display SET payload=?3 WHERE session_id=?1 AND source_position=?2",
                params![&stored_session, position, payload.to_string()],
            )?;
        }
    }
    Ok(())
}

/// Rebuild the display rows from `changed_position` on for a legacy session, and
/// for exchanges belonging to the same run in a canonical one.
///
/// **The projection is the healed conversation, not the stored rows.**
/// `Manager::load` repairs a transcript before anything reads it — an assistant
/// that asked for a tool whose result never landed gains the synthetic
/// `[Tool execution lost — …]` result, a duplicate result is dropped, an
/// assistant left with no body is dropped — and a display page has to show the
/// same conversation, or the two readers of one journal disagree. A canonical
/// session used to skip this, on the assumption that the canon is already clean.
/// It is not: an interrupted run writes the assistant and its call but never the
/// result, so the same conversation showed the placeholder while it was legacy
/// and lost it once it was converted — the conversion was visible to the reader
/// even though every entry it wrote was byte-identical.
///
/// Returns the position this call indexed up to (exclusive), so a caller
/// rebuilding a whole conversation walks it one exchange at a time instead of
/// re-projecting the rest of it on every batch.
pub(crate) fn update_display_index_from(
    db: &Connection,
    session: &str,
    changed_position: i64,
) -> Result<i64> {
    let stored_session = crate::session::identity::session_key(db, session)?;
    let last: i64 = db.query_row(
        "SELECT coalesce(max(position),-1) FROM history_shapes WHERE session_id=?1",
        [&stored_session],
        |r| r.get(0),
    )?;
    if crate::session::content::format(db, session)? != crate::session::content::FORMAT_VERSION {
        // A legacy row carries its run annotation in the payload itself, so the
        // window has to be projected with the markers around it: the tail from
        // the affected boundary, once.
        rewrite_legacy_tail(db, &stored_session, session, changed_position)?;
        return Ok(last + 1);
    }
    // Every exchange whose shapes are on disk is healed, then the rows nothing has
    // indexed are appended. Both halves are bounded by `last`: a lazy rebuild
    // projects the batch it just wrote, not the rest of the conversation, and the
    // append is filtered in SQL — re-projecting the remainder on every batch is
    // what turned a read of a 14,000-entry conversation into a timeout.
    let first = affected_history_start(db, session, changed_position)?;
    let session_end: i64 = db.query_row(
        "SELECT coalesce(max(position)+1,0) FROM entries WHERE session_id=?1",
        [&stored_session],
        |r| r.get(0),
    )?;
    let mut boundaries: Vec<i64> = {
        let mut query = db.prepare_cached(
            "SELECT position FROM entries
              WHERE session_id=?1 AND entry_type='user' AND position>?2 ORDER BY position",
        )?;
        let rows = query
            .query_map(params![&stored_session, first], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<i64>>>()?;
        rows
    };
    boundaries.push(last + 1);
    let mut from = first;
    // Nothing more can join the exchange that just ended when the shapes stop
    // exactly on the entry that opens the next one: a rebuild batch filling its
    // slice just before a user message leaves the trailing exchange whole, and
    // the next batch does not walk back into it (its entries start a new run, so
    // `affected_history_start` has no reason to). Without this the last exchange
    // of such a batch is only ever appended plain - a call whose result never
    // landed keeps no `[Tool execution lost — …]` row.
    let shapes_end_at_exchange_boundary: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM entries WHERE session_id=?1 AND position=?2 AND entry_type='user')",
        params![&stored_session, last + 1],
        |row| row.get(0),
    )?;
    for &boundary in &boundaries {
        if from > last {
            break;
        }
        if boundary <= from {
            continue;
        }
        let mut to = boundary;
        // A late result/final message can belong to a run that started before
        // intervening user entries. Heal that run together: splitting it at
        // those entries manufactures a lost-tool result and leaves the real
        // result at the wrong display position. Only header keys are read.
        loop {
            let related: i64 = db.query_row(
                "SELECT coalesce(max(position)+1,?3) FROM entries WHERE session_id=?1 AND run_id IN
                 (SELECT run_id FROM entries WHERE session_id=?1 AND position>=?2 AND position<?3 AND run_id IS NOT NULL)",
                params![&stored_session,from,to], |row| row.get(0))?;
            if related <= to {
                break;
            }
            to = boundaries
                .iter()
                .copied()
                .find(|end| *end >= related)
                .unwrap_or(related);
        }
        // A window that runs past the shapes on disk is an exchange a batch
        // stopped inside; the next batch walks back to its start and heals it once
        // it is whole. The conversation's own end always counts as whole, and so
        // does a stop on an exchange boundary.
        let complete = to <= last + 1
            && (to < last + 1
                || session_end <= last + 1
                || (to == last + 1 && shapes_end_at_exchange_boundary));
        let to = to.min(last + 1);
        if to > from && complete {
            // A window the healing leaves alone still has to reach the index
            // where it belongs. The next healed window is written *after*
            // everything already indexed, so a skipped window is not merely
            // deferred: it becomes a hole that the final append fills in at the
            // end of the transcript, and the reader gets the conversation out of
            // order. Indexing it here keeps ordinals contiguous in position
            // order, and costs exactly what the append would have.
            if !rewrite_healed_window(db, &stored_session, from, to)? {
                append_plain_shapes(db, &stored_session, from, to)?;
            }
        } else if to > from {
            // A partial batch can publish its plain shapes; a later batch
            // finishes this window and replaces them with the healed view.
            append_plain_shapes(db, &stored_session, from, to)?;
        }
        from = to;
    }
    // Completed windows already decided which source entries remain visible.
    // Revisiting them here would resurrect duplicate/empty rows they removed.
    append_plain_shapes(db, &stored_session, from, last + 1)?;
    Ok(last + 1)
}

/// Project a whole conversation's display rows. A rewrite (conversion, reset,
/// fork, compaction) needs the same projection a single append does; the name
/// exists because the caller has no position to name.
pub(crate) fn project_history_index(
    db: &Connection,
    session: &str,
    changed_position: i64,
) -> Result<()> {
    update_display_index_from(db, session, changed_position).map(|_| ())
}

/// Today's legacy projection, unchanged: heal the tail from the boundary the
/// change touches, and rewrite it.
fn rewrite_legacy_tail(
    db: &Connection,
    stored_session: &crate::session::identity::StoredIdentity,
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
            params![stored_session, start],
            |r| r.get(0),
        )?
    };
    let start = if ordinal == 0 { 0 } else { start };
    let mut stmt = db.prepare_cached(
        "SELECT position,payload FROM history_shapes WHERE session_id=?1 AND position>=?2 ORDER BY position",
    )?;
    let rows = stmt
        .query_map(params![stored_session, start], |r| {
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
    let mut upsert = db.prepare_cached("INSERT INTO history_display(session_id,ordinal,source_position,is_user,payload) VALUES (?1,?2,?3,?4,?5)
        ON CONFLICT(session_id,ordinal) DO UPDATE SET source_position=excluded.source_position,is_user=excluded.is_user,payload=excluded.payload
        WHERE history_display.source_position IS NOT excluded.source_position OR history_display.payload<>excluded.payload")?;
    for (index, payload) in projected.iter().enumerate() {
        let source = payload["id"]
            .as_str()
            .and_then(|id| positions.get(id))
            .copied();
        upsert.execute(params![
            stored_session,
            ordinal + index as i64,
            source,
            payload["role"] == "user",
            payload.to_string()
        ])?;
    }
    db.execute(
        "DELETE FROM history_display WHERE session_id=?1 AND ordinal>=?2",
        params![stored_session, ordinal + projected.len() as i64],
    )?;
    Ok(())
}

/// Replace the display rows of `[start, end)` when the healing projection
/// differs from what the window stores, and leave them alone when it does not.
///
/// Returns whether it wrote: a window it left alone is still unindexed, and the
/// caller has to place those rows or they end up behind a later window.
fn rewrite_healed_window(
    db: &Connection,
    stored_session: &crate::session::identity::StoredIdentity,
    start: i64,
    end: i64,
) -> Result<bool> {
    let ordinal: i64 = if start == 0 {
        0
    } else {
        db.query_row(
            "SELECT coalesce((SELECT min(ordinal) FROM history_display
            WHERE session_id=?1 AND source_position>=?2),
            (SELECT max(ordinal)+1 FROM history_display WHERE session_id=?1),0)",
            params![stored_session, start],
            |r| r.get(0),
        )?
    };
    // An index with nothing at or after this position has to be rebuilt from its
    // first row: writing a window into an empty index would place it at zero.
    let start = if ordinal == 0 { 0 } else { start };
    let mut stmt = db.prepare_cached(
        "SELECT position,payload FROM history_shapes
          WHERE session_id=?1 AND position>=?2 AND position<?3 ORDER BY position",
    )?;
    let rows = stmt
        .query_map(params![stored_session, start, end], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if rows.is_empty() {
        return Ok(false);
    }
    let mut positions = std::collections::HashMap::new();
    let mut entries = Vec::with_capacity(rows.len());
    for (position, row) in rows {
        let entry: SessionEntry = serde_json::from_str(&row)?;
        positions.insert(entry.id.clone(), position);
        entries.push(entry);
    }
    let plain = project_entries(&entries);
    let mut healed = entries;
    repair::strip_empty_assistants(&mut healed);
    repair::dedupe_tool_entries(&mut healed);
    repair::repair_dangling_tool_calls(&mut healed);
    let projected = project_entries(&healed);
    if projected == plain {
        let stale_placeholder: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM history_display WHERE session_id=?1 AND source_position IS NULL
               AND ordinal>=?2 AND ordinal<coalesce(
                 (SELECT min(ordinal) FROM history_display WHERE session_id=?1 AND source_position>=?3),
                 (SELECT max(ordinal)+1 FROM history_display WHERE session_id=?1),0))",
            params![stored_session,ordinal,end],|row|row.get(0))?;
        if !stale_placeholder {
            return Ok(false);
        }
    }
    let mut upsert = db.prepare_cached("INSERT INTO history_display(session_id,ordinal,source_position,is_user,payload) VALUES (?1,?2,?3,?4,?5)
        ON CONFLICT(session_id,ordinal) DO UPDATE SET source_position=excluded.source_position,is_user=excluded.is_user,payload=excluded.payload
        WHERE history_display.source_position IS NOT excluded.source_position OR history_display.payload<>excluded.payload")?;
    for (index, payload) in projected.iter().enumerate() {
        let source = payload["id"]
            .as_str()
            .and_then(|id| positions.get(id))
            .copied();
        upsert.execute(params![
            stored_session,
            ordinal + index as i64,
            source,
            payload["role"] == "user",
            payload.to_string()
        ])?;
    }
    // A repaired exchange can be longer or shorter than what it replaced; the
    // rows after it are re-appended from their shapes by the next window.
    db.execute(
        "DELETE FROM history_display WHERE session_id=?1 AND ordinal>=?2",
        params![stored_session, ordinal + projected.len() as i64],
    )?;
    Ok(true)
}

/// Append the shapes in `[start, end)` that nothing has indexed yet, in position
/// order. Terminal annotations are joined from `runs` on bounded page reads, so
/// closing a long run does no suffix rebuild.
fn append_plain_shapes(
    db: &Connection,
    stored_session: &crate::session::identity::StoredIdentity,
    start: i64,
    end: i64,
) -> Result<()> {
    let mut ordinal: i64 = db.query_row(
        "SELECT coalesce(max(ordinal)+1,0) FROM history_display WHERE session_id=?1",
        [stored_session],
        |row| row.get(0),
    )?;
    let mut query=db.prepare_cached("SELECT h.position,h.payload FROM history_shapes h WHERE h.session_id=?1 AND h.position>=?2 AND h.position<?3 AND NOT EXISTS(SELECT 1 FROM history_display d WHERE d.session_id=h.session_id AND d.source_position=h.position) ORDER BY h.position")?;
    let rows = query
        .query_map(params![stored_session, start, end], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut insert=db.prepare_cached("INSERT INTO history_display(session_id,ordinal,source_position,is_user,payload) VALUES (?1,?2,?3,?4,?5)")?;
    for (position, raw) in rows {
        let entry: SessionEntry = serde_json::from_str(&raw)?;
        if let Some(payload) = project_entries(&[entry]).pop() {
            insert.execute(params![
                stored_session,
                ordinal,
                position,
                payload["role"] == "user",
                payload.to_string()
            ])?;
            ordinal += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "incremental_index_tests.rs"]
mod incremental_tests;
#[cfg(test)]
#[path = "index_tests.rs"]
mod tests;
