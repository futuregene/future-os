//! Terminal transcript and replay visibility share one SQLite transaction.
//! This is called only for an explicit run commit, never for history import,
//! fork or reset. No historical scan or completion-time body work is involved.
use super::stream::{append_boundary, StreamIdentity};
use anyhow::{ensure, Context, Result};
use rusqlite::{params, OptionalExtension, Transaction};
use serde_json::{json, Value};

pub(crate) fn publish(tx: &Transaction<'_>, session: &str, entry: &Value) -> Result<()> {
    let content = &entry["content"];
    let run = content["run_id"]
        .as_str()
        .context("terminal run identity missing")?;
    let (epoch, sequence): (i64, i64) = tx.query_row(
        "SELECT coalesce(epoch,0),coalesce(run_sequence,-1) FROM runs WHERE session_id=?1 AND run_id=?2",
        params![session,run], |row| Ok((row.get(0)?,row.get(1)?)),
    )?;
    let prior: Option<(String, String)> = tx.query_row(
        "SELECT c.event_type,c.metadata_json FROM stream_runs r JOIN stream_changes c ON c.stream_id=r.id WHERE r.session_id=?1 AND r.run_id=?2 AND r.epoch=?3 AND NOT EXISTS(SELECT 1 FROM retired_streams d WHERE d.stream_id=r.id) ORDER BY c.last_cursor DESC LIMIT 1",
        params![session,run,epoch], |row| Ok((row.get(0)?,row.get(1)?)),
    ).optional()?;
    if let Some((kind, metadata)) = prior.filter(|(kind, _)| kind == "agent_end") {
        let previous: Value = serde_json::from_str(&metadata)?;
        ensure!(
            previous["terminal_entry_id"] == entry["id"],
            "run already has a different terminal boundary ({kind})"
        );
        return Ok(());
    }
    let state = content["state"]
        .as_str()
        .context("terminal state missing")?;
    let mut data = json!({
        "type":"agent_end", "state":state,
        "usage":{"output_tokens":content["run_tokens"].as_i64().unwrap_or(0)},
        "duration_ms":content["run_duration_ms"].as_i64().unwrap_or(0),
    });
    for field in ["error", "truncation", "interruption_cause"] {
        if let Some(value) = content.get(field) {
            data[field] = value.clone();
        }
    }
    if state == crate::session::RUN_STATE_INCOMPLETE
        || state == crate::session::RUN_STATE_INTERRUPTED_BY_RESTART
    {
        data["reason"] = json!("incomplete");
    } else if state == crate::session::RUN_STATE_CANCELLED {
        data["reason"] = json!("cancelled");
    }
    let cursor = append_boundary(
        tx,
        &StreamIdentity {
            session_id: session.into(),
            run_id: run.into(),
            epoch,
        },
        "agent_end",
        json!({"timestamp":entry["timestamp"],"run_sequence":sequence,"terminal_entry_id":entry["id"],"data":data}),
    )?;
    // The entry owns the same immutable field. Publish another scoped range
    // link, never copy/read its body on this transaction's writer thread.
    for name in ["error", "truncation", "interruption_cause"] {
        let field = format!("/content/{name}");
        let range: Option<(i64,i64,i64,String)> = tx.query_row(
            "SELECT content_id,byte_offset,byte_length,encoding FROM content_references WHERE session_id=?1 AND owner_kind='entry' AND owner_id=?2 AND field=?3",
            params![session,entry["id"].as_str(),field], |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
        ).optional()?;
        if let Some((body, offset, length, encoding)) = range {
            let linked = tx.execute("INSERT INTO stream_change_content(change_id,field,encoding,content_id,byte_offset,byte_length)
                SELECT c.id,?5,?6,?7,?8,?9 FROM stream_runs r JOIN stream_changes c ON c.stream_id=r.id
                WHERE r.session_id=?1 AND r.run_id=?2 AND r.epoch=?3 AND c.last_cursor=?4",
                params![session,run,epoch,cursor,format!("/data/{name}"),encoding,body,offset,length])?;
            ensure!(linked == 1, "terminal field has no published change");
        }
    }
    Ok(())
}
