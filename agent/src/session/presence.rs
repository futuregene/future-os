//! Durable catalogue evidence; absence is never a deletion event.
use super::history;
use super::Manager;
use anyhow::{ensure, Result};
use serde_json::json;

impl Manager {
    /// Catalogue diagnostics only: no history bodies, API keys or paths.
    pub(crate) fn storage_health(&self) -> Result<serde_json::Value> {
        let store = self.storage()?;
        let mut result=store.db.read(move |db| {
            let tx = db.transaction()?;
            let schema: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
            let mut result = json!({"schemaVersion":schema});
            for (key,query) in [
                ("sessions", "SELECT count(*) FROM sessions WHERE revision>=0"),
                ("deletedSessions", "SELECT count(*) FROM session_tombstones"),
                ("failedImports", "SELECT count(*) FROM legacy_imports WHERE status='skipped'"),
                ("unrecoverableQueuedInputs", "SELECT count(*) FROM queued_run_receipts WHERE status='recovery_failed'"),
                ("failedRunRecoveries", "SELECT count(*) FROM runs WHERE status='recovery_failed'"),
                ("pendingQueuedInputs", "SELECT count(*) FROM queued_run_receipts WHERE status IN ('queued','cancel_requested')"),
                ("catalogueConflicts", "SELECT count(*) FROM session_tombstones t JOIN sessions s ON s.id=t.session_id WHERE s.revision>=0"),
            ] {
                result[key] = tx.query_row(query, [], |row| row.get::<_,i64>(0))?.into();
            }
            // Parameterized rather than spelled out in the list above: a stale
            // literal here reports every session as pending the moment the
            // projection version moves.
            result["pendingHistoryIndexes"] = tx
                .query_row(
                    "SELECT count(*) FROM sessions s WHERE revision>=0 AND NOT EXISTS(SELECT 1 FROM history_index_state h WHERE h.session_id=s.id AND h.projection_version=?1)",
                    [history::PROJECTION_VERSION],
                    |row| row.get::<_, i64>(0),
                )?
                .into();
            for (key,stored) in [("databaseId","database_id"),("restoreGeneration","restore_generation")] {
                result[key] = tx.query_row("SELECT value FROM storage_meta WHERE key=?1",[stored],|row|row.get::<_,String>(0))?.into();
            }
            tx.commit()?;
            Ok(result)
        })?;
        result["io"] = store.db.metrics();
        Ok(result)
    }

    pub(crate) fn session_presence(&self, ids: &[String]) -> Result<serde_json::Value> {
        ensure!(
            ids.len() <= future_rpc::session_deletion::MAX_DELETE_SESSIONS,
            "too many session ids"
        );
        let ids = ids.to_vec();
        self.storage()?.db.call(move |db| {
            let tx = db.transaction()?;
            let database: String = tx.query_row(
                "SELECT value FROM storage_meta WHERE key='database_id'",
                [],
                |row| row.get(0),
            )?;
            let generation: String = tx.query_row(
                "SELECT value FROM storage_meta WHERE key='restore_generation'",
                [],
                |row| row.get(0),
            )?;
            let mut results = Vec::new();
            for id in ids {
                let key = super::identity::BoundIdentity::session(&tx, &id)?;
                // Presence answers ONE question: does this id belong to this
                // database? Any row in `sessions` is proof that it does —
                // including the `revision=-1` placeholder `bind_events` writes
                // when a session is created, before its first transcript entry
                // exists. Requiring `revision>=0` made a brand-new session
                // report `missing`, which the Desktop's source fence then
                // refused to prompt: the first message could never land, so the
                // revision could never advance. That is a deadlock, not
                // evidence that the conversation belongs to another database.
                //
                // Two rules keep the widening honest:
                // - a tombstone always wins. A deleted identity refuses
                //   re-creation, but constructing the refused session still
                //   leaves its placeholder row behind; that leftover row must
                //   not resurrect the id (`deleted` stays the answer).
                // - listing (`list_sessions`) still requires `revision>=0`, so
                //   an empty session stays hidden until its first turn lands.
                let (has_row, live_row, deleted, skipped): (bool, bool, bool, bool) = tx
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1),
                     EXISTS(SELECT 1 FROM sessions WHERE id=?1 AND revision>=0),
                     EXISTS(SELECT 1 FROM session_tombstones WHERE session_id=?1),
                     EXISTS(SELECT 1 FROM legacy_imports WHERE session_id=?1 AND status='skipped')",
                        [&key],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                    )?;
                ensure!(
                    !(live_row && deleted),
                    "session catalogue conflicts with a deletion tombstone"
                );
                let present = has_row && !deleted;
                let status = if deleted {
                    "deleted"
                } else if skipped {
                    "import_failed"
                } else if present {
                    "present"
                } else {
                    "missing"
                };
                results.push(json!({"sessionId":id,"status":status}));
            }
            tx.commit()?;
            Ok(json!({"databaseId":database,"restoreGeneration":generation,"sessions":results}))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_deletion_has_durable_evidence_and_cannot_be_resurrected() {
        let root = tempfile::tempdir().unwrap();
        let manager = Manager::new(root.path().join("sessions"));
        let mut session = crate::session::Session::new(".", "mock");
        session
            .entries
            .push(crate::session::SessionEntry::session_info(
                json!({"cwd":".","model":"mock"}),
                "mock".into(),
                String::new(),
            ));
        manager.save(&session).unwrap();
        let original = manager.session_presence(&[session.id.clone()]).unwrap();
        assert_eq!(original["sessions"][0]["status"], "present");
        manager.delete_many(&[session.id.clone()]).unwrap();
        assert!(manager.save(&session).is_err());
        let deleted = manager
            .session_presence(&[session.id.clone(), "never-existed".into()])
            .unwrap();
        assert_eq!(deleted["sessions"][0]["status"], "deleted");
        assert_eq!(deleted["sessions"][1]["status"], "missing");
        assert_eq!(deleted["databaseId"], original["databaseId"]);
        drop(manager);
        let reopened = Manager::new(root.path().join("sessions"));
        let after = reopened.session_presence(&[session.id]).unwrap();
        assert_eq!(after["sessions"][0]["status"], "deleted");
        assert_eq!(after["databaseId"], original["databaseId"]);
    }
}
