//! Durable acceptance facts. Restart preserves inputs and closes queued work;
//! it never reconstructs a scheduler or re-executes model/tool side effects.
use super::{Manager, SessionEntry};
use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde_json::Value;

impl Manager {
    pub(crate) fn queued_receipt(
        &self,
        session: &str,
        request: &str,
    ) -> Result<Option<(String, i64, String)>> {
        let (session, request) = (session.to_owned(), request.to_owned());
        self.storage()?.db.call(move |db| Ok(db.query_row("SELECT run_id,run_sequence,fingerprint FROM queued_run_receipts WHERE session_id=?1 AND client_request_id=?2",params![session,request],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?))
    }

    pub(crate) fn has_queued_receipt_run(&self, session: &str, run: &str) -> Result<bool> {
        let (session, run) = (session.to_owned(), run.to_owned());
        self.storage()?.db.call(move |db|Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM queued_run_receipts WHERE session_id=?1 AND run_id=?2)",params![session,run],|row|row.get(0))?))
    }

    pub(crate) fn record_queued_receipt(
        &self,
        session: &str,
        request: &str,
        ack: &crate::runtime::RunAck,
        fingerprint: &str,
        payload: &Value,
        user: &SessionEntry,
    ) -> Result<()> {
        let sequence = i64::try_from(
            ack.run_sequence
                .ok_or_else(|| anyhow::anyhow!("queued acknowledgement has no sequence"))?,
        )?;
        let (session, request, run, fingerprint, payload, user) = (
            session.to_owned(),
            request.to_owned(),
            ack.run_id.clone(),
            fingerprint.to_owned(),
            payload.to_string(),
            Self::serialize_entry(user)?,
        );
        self.storage()?.db.call(move |db| {
            db.execute("INSERT INTO queued_run_receipts(session_id,run_id,client_request_id,run_sequence,fingerprint,payload_json,user_entry_json,status)
                VALUES (?1,?2,?3,?4,?5,?6,?7,'queued')",params![session,run,request,sequence,fingerprint,payload,user])?;
            Ok(())
        })
    }

    pub(crate) fn cancel_queued_receipt(&self, session: &str, run: &str) -> Result<()> {
        self.cancel_queued_receipts(session, &[run.to_owned()])
    }

    pub(crate) fn cancel_queued_receipts(&self, session: &str, runs: &[String]) -> Result<()> {
        let (session, runs) = (session.to_owned(), runs.to_vec());
        self.storage()?.db.call(move |db| {
            let tx = super::database::begin_write_transaction(db)?;
            for run in runs {
                tx.execute("UPDATE queued_run_receipts SET status='cancel_requested' WHERE session_id=?1 AND run_id=?2 AND status='queued'",params![session,run])?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    pub(crate) fn finish_cancelled_receipts(&self, session: &str) -> Result<usize> {
        let result = self
            .storage()?
            .recover_queued_receipts(Some(session), true)?;
        if result > 0 {
            self.invalidate_display_projection(session);
        }
        Ok(result)
    }
}

impl super::sqlite_store::SqliteStore {
    pub(crate) fn recover_queued_receipts(
        &self,
        session: Option<&str>,
        only_cancelled: bool,
    ) -> Result<usize> {
        let session = session.map(str::to_owned);
        let mut total = 0;
        loop {
            let session = session.clone();
            let count = self.db.call(move |db| {
                let tx = super::database::begin_write_transaction(db)?;
                let rows: Vec<(String,String,i64,String,String)> = {
                    let mut query = tx.prepare("SELECT session_id,run_id,run_sequence,COALESCE(user_entry_json,'null'),status FROM queued_run_receipts
                        WHERE (status='cancel_requested' OR (?2=0 AND status='queued')) AND (?1 IS NULL OR session_id=?1)
                        ORDER BY session_id,run_sequence LIMIT 32")?;
                    let rows = query.query_map(params![session,only_cancelled],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)))?.collect::<rusqlite::Result<_>>()?;
                    rows
                };
                let count = rows.len();
                for (session,run,sequence,raw,status) in rows {
                    let present: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1 AND revision>=0)",[&session],|row|row.get(0))?;
                    if !present {
                        tx.execute("UPDATE queued_run_receipts SET status='recovery_failed' WHERE session_id=?1 AND run_id=?2",params![session,run])?;
                        continue;
                    }
                    let started: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM entries WHERE session_id=?1 AND run_id=?2 AND entry_type='run_started')",params![session,run],|row|row.get(0))?;
                    if started {
                        tx.execute("UPDATE queued_run_receipts SET status='started',payload_json=NULL,user_entry_json=NULL WHERE session_id=?1 AND run_id=?2",params![session,run])?;
                        continue;
                    }
                    let user = serde_json::from_str::<SessionEntry>(&raw).ok().filter(|user|
                        user.meta.as_ref().and_then(|meta|meta["run_id"].as_str()) == Some(run.as_str()));
                    let Some(user) = user else {
                        // One malformed receipt must not strand every other
                        // conversation. Retain its raw bytes for diagnosis.
                        tx.execute("UPDATE queued_run_receipts SET status='recovery_failed' WHERE session_id=?1 AND run_id=?2",params![session,run])?;
                        continue;
                    };
                    let cancelled = status == "cancel_requested";
                    let state = if cancelled { super::RUN_STATE_CANCELLED } else { super::RUN_STATE_INTERRUPTED_BY_RESTART };
                    let mut terminal = SessionEntry::run_terminal(&run,state,0,0,Some(if cancelled {"Queued run cancelled before execution."} else {"Queued input preserved after restart; execution was not resumed."}));
                    terminal.content.as_mut().expect("terminal content")["run_sequence"] = sequence.into();
                    super::sqlite_store::insert_transcript_entries(&tx,&session,vec![serde_json::from_str(&Manager::serialize_entry(&user)?)?,serde_json::from_str(&Manager::serialize_entry(&terminal)?)?])?;
                    tx.execute("UPDATE sessions SET revision=revision+1 WHERE id=?1",[&session])?;
                    tx.execute("UPDATE queued_run_receipts SET status=?3 WHERE session_id=?1 AND run_id=?2",params![session,run,if cancelled {"cancelled"} else {"interrupted"}])?;
                }
                tx.commit()?;
                Ok(count)
            })?;
            total += count;
            if count == 0 {
                return Ok(total);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn restart_preserves_accepted_inputs_once_without_resuming_execution() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("sessions");
        let manager = Manager::new(directory.clone());
        let mut session = super::super::Session::new(".", "mock");
        session
            .entries
            .push(SessionEntry::new_user("user", json!("original")));
        manager.save(&session).unwrap();
        for (run, sequence, cancelled) in [("queued", 4, false), ("cancelled", 5, true)] {
            let mut user = SessionEntry::new_user("user", json!(format!("accepted {run} 中文")));
            user.meta = Some(json!({"run_id":run}));
            manager
                .record_queued_receipt(
                    &session.id,
                    run,
                    &crate::runtime::RunAck::queued(run.into(), sequence, 1),
                    "fingerprint",
                    &json!({"message":run}),
                    &user,
                )
                .unwrap();
            if cancelled {
                manager.cancel_queued_receipt(&session.id, run).unwrap();
            }
        }
        drop(manager);
        let reopened = Manager::new(directory.clone());
        let entries = reopened.load(&session.id).unwrap().entries;
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.entry_type == "user")
                .count(),
            3
        );
        assert!(!entries
            .iter()
            .any(|entry| entry.entry_type == "run_started"));
        for (run, state) in [
            ("queued", "interrupted_by_restart"),
            ("cancelled", "cancelled"),
        ] {
            let terminal = entries
                .iter()
                .find(|entry| {
                    entry.entry_type == "run_terminal"
                        && entry
                            .content
                            .as_ref()
                            .is_some_and(|value| value["run_id"] == run)
                })
                .unwrap();
            assert_eq!(terminal.content.as_ref().unwrap()["state"], state);
        }
        assert_eq!(super::super::next_run_sequence(&entries), 6);
        assert!(reopened
            .queued_receipt(&session.id, "queued")
            .unwrap()
            .is_some());
        drop(reopened);
        let again = Manager::new(directory);
        assert_eq!(
            again.load(&session.id).unwrap().entries.len(),
            entries.len()
        );
    }

    #[test]
    fn missing_session_retains_raw_receipt_and_reports_recovery_failure() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("sessions");
        let manager = Manager::new(directory.clone());
        let mut user = SessionEntry::new_user("user", json!("preserve me"));
        user.meta = Some(json!({"run_id":"orphan-run"}));
        manager
            .record_queued_receipt(
                "missing",
                "request",
                &crate::runtime::RunAck::queued("orphan-run".into(), 1, 1),
                "fingerprint",
                &json!({"message":"preserve me"}),
                &user,
            )
            .unwrap();
        drop(manager);
        let reopened = Manager::new(directory);
        assert_eq!(
            reopened.storage_health().unwrap()["unrecoverableQueuedInputs"],
            1
        );
        reopened
            .storage()
            .unwrap()
            .db
            .call(|db| {
                let raw: String = db.query_row(
                    "SELECT user_entry_json FROM queued_run_receipts WHERE run_id='orphan-run'",
                    [],
                    |row| row.get(0),
                )?;
                assert!(raw.contains("preserve me"));
                Ok(())
            })
            .unwrap();
    }
}
