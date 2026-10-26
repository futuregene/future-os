//! Explicit rewrites retire only earlier canonical checkpoint receipts. Idle
//! slices visit one indexed receipt / 64 error owners, never historical bodies.
use crate::session::database;
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

pub(super) fn generation(db: &Connection, session: &str) -> Result<i64> {
    Ok(db.query_row(
        "SELECT coalesce((SELECT generation FROM content_history_versions WHERE session_id=?1),0)",
        [session],
        |row| row.get(0),
    )?)
}
pub(super) fn record(db: &Connection, session: &str, key: &str) -> Result<()> {
    db.execute("INSERT INTO checkpoint_generations(session_id,input_key,history_generation) VALUES (?1,?2,?3)",params![session,key,generation(db,session)?])?;
    Ok(())
}
pub(super) fn is_current(db: &Connection, session: &str, key: &str) -> Result<bool> {
    let stored:Option<i64>=db.query_row("SELECT history_generation FROM checkpoint_generations WHERE session_id=?1 AND input_key=?2",params![session,key],|row|row.get(0)).optional()?;
    Ok(
        stored.context("canonical checkpoint generation is missing; preserve its receipt")?
            == generation(db, session)?,
    )
}

pub(crate) fn advance(db: &Connection, session: &str) -> Result<()> {
    let previous = generation(db, session)?;
    let next = previous
        .checked_add(1)
        .context("history rewrite generation overflow")?;
    db.execute("INSERT INTO content_history_versions(session_id,generation) VALUES (?1,?2) ON CONFLICT(session_id) DO UPDATE SET generation=excluded.generation",params![session,next])?;
    db.execute("INSERT INTO retired_checkpoint_generations(session_id,through_generation)
        SELECT ?1,?2 WHERE EXISTS(SELECT 1 FROM checkpoint_generations WHERE session_id=?1 AND history_generation<=?2)
        ON CONFLICT(session_id) DO UPDATE SET through_generation=max(through_generation,excluded.through_generation)",params![session,previous])?;
    Ok(())
}

pub(super) fn reclaim(db: &mut Connection) -> Result<bool> {
    let scope:Option<(String,i64)>=db.query_row("SELECT session_id,through_generation FROM retired_checkpoint_generations ORDER BY session_id LIMIT 1",[],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
    let Some((session, through)) = scope else {
        return Ok(true);
    };
    database::try_maintenance_transaction(db, |tx| {
        let key:Option<String>=tx.query_row("SELECT input_key FROM checkpoint_generations WHERE session_id=?1 AND history_generation<=?2 ORDER BY history_generation,input_key LIMIT 1",params![session,through],|row|row.get(0)).optional()?;
        if let Some(key) = key {
            ensure!(
                key.len() == 64 && key.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "invalid canonical checkpoint retirement identity; preserve ownership"
            );
            let owner = format!("compaction-error:{key}");
            tx.execute("DELETE FROM content_references WHERE (session_id,owner_kind,owner_id,field) IN
                (SELECT session_id,owner_kind,owner_id,field FROM content_references WHERE session_id=?1 AND owner_kind='tool' AND owner_id=?2 LIMIT 64)",params![session,owner])?;
            let remaining:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM content_references WHERE session_id=?1 AND owner_kind='tool' AND owner_id=?2)",params![session,owner],|row|row.get(0))?;
            if !remaining {
                tx.execute(
                    "DELETE FROM compaction_operations WHERE session_id=?1 AND input_key=?2",
                    params![session, key],
                )?;
            }
        }
        let remaining:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM checkpoint_generations WHERE session_id=?1 AND history_generation<=?2)",params![session,through],|row|row.get(0))?;
        if !remaining {
            tx.execute("DELETE FROM retired_checkpoint_generations WHERE session_id=?1 AND through_generation=?2",params![session,through])?;
        }
        Ok(())
    })?;
    Ok(!db.query_row(
        "SELECT EXISTS(SELECT 1 FROM retired_checkpoint_generations)",
        [],
        |row| row.get::<_, bool>(0),
    )?)
}

#[cfg(test)]
#[path = "checkpoint_retirement_tests.rs"]
mod tests;
