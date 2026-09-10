//! Body-free history index. Ordered message blocks remain the body store.
//! Reconciliation runs inside the transcript transaction, never on page reads.
use super::{
    display::project_entries, projection::hydrate_entry_projections, repair, SessionEntry,
};
use anyhow::Result;
use rusqlite::{params, Connection};
use serde_json::{json, Value};

/// Retain only the fields needed for visibility, pairing and run annotations.
/// In particular, do not duplicate text, reasoning, arguments or attachments.
fn shape(value: &Value) -> Value {
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
        "session_info" | "model_change" | "run_started" | "run_terminal"
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

pub(crate) fn insert_shape(
    db: &Connection,
    session: &str,
    position: i64,
    value: &Value,
) -> Result<()> {
    db.execute(
        "INSERT INTO history_shapes(session_id,position,payload) VALUES (?1,?2,?3)",
        params![session, position, shape(value).to_string()],
    )?;
    Ok(())
}

pub(crate) fn reconcile(db: &Connection, session: &str) -> Result<()> {
    let mut stmt = db.prepare(
        "SELECT position,payload FROM history_shapes WHERE session_id=?1 ORDER BY position",
    )?;
    let rows = stmt
        .query_map([session], |r| {
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
    for (ordinal, payload) in projected.iter().enumerate() {
        let source = payload["id"]
            .as_str()
            .and_then(|id| positions.get(id))
            .copied();
        upsert.execute(params![
            session,
            ordinal as i64,
            source,
            payload["role"] == "user",
            payload.to_string()
        ])?;
    }
    db.execute(
        "DELETE FROM history_display WHERE session_id=?1 AND ordinal>=?2",
        params![session, projected.len() as i64],
    )?;
    Ok(())
}

pub(crate) struct HistoryPage {
    pub rows: Vec<(String, Option<String>)>,
    pub has_more: bool,
    pub next_offset: i64,
}

pub(crate) fn read_page(
    db: &Connection,
    session: &str,
    before: Option<i64>,
    offset: Option<i64>,
    limit: Option<i64>,
) -> Result<HistoryPage> {
    let total: i64 = db.query_row(
        "SELECT COALESCE(MAX(ordinal)+1,0) FROM history_display WHERE session_id=?1",
        [session],
        |r| r.get(0),
    )?;
    let (start, end) = if let Some(before) = before {
        let end = before.clamp(0, total);
        let count = limit.unwrap_or(10).clamp(1, 100);
        let start: i64 = db.query_row("SELECT COALESCE((SELECT ordinal FROM history_display WHERE session_id=?1 AND is_user=1 AND ordinal<?2 ORDER BY ordinal DESC LIMIT 1 OFFSET ?3),0)",params![session,end,count-1],|r|r.get(0))?;
        (start, end)
    } else {
        let start = offset.unwrap_or(0).clamp(0, total);
        (
            start,
            (start + limit.unwrap_or(250).clamp(1, 1000)).min(total),
        )
    };
    let mut stmt = db.prepare("SELECT d.payload,e.payload FROM history_display d LEFT JOIN entry_records e ON e.session_id=d.session_id AND e.position=d.source_position WHERE d.session_id=?1 AND d.ordinal>=?2 AND d.ordinal<?3 ORDER BY d.ordinal")?;
    let mut cursor = stmt.query(params![session, start, end])?;
    let mut rows = Vec::new();
    let mut body_bytes = 0usize;
    while let Some(row) = cursor.next()? {
        let overlay: String = row.get(0)?;
        let body: Option<String> = row.get(1)?;
        body_bytes = body_bytes
            .saturating_add(overlay.len())
            .saturating_add(body.as_ref().map_or(0, String::len));
        rows.push((overlay, body));
        // Bound forward-page IO as well as response serialization. Backward
        // pages retain the existing whole-user-exchange contract.
        if before.is_none() && body_bytes >= 8 * 1024 * 1024 {
            break;
        }
    }
    let read_end = start + rows.len() as i64;
    Ok(HistoryPage {
        rows,
        has_more: if before.is_some() {
            start > 0
        } else {
            read_end < total
        },
        next_offset: if before.is_some() { start } else { read_end },
    })
}

/// Decode only selected bodies, outside the database worker. The index carries
/// cross-page annotations; single-row projection must not infer run boundaries.
pub(crate) fn materialize(
    page: HistoryPage,
    before: Option<i64>,
    offset: Option<i64>,
) -> Result<Value> {
    let mut entries = Vec::new();
    let mut bytes = 0usize;
    let mut next = page.next_offset;
    let mut more = page.has_more;
    for (overlay, raw) in page.rows {
        let overlay: Value = serde_json::from_str(&overlay)?;
        let mut payload = if let Some(raw) = raw.filter(|_| overlay["kind"] != "session_info") {
            let mut entry: SessionEntry = serde_json::from_str(&raw)?;
            hydrate_entry_projections(&mut entry);
            let mut value = project_entries(&[entry])
                .pop()
                .ok_or_else(|| anyhow::anyhow!("history index refers to a non-display entry"))?;
            for key in ["usage", "run"] {
                if let Some(annotation) = overlay.get(key) {
                    value[key] = annotation.clone();
                }
            }
            value
        } else {
            overlay
        };
        let size = serde_json::to_vec(&payload)?.len();
        if before.is_none() && !entries.is_empty() && bytes.saturating_add(size) > 8 * 1024 * 1024 {
            next = offset.unwrap_or(0).max(0) + entries.len() as i64;
            more = true;
            break;
        }
        bytes = bytes.saturating_add(size);
        entries.push(payload.take());
    }
    Ok(json!({"entries":entries,"hasMore":more,"nextOffset":next}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Manager, Session};

    #[test]
    fn paged_projection_matches_full_history_and_reconciles_appends() {
        let dir = tempfile::tempdir().unwrap();
        let manager = Manager::new(dir.path().join("sessions"));
        let mut session = Session::new("/synthetic", "mock");
        session.entries.push(SessionEntry::session_info(
            json!({"session_name":"old","tokens_in":0}),
            "mock".into(),
            String::new(),
        ));
        let mut user = SessionEntry::new_user(
            "user",
            json!([{"type":"text","text":"question"},{"type":"text","text":"hidden attachment context"}]),
        );
        user.meta = Some(json!({"run_id":"r","attachments":[{"name":"synthetic"}]}));
        session.entries.push(user);
        let assistant = SessionEntry::new_assistant(
            json!([{"type":"reasoning","text":"synthetic thought"},{"type":"text","text":"answer"},{"type":"tool_call","id":"tc","name":"read","args":{"path":"/synthetic"}}]),
            Vec::new(),
        );
        session.entries.push(assistant);
        session.entries.push(SessionEntry::session_info(
            json!({"session_name":"new","tokens_in":50,"tokens_cache_r":5}),
            "mock".into(),
            String::new(),
        ));
        session
            .entries
            .push(SessionEntry::run_terminal("r", "completed", 12, 34, None));
        manager.save(&session).unwrap();
        let assert_pages = || {
            let expected = project_entries(&manager.load(&session.id).unwrap().entries);
            let mut actual = Vec::new();
            let mut offset = 0;
            loop {
                let page = manager
                    .history_page(&session.id, None, Some(offset), Some(1))
                    .unwrap();
                actual.extend(page["entries"].as_array().unwrap().iter().cloned());
                if page["hasMore"] == false {
                    break;
                }
                offset = page["nextOffset"].as_i64().unwrap();
            }
            assert_eq!(actual, expected);
        };
        assert_pages();
        // Real result replaces the synthetic lost-tool slot when inserted
        // inside its tool-response window, without persisting a placeholder.
        session
            .entries
            .insert(3, SessionEntry::new_tool("tc", "real result"));
        manager.save(&session).unwrap();
        assert_pages();
        manager
            .append_entries(
                &session.id,
                &[SessionEntry::new_user("user", json!("next"))],
            )
            .unwrap();
        let page = manager
            .history_page(&session.id, Some(i64::MAX), None, Some(1))
            .unwrap();
        assert_eq!(page["entries"].as_array().unwrap().len(), 1);
        assert_eq!(page["entries"][0]["blocks"][0]["text"], "next");
        assert_pages();
    }

    #[test]
    fn cold_page_reads_only_selected_bodies() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("sessions");
        let manager = Manager::new(sessions.clone());
        let mut session = Session::new("/synthetic", "mock");
        for index in 0..200 {
            let mut user = SessionEntry::new_user("user", json!(format!("question-{index}")));
            user.id = format!("u-{index}");
            session.entries.push(user);
            let mut reply = SessionEntry::new_assistant(json!("x".repeat(16384)), Vec::new());
            reply.id = format!("a-{index}");
            session.entries.push(reply);
        }
        manager.save(&session).unwrap();
        manager.initialize().unwrap();
        // Break an off-page body AFTER indexing. A full load would fail, but
        // a cold tail page must never decode or return that body's payload.
        manager.test_execute("PRAGMA ignore_check_constraints=ON; UPDATE entries SET content_json='invalid' WHERE position=1; PRAGMA ignore_check_constraints=OFF;");
        drop(manager);
        let manager = Manager::new(sessions);
        let page = manager
            .history_page(&session.id, Some(i64::MAX), None, Some(1))
            .unwrap();
        assert_eq!(page["entries"].as_array().unwrap().len(), 2);
        assert_eq!(page["entries"][0]["id"], "u-199");
        assert!(manager.load(&session.id).is_err());
        let id = session.id.clone();
        let count = manager
            .storage()
            .unwrap()
            .db
            .call(move |db| {
                Ok(read_page(db, &id, Some(i64::MAX), None, Some(1))?
                    .rows
                    .len())
            })
            .unwrap();
        assert_eq!(count, 2);
        manager.delete(&session.id).unwrap();
        let count: i64 = manager
            .storage()
            .unwrap()
            .db
            .call(|db| Ok(db.query_row("SELECT count(*) FROM history_display", [], |r| r.get(0))?))
            .unwrap();
        assert_eq!(count, 0);
    }
}
