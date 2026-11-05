//! Disposable projections are rebuilt per conversation, never by rewriting
//! authoritative entries or replaying a run. Read bodies in bounded batches.
use anyhow::Result;
use rusqlite::{params, Connection};

/// Bump whenever [`ensure_history_index`] would produce a different index from
/// the same journal. `history_index_state.projection_version` records which
/// version built a session's rows, so a bump makes every session rebuild once,
/// lazily, on its next read — the projection is derived data, never the journal.
///
/// 2: `entry_shape` keeps a `compaction` entry's checkpoint. Version 1 shaped
/// every compaction row with `content: ""`, so `payload.checkpoint` was the
/// empty string: the divider lost its numbers, its `phase` (which decides
/// whether an in-turn checkpoint stays inside the exchange or splits it) and its
/// identity. Those rows must be rebuilt, not patched — the shape is the input.
///
/// 3: a canonical session's rows are the *healed* rows too
/// ([`super::update_display_index_from`]), not the stored ones. Version 2 copied
/// a canonical session's shapes straight through, so a run cut off between a
/// tool call and its result lost the `[Tool execution lost — …]` row a legacy
/// session showed for the same journal — and converting the legacy session into
/// the canon dropped the row. Rows built under 2 must be rebuilt once.
///
/// 4: a rebuild batch that stops exactly on an exchange boundary heals that
/// trailing exchange (it is whole: nothing more can join it). Under 3 the next
/// batch opened a new run, so `affected_history_start` never walked back into
/// the exchange the previous batch ended on and its rows stayed unhealed — the
/// same placeholder lost, but only for conversations where a batch boundary
/// landed there.
///
/// 5: every exchange window reaches the index in position order. Under 4 a
/// window the healing left alone was skipped, and the next healed window was
/// written behind everything already indexed, so the skipped window's rows were
/// appended at the end of the transcript: the same entries, in the wrong order
/// (measured on a 2,287-entry conversation: the eleven entries that ended one
/// run were displayed after a window twenty user turns later).
pub(crate) const PROJECTION_VERSION: i64 = 5;

/// Keep healthy reads on a read transaction. If reconstruction is needed,
/// release that snapshot before reserving a writer and recheck under the lock.
pub(crate) fn with_history_index<T>(
    db: &mut Connection,
    session: &str,
    read: impl FnOnce(&Connection) -> Result<T>,
) -> Result<T> {
    let tx = db.transaction()?;
    if history_index_ready(&tx, session)? {
        let result = read(&tx)?;
        tx.commit()?;
        return Ok(result);
    }
    tx.rollback()?;
    let tx = crate::session::database::begin_write_transaction(db)?;
    ensure_history_index(&tx, session)?;
    let result = read(&tx)?;
    tx.commit()?;
    Ok(result)
}

pub(super) fn history_index_ready(db: &Connection, session: &str) -> Result<bool> {
    let stored_session = crate::session::identity::session_key(db, session)?;
    // A ready flag alone is not evidence: the projections can be dropped under it
    // (an operator purge deletes the rows; a rebuild that raced that purge can
    // then mark the session ready with nothing in the table). A session that has
    // entries but no display rows is not indexed, whatever the flag says - that
    // is what made a converted conversation read as "history unavailable".
    Ok(db.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM history_index_state st
              WHERE st.session_id=?1 AND st.projection_version=?2
                AND (EXISTS(SELECT 1 FROM history_display d WHERE d.session_id=st.session_id)
                     OR NOT EXISTS(SELECT 1 FROM entries e WHERE e.session_id=st.session_id)))",
        params![&stored_session, PROJECTION_VERSION],
        |row| row.get(0),
    )?)
}

pub(crate) fn mark_history_index_ready(db: &Connection, session: &str) -> Result<()> {
    let stored_session = crate::session::identity::session_key(db, session)?;
    db.execute(
        "INSERT INTO history_index_state(session_id,projection_version)
        SELECT id,?2 FROM sessions WHERE id=?1
        ON CONFLICT(session_id) DO UPDATE SET projection_version=excluded.projection_version",
        params![&stored_session, PROJECTION_VERSION],
    )?;
    Ok(())
}

/// Caller owns a transaction, so readers cannot observe a partial rebuild.
pub(crate) fn ensure_history_index(db: &Connection, session: &str) -> Result<()> {
    let stored_session = crate::session::identity::session_key(db, session)?;
    if history_index_ready(db, session)? {
        return Ok(());
    }
    db.execute(
        "DELETE FROM history_shapes WHERE session_id=?1",
        [&stored_session],
    )?;
    db.execute(
        "DELETE FROM history_display WHERE session_id=?1",
        [&stored_session],
    )?;
    let canonical =
        crate::session::content::format(db, session)? == crate::session::content::FORMAT_VERSION;
    let source = if canonical {
        "entry_header_records"
    } else {
        "entry_records"
    };
    let mut position = -1;
    loop {
        let rows: Vec<(i64, String)> = {
            let mut query = db.prepare_cached(&format!("SELECT position,payload FROM {source} WHERE session_id=?1 AND position>?2 ORDER BY position LIMIT 64"))?;
            let rows = query
                .query_map(params![&stored_session, position], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })?
                .collect::<rusqlite::Result<_>>()?;
            rows
        };
        if rows.is_empty() {
            break;
        }
        for (next, raw) in rows {
            super::insert_entry_shape(db, session, next, &serde_json::from_str(&raw)?)?;
            position = next;
        }
    }
    super::project_history_index(db, session, 0)?;
    mark_history_index_ready(db, session)
}

impl crate::session::Manager {
    /// Leave invalid projections invisible until the last batch commits. Each
    /// batch releases the shared writer so interactive writes get their turn.
    pub(super) fn rebuild_history_index(&self, session: &str) -> Result<()> {
        let request = crate::session::database::request::Context::current();
        let _rebuild = loop {
            if let Some(request) = &request {
                request.check()?;
            }
            if let Some(guard) = self
                .history_rebuild
                .try_lock_for(std::time::Duration::from_millis(50))
            {
                break guard;
            }
        };
        let store = self.storage()?;
        let id = session.to_owned();
        let revision = store.db.call_derived("history_index_begin", move |db| {
            let work = |db: &mut Connection| -> Result<Option<i64>> {
                let tx = crate::session::database::begin_write_transaction(db)?;
                if history_index_ready(&tx, &id)? {
                    return Ok(None);
                }
                if crate::session::content::format(&tx, &id)?
                    != crate::session::content::FORMAT_VERSION
                {
                    ensure_history_index(&tx, &id)?;
                    tx.commit()?;
                    return Ok(None);
                }
                let key = crate::session::identity::session_key(&tx, &id)?;
                let revision = tx.query_row(
                    "SELECT revision FROM sessions WHERE id=?1 AND revision>=0",
                    [&key],
                    |row| row.get::<_, i64>(0),
                )?;
                tx.execute("DELETE FROM history_shapes WHERE session_id=?1", [&key])?;
                tx.execute("DELETE FROM history_display WHERE session_id=?1", [&key])?;
                tx.commit()?;
                Ok(Some(revision))
            };
            work(db)
        })?;
        let Some(revision) = revision else {
            return Ok(());
        };
        let started = std::time::Instant::now();
        let mut after = -1;
        loop {
            if let Some(request) = &request {
                request.check()?;
            }
            let id = session.to_owned();
            let next = store.db.call_derived("history_index_batch", move |db| {
                let work = |db: &mut Connection| rebuild_batch(db, &id, revision, after);
                work(db)
            })?;
            let Some(next) = next else {
                break;
            };
            after = next;
        }
        tracing::debug!(
            event = "history_projection_rebuilt",
            session_id = session,
            elapsed_ms = started.elapsed().as_millis() as u64
        );
        Ok(())
    }
}

fn rebuild_batch(
    db: &mut Connection,
    session: &str,
    revision: i64,
    after: i64,
) -> Result<Option<i64>> {
    let tx = crate::session::database::begin_write_transaction(db)?;
    // An interleaved foreground write may already have repaired the index.
    if history_index_ready(&tx, session)? {
        return Ok(None);
    }
    let key = crate::session::identity::session_key(&tx, session)?;
    let current: i64 = tx.query_row(
        "SELECT revision FROM sessions WHERE id=?1 AND revision>=0",
        [&key],
        |row| row.get(0),
    )?;
    anyhow::ensure!(
        current == revision,
        "history changed during projection rebuild; retry this read"
    );
    let rows = {
        let mut query = tx.prepare_cached("SELECT position,payload FROM entry_header_records WHERE session_id=?1 AND position>?2 ORDER BY position LIMIT 64")?;
        let rows = query
            .query_map(params![key, after], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let next = rows.last().map(|row| row.0);
    for (position, raw) in &rows {
        super::insert_entry_shape(&tx, session, *position, &serde_json::from_str(raw)?)?;
    }
    if rows.is_empty() {
        mark_history_index_ready(&tx, session)?;
    } else {
        // The shape insert advances `next`; the projection only closes an
        // exchange whose shapes are all on disk, and the next batch continues
        // inside the exchange this one stopped in.
        super::update_display_index_from(&tx, session, after + 1)?;
    }
    tx.commit()?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use crate::session::{Manager, Session, SessionEntry};
    use serde_json::json;

    #[test]
    fn canonical_rebuild_yields_between_batches_and_publishes_only_complete_index() {
        let root = tempfile::tempdir().unwrap();
        let manager = Manager::new(root.path().join("sessions"));
        let store = manager.storage().unwrap();
        store.reserve_segmented("rebuild-batches").unwrap();
        let entries = (0..200).map(|index| serde_json::json!({"id":format!("u-{index}"),"type":"user","role":"user","content":format!("turn {index}")})).collect();
        store.replace("rebuild-batches", entries).unwrap();
        store
            .db
            .call(|db| {
                db.execute(
                    "DELETE FROM history_index_state WHERE session_id='rebuild-batches'",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        let before = store.db.metrics()["writerJob"]["count"].as_u64().unwrap();
        manager.rebuild_history_index("rebuild-batches").unwrap();
        let after = store.db.metrics()["writerJob"]["count"].as_u64().unwrap();
        assert!(
            after - before >= 5,
            "the shared writer must be released between 64-row batches"
        );
        let (ready, rows) = store
            .db
            .read(|db| {
                Ok((
                    super::history_index_ready(db, "rebuild-batches")?,
                    db.query_row(
                        "SELECT count(*) FROM history_display WHERE session_id='rebuild-batches'",
                        [],
                        |row| row.get::<_, i64>(0),
                    )?,
                ))
            })
            .unwrap();
        assert!(ready);
        assert_eq!(rows, 200);
    }

    /// A rebuild batch is a slice of rows, and a slice can end exactly on the
    /// boundary between two exchanges. That trailing exchange is whole — nothing
    /// more can join it — so it must be projected healed like any other; if it is
    /// skipped, the next batch starts at the following user message, never walks
    /// back (a new message opens a new run), and the exchange keeps its unhealed
    /// rows forever. What that costs a reader: the `[Tool execution lost — …]`
    /// result of a call whose answer never landed.
    #[test]
    fn a_batch_that_ends_on_an_exchange_boundary_still_heals_that_exchange() {
        let root = tempfile::tempdir().unwrap();
        let manager = Manager::new(root.path().join("sessions"));
        let store = manager.storage().unwrap();
        store.reserve_segmented("boundary").unwrap();
        let mut entries = Vec::new();
        // Positions 0..=61: two whole exchanges, so the dangling one straddles the
        // 64th row rather than sitting at the very start.
        for index in 0..62 {
            entries.push(serde_json::json!({
                "id": format!("u-{index}"),
                "type": "user",
                "role": "user",
                "content": format!("turn {index}"),
                "meta": {"run_id": format!("run-{index}")}
            }));
        }
        // 62: a call the run never answered. 63: the run's terminal marker — the
        // last row of the first batch. 64: the next user message, the first row of
        // the second batch and the boundary the first batch stops on.
        entries.push(serde_json::json!({
            "id": "cut-off",
            "type": "assistant",
            "role": "assistant",
            "content": null,
            "tool_calls": [{
                "id": "call-cut-off",
                "type": "function",
                "function": {"name": "shell", "arguments": {"command": "cargo test"}}
            }],
            "meta": {"run_id": "run-cut-off"}
        }));
        entries.push(serde_json::json!({
            "id": "cut-off-terminal",
            "type": "run_terminal",
            "role": "system",
            "content": {"run_id": "run-cut-off", "state": "interrupted"}
        }));
        for index in 64..199 {
            entries.push(serde_json::json!({
                "id": format!("u-{index}"),
                "type": "user",
                "role": "user",
                "content": format!("turn {index}"),
                "meta": {"run_id": format!("run-{index}")}
            }));
        }
        store.replace("boundary", entries).unwrap();
        store
            .db
            .call(|db| {
                db.execute(
                    "DELETE FROM history_index_state WHERE session_id='boundary'",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        manager.rebuild_history_index("boundary").unwrap();
        let page = manager
            .history_page("boundary", None, Some(0), Some(1000))
            .unwrap();
        let ids: Vec<&str> = page["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["id"].as_str().unwrap())
            .collect();
        let assistant = ids.iter().position(|id| *id == "cut-off").unwrap();
        assert_eq!(
            ids[assistant + 1],
            "missing-tool:cut-off:call-cut-off",
            "the placeholder must follow the call it stands in for, not be appended at the end"
        );
        // The page holds one batch's worth of rows; the placeholder is its last
        // row, which is where the exchange the batch stopped on ends.
        assert_eq!(ids.len(), assistant + 2);
    }

    #[test]
    fn healthy_pages_and_metadata_do_not_wait_for_an_unrelated_writer() {
        let root = tempfile::tempdir().unwrap();
        let manager = Manager::new(root.path().join("sessions"));
        let mut session = Session::new(".", "mock");
        session
            .entries
            .push(SessionEntry::new_user("user", json!("retained history")));
        manager.save(&session).unwrap();
        let writer = rusqlite::Connection::open(manager.database_path()).unwrap();
        writer.execute_batch("BEGIN IMMEDIATE").unwrap();
        let page = manager
            .history_page(&session.id, None, None, Some(10))
            .unwrap();
        assert_eq!(page["entries"][0]["blocks"][0]["text"], "retained history");
        assert_eq!(manager.load_metadata(&session.id).unwrap().id, session.id);
        writer.execute_batch("ROLLBACK").unwrap();
    }

    #[test]
    fn incomplete_projection_rebuilds_without_changing_authoritative_rows() {
        let root = tempfile::tempdir().unwrap();
        let manager = Manager::new(root.path().join("sessions"));
        let mut session = Session::new(".", "mock");
        session
            .entries
            .push(SessionEntry::new_user("user", json!("历史中文𠮷")));
        session
            .entries
            .push(SessionEntry::new_assistant(json!("answer"), vec![]));
        manager.save(&session).unwrap();
        let before = manager.storage().unwrap().entries(&session.id).unwrap();
        let expected = manager
            .history_page(&session.id, None, None, Some(100))
            .unwrap();
        let id = session.id.clone();
        manager
            .storage()
            .unwrap()
            .db
            .call(move |db| {
                db.execute("DELETE FROM history_shapes WHERE session_id=?1", [&id])?;
                db.execute(
                    "DELETE FROM history_display WHERE session_id=?1 AND ordinal=0",
                    [&id],
                )?;
                Ok(())
            })
            .unwrap();
        assert_eq!(
            manager.storage_health().unwrap()["pendingHistoryIndexes"],
            1
        );
        assert_eq!(
            manager
                .history_page(&session.id, None, None, Some(100))
                .unwrap(),
            expected
        );
        assert_eq!(
            manager.storage().unwrap().entries(&session.id).unwrap(),
            before
        );
        assert_eq!(
            manager.storage_health().unwrap()["pendingHistoryIndexes"],
            0
        );
    }
}
