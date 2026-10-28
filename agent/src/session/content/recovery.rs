//! Startup recovery publishes only ranges covered by committed semantic changes.
//! Message IDs originate before generation, so no text/timestamp ownership guess
//! or replay of model/tool side effects is involved.
use super::{stream::*, ContentRange};
use crate::session::{sqlite_store::SqliteStore, SessionEntry, RUN_STATE_INTERRUPTED_BY_RESTART};
use anyhow::{Context, Result};
use rusqlite::params;
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct InvalidRecoveryData(&'static str);

fn is_local_recovery_failure(error: &anyhow::Error) -> bool {
    // Never downgrade a storage/worker failure into a broken conversation.
    // Only explicitly identified run data failures may be isolated.
    !error.is::<crate::service::StartupCancelled>()
        && !error.chain().any(|cause| {
            cause.is::<std::io::Error>()
                || matches!(
                    cause.downcast_ref::<rusqlite::Error>(),
                    Some(rusqlite::Error::SqliteFailure(..))
                )
        })
        && error.is::<InvalidRecoveryData>()
}

struct Draft {
    id: String,
    timestamp: String,
    blocks: Vec<Value>,
    keys: Vec<String>,
    links: Vec<(String, ContentRange, Encoding)>,
    metadata: Value,
    role: &'static str,
    tool_slots: HashMap<u64, usize>,
}
impl Draft {
    fn assistant(id: String, timestamp: String, run: &str) -> Self {
        Self {
            id,
            timestamp,
            blocks: Vec::new(),
            keys: Vec::new(),
            links: Vec::new(),
            metadata: json!({"run_id":run,"recovered_partial":true}),
            role: "assistant",
            tool_slots: HashMap::new(),
        }
    }
    fn text(&mut self, key: &str, kind: &str, range: ContentRange) -> Result<()> {
        let ordinal = if let Some(index) = self.keys.iter().position(|known| known == key) {
            index
        } else {
            let index = self.blocks.len();
            self.keys.push(key.to_owned());
            self.blocks.push(json!({"type":kind,"text":""}));
            index
        };
        let field = format!("/content/{ordinal}/text");
        if let Some((_, prior, _)) = self.links.iter_mut().find(|(known, _, _)| known == &field) {
            anyhow::ensure!(
                prior.id == range.id && prior.offset + prior.length == range.offset,
                InvalidRecoveryData("recovery content range has a gap")
            );
            prior.length += range.length;
        } else {
            self.links.push((field, range, Encoding::Text));
        }
        Ok(())
    }
    fn tool_slot(&mut self, data: &Value) -> Result<usize> {
        // The released SSE carrier omits tc_index for the first tool.
        let index = data["tc_index"].as_u64().unwrap_or(0);
        let key = data["content_key"].as_str().context(InvalidRecoveryData(
            "recovery tool input has no content identity",
        ))?;
        let ordinal = *self.tool_slots.entry(index).or_insert_with(|| {
            let ordinal = self.blocks.len();
            self.keys.push(key.into());
            self.blocks.push(json!({"type":"text","text":""}));
            ordinal
        });
        if self.keys[ordinal] != key {
            // A provider snapshot replaces its unfinished argument generation.
            // Keep the original chronological slot, without old prefix bytes.
            self.keys[ordinal] = key.into();
            self.blocks[ordinal] = json!({"type":"text","text":""});
            let prefix = format!("/content/{ordinal}/");
            self.links
                .retain(|(field, _, _)| !field.starts_with(&prefix));
        }
        Ok(ordinal)
    }
    fn value(&self) -> Value {
        let mut metadata = self.metadata.clone();
        if !self.tool_slots.is_empty() {
            metadata["recovered_incomplete_tool_input"] = json!(true);
        }
        json!({"id":self.id,"type":self.role,"role":self.role,"timestamp":self.timestamp,"meta":metadata,"content":self.blocks})
    }
}

impl SqliteStore {
    /// Called while startup still excludes executors. Old conversations are
    /// untouched; only explicit active format-2 run records enter this path.
    #[cfg(test)]
    pub(crate) fn recover_segmented_runs(&self) -> Result<()> {
        self.recover_segmented_runs_cancellable(&Default::default())
    }

    fn isolate_failed_recovery(
        &self,
        session: &str,
        run: &str,
        epoch: i64,
        error: &anyhow::Error,
    ) -> Result<()> {
        let (session, run, error) = (session.to_owned(), run.to_owned(), format!("{error:#}"));
        self.db.call(move |db| {
            let tx = crate::session::database::begin_write_transaction(db)?;
            let run = crate::session::identity::BoundIdentity::run(&tx, &session, &run)?;
            let session = crate::session::identity::BoundIdentity::session(&tx, &session)?;
            let changed = tx.execute("UPDATE runs SET status='recovery_failed',error=?4 WHERE session_id=?1 AND run_id=?2 AND coalesce(epoch,0)=?3 AND status='running'", params![session,run,epoch,error])?;
            anyhow::ensure!(changed == 1, "interrupted run changed during recovery isolation");
            tx.commit()?;
            Ok(())
        })
    }

    pub(crate) fn retry_segmented_recovery(&self, session: &str) -> Result<()> {
        let id = session.to_owned();
        let runs = self.db.read(move |db| {
            let id = crate::session::identity::BoundIdentity::session(db, &id)?;
            let mut query = db.prepare("SELECT run_id,coalesce(epoch,0),coalesce(run_sequence,-1) FROM runs WHERE session_id=?1 AND status='recovery_failed' ORDER BY run_id")?;
            let rows = query.query_map([id], |row| Ok((row.get::<_,crate::session::identity::BoundIdentity>(0)?,row.get::<_,i64>(1)?,row.get::<_,i64>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })?;
        anyhow::ensure!(
            !runs.is_empty(),
            "session has no isolated run recovery to retry"
        );
        let snapshot = self.db.call(|db| {
            let path = std::path::Path::new(db.path().context("recovery database has no path")?);
            crate::session::create_database_snapshot(
                db,
                &path
                    .parent()
                    .context("recovery database has no directory")?
                    .join("backups"),
                "agent-before-recovery-retry",
            )
        })?;
        tracing::info!(session, snapshot = %snapshot.display(), "preserved database before explicit recovery retry");
        let cancellation = crate::service::StartupCancellation::default();
        for (run, epoch, sequence) in runs {
            let (id, key) = (session.to_owned(), run.to_string());
            self.db.call(move |db| {
                let tx = crate::session::database::begin_write_transaction(db)?;
                let key = crate::session::identity::BoundIdentity::run(&tx, &id, &key)?;
                let id = crate::session::identity::BoundIdentity::session(&tx, &id)?;
                let changed = tx.execute("UPDATE runs SET status='running' WHERE session_id=?1 AND run_id=?2 AND status='recovery_failed'", params![id,key])?;
                anyhow::ensure!(changed == 1, "isolated run changed before retry");
                tx.commit()?;
                Ok(())
            })?;
            if let Err(error) =
                self.recover_segmented_run(session, &run, epoch, sequence, &cancellation)
            {
                self.isolate_failed_recovery(session, &run, epoch, &error)?;
                return Err(error).with_context(|| {
                    format!(
                        "recovery retry failed; pre-retry snapshot: {}",
                        snapshot.display()
                    )
                });
            }
        }
        Ok(())
    }

    pub(crate) fn recover_segmented_runs_cancellable(
        &self,
        cancellation: &crate::service::StartupCancellation,
    ) -> Result<()> {
        loop {
            cancellation.check()?;
            let runs=self.db.read(|db| {
                let mut query=db.prepare("SELECT r.session_id,r.run_id,coalesce(r.epoch,0),coalesce(r.run_sequence,-1) FROM runs r JOIN session_formats f ON f.session_id=r.session_id JOIN sessions s ON s.id=r.session_id WHERE f.format=2 AND r.status='running' AND s.revision>=0 ORDER BY r.session_id,r.run_id LIMIT 16")?;
                let rows=query.query_map([],|row|Ok((row.get::<_,crate::session::identity::BoundIdentity>(0)?,row.get::<_,crate::session::identity::BoundIdentity>(1)?,row.get::<_,i64>(2)?,row.get::<_,i64>(3)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;Ok(rows)
            })?;
            if runs.is_empty() {
                return Ok(());
            }
            for (session, run, epoch, sequence) in runs {
                cancellation.check()?;
                let started = std::time::Instant::now();
                tracing::info!(session = %session, run = %run, epoch, sequence, "session storage: recovering interrupted run");
                let result = self
                    .recover_segmented_run(&session, &run, epoch, sequence, cancellation)
                    .with_context(|| {
                        format!("recover interrupted run session={session} run={run} epoch={epoch}")
                    });
                if let Err(error) = result {
                    if !is_local_recovery_failure(&error) {
                        return Err(error);
                    }
                    self.isolate_failed_recovery(&session, &run, epoch, &error)?;
                    tracing::error!(session = %session, run = %run, epoch, error = %format!("{error:#}"), "session recovery isolated; original history and streams retained");
                    cancellation.record_progress();
                    continue;
                }
                cancellation.record_progress();
                tracing::info!(session = %session, run = %run, epoch, elapsed_ms = started.elapsed().as_millis() as u64, "session storage: interrupted run recovered");
            }
        }
    }

    fn recover_segmented_run(
        &self,
        session: &str,
        run: &str,
        epoch: i64,
        sequence: i64,
        cancellation: &crate::service::StartupCancellation,
    ) -> Result<()> {
        let (s, r) = (session.to_owned(), run.to_owned());
        let epochs = self.db.read(move |db| {
            let r=crate::session::identity::BoundIdentity::run(db,&s,&r)?;
            let s=crate::session::identity::BoundIdentity::session(db,&s)?;
            let mut query = db.prepare(
                "SELECT epoch FROM stream_runs WHERE session_id=?1 AND run_id=?2 AND NOT EXISTS(SELECT 1 FROM retired_streams d WHERE d.stream_id=stream_runs.id) ORDER BY epoch",
            )?;
            let rows = query
                .query_map(params![s, r], |row| row.get::<_, i64>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })?;
        let terminal_epoch_seen = epochs.contains(&epoch);
        for epoch in epochs {
            cancellation.check()?;
            let identity = StreamIdentity {
                session_id: session.into(),
                run_id: run.into(),
                epoch,
            };
            let mut draft: Option<Draft> = None;
            let mut cursor = -1;
            loop {
                cancellation.check()?;
                let scope = identity.clone();
                let page = self
                    .db
                    .read(move |db| {
                        let tx = db.transaction()?;
                        let page = read_encoded_changes(&tx, &scope, cursor, 128)
                            .context(InvalidRecoveryData("invalid recovery stream page"))?;
                        tx.commit()?;
                        Ok(page)
                    })
                    .with_context(|| {
                        format!("read recovery stream epoch={epoch} after_cursor={cursor}")
                    })?
                    .decode()
                    .context(InvalidRecoveryData("invalid encoded recovery data"))
                    .with_context(|| {
                        format!("decode recovery stream epoch={epoch} after_cursor={cursor}")
                    })?;
                cancellation.record_progress();
                for change in page.changes {
                    cancellation.check()?;
                    cursor = change.last_cursor;
                    anyhow::ensure!(
                        cursor <= page.durable_cursor,
                        InvalidRecoveryData("recovery crossed durable watermark")
                    );
                    let data = &change.metadata["data"];
                    let timestamp = change.metadata["timestamp"]
                        .as_str()
                        .context(InvalidRecoveryData("recovery event has no timestamp"))?
                        .to_owned();
                    let entry = data["entry_id"].as_str().unwrap_or("");
                    match change.event_type.as_str() {
                        "message_start" => {
                            if let Some(previous) = draft.take() {
                                self.publish_recovered(session, previous)?;
                            }
                            anyhow::ensure!(
                                !entry.is_empty(),
                                InvalidRecoveryData("recovery message has no identity")
                            );
                            if !self.entry_exists(session, entry)? {
                                draft = Some(Draft::assistant(entry.into(), timestamp, run));
                            }
                        }
                        "text_start" | "thinking_start" => {
                            if let Some(draft) = draft.as_mut().filter(|draft| draft.id == entry) {
                                let key = data["content_key"].as_str().context(
                                    InvalidRecoveryData("recovery start has no block identity"),
                                )?;
                                if !draft.keys.iter().any(|known| known == key) {
                                    draft.keys.push(key.into());
                                    draft.blocks.push(json!({"type":if change.event_type == "text_start" {"text"} else {"reasoning"},"text":""}));
                                }
                            }
                        }
                        "text_chunk" | "thinking_delta" => {
                            if let Some(draft) = draft.as_mut().filter(|draft| draft.id == entry) {
                                let key = data["content_key"].as_str().context(
                                    InvalidRecoveryData("recovery delta has no content identity"),
                                )?;
                                if let Some((_, range, _)) = change
                                    .bodies
                                    .iter()
                                    .find(|(field, _, _)| field == "/data/text")
                                {
                                    draft.text(
                                        key,
                                        if change.event_type == "text_chunk" {
                                            "text"
                                        } else {
                                            "reasoning"
                                        },
                                        *range,
                                    )?;
                                }
                            }
                        }
                        "tool_start" | "tool_delta"
                            if change.event_type == "tool_delta" || data["phase"] == "input" =>
                        {
                            if let Some(draft) = draft.as_mut().filter(|draft| draft.id == entry) {
                                draft.tool_slot(data)?;
                                let field = if change.event_type == "tool_start" {
                                    "/data/tool_args"
                                } else {
                                    "/data/text"
                                };
                                if let Some((_, range, _)) =
                                    change.bodies.iter().find(|(known, _, _)| known == field)
                                {
                                    draft.text(
                                        data["content_key"].as_str().context(
                                            InvalidRecoveryData(
                                                "recovery tool content identity missing",
                                            ),
                                        )?,
                                        "text",
                                        *range,
                                    )?;
                                }
                            }
                        }
                        "thinking_end" => {
                            if let Some(draft) = draft.as_mut().filter(|draft| draft.id == entry) {
                                if let Some(index) = data["content_key"].as_str().and_then(|key| {
                                    draft.keys.iter().position(|known| known == key)
                                }) {
                                    let provider_range = change
                                        .bodies
                                        .iter()
                                        .find(|(field, _, encoding)| {
                                            field == "/data/provider_metadata"
                                                && *encoding == Encoding::Json
                                        })
                                        .map(|(_, range, _)| *range);
                                    if let Some(range) = provider_range {
                                        draft.blocks[index]["provider_metadata"] = json!({});
                                        draft.links.push((
                                            format!("/content/{index}/provider_metadata"),
                                            range,
                                            Encoding::Json,
                                        ));
                                    } else if let Some(provider) = data
                                        .get("provider_metadata")
                                        .filter(|provider| provider.is_object())
                                    {
                                        draft.blocks[index]["provider_metadata"] = provider.clone();
                                    }
                                }
                            }
                        }
                        "tool_input_end" => {
                            if let Some(draft) = draft.as_mut().filter(|draft| draft.id == entry) {
                                if let Some((_, range, Encoding::Json)) = change
                                    .bodies
                                    .iter()
                                    .find(|(field, _, _)| field == "/data/tool_args")
                                {
                                    let provider_range = change
                                        .bodies
                                        .iter()
                                        .find(|(field, _, encoding)| {
                                            field == "/data/provider_metadata"
                                                && *encoding == Encoding::Json
                                        })
                                        .map(|(_, range, _)| *range);
                                    let provider = if provider_range.is_some() {
                                        json!({})
                                    } else {
                                        data.get("provider_metadata")
                                            .cloned()
                                            .unwrap_or_else(|| json!({}))
                                    };
                                    let ordinal = draft.tool_slot(data)?;
                                    let prefix = format!("/content/{ordinal}/");
                                    draft
                                        .links
                                        .retain(|(field, _, _)| !field.starts_with(&prefix));
                                    draft.blocks[ordinal] = json!({"type":"tool_call","id":data["tool_id"],"name":data["tool_name"],"args":"","provider_metadata":provider});
                                    draft.links.push((
                                        format!("/content/{ordinal}/args"),
                                        *range,
                                        Encoding::Json,
                                    ));
                                    if let Some(range) = provider_range {
                                        draft.links.push((
                                            format!("/content/{ordinal}/provider_metadata"),
                                            range,
                                            Encoding::Json,
                                        ));
                                    }
                                    draft
                                        .tool_slots
                                        .remove(&data["tc_index"].as_u64().unwrap_or(0));
                                }
                            }
                        }
                        "message_end" => {
                            if draft.as_ref().is_some_and(|draft| draft.id == entry) {
                                self.publish_recovered(
                                    session,
                                    draft.take().expect("matching draft"),
                                )?;
                            }
                        }
                        "tool_end"
                            if !entry.is_empty() && !self.entry_exists(session, entry)? =>
                        {
                            let error = super::transport::event_is_error(data);
                            let mut tool = Draft {
                                id: entry.into(),
                                timestamp,
                                role: "tool",
                                tool_slots: HashMap::new(),
                                keys: Vec::new(),
                                blocks: vec![
                                    json!({"type":"tool_result","tool_call_id":data["tool_id"],"content":"","is_error":error}),
                                ],
                                links: Vec::new(),
                                metadata: json!({"run_id":run,"recovered_partial":true}),
                            };
                            for (field, range, encoding) in change.bodies {
                                match field.as_str() {
                                    "/data/text" => tool.links.push((
                                        "/content/0/content".into(),
                                        range,
                                        encoding,
                                    )),
                                    "/data/shell_result" => {
                                        tool.metadata["shell_result"] = json!("");
                                        tool.links.push((
                                            "/meta/shell_result".into(),
                                            range,
                                            Encoding::Json,
                                        ));
                                    }
                                    _ => {}
                                }
                            }
                            self.publish_recovered(session, tool)?;
                        }
                        _ => {}
                    }
                }
                if !page.has_more {
                    break;
                }
            }
            if let Some(draft) = draft {
                self.publish_recovered(session, draft)?;
            }
            self.release_unpublished_stream_owners(&identity, cancellation)?;
        }
        if !terminal_epoch_seen {
            self.release_unpublished_stream_owners(
                &StreamIdentity {
                    session_id: session.into(),
                    run_id: run.into(),
                    epoch,
                },
                cancellation,
            )?;
        }
        let (session, run) = (session.to_owned(), run.to_owned());
        self.db.call(move|db| {
            let tx=crate::session::database::begin_write_transaction(db)?;
            let bound=StreamIdentity {session_id:session.clone(),run_id:run.clone(),epoch}.bind(&tx)?;
            let running:bool=tx.query_row("SELECT status='running' FROM runs WHERE session_id=?1 AND run_id=?2",params![bound.session_id,bound.run_id],|row|row.get(0))?;
            if !running {return Ok(());}
            let stopped: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM entries WHERE session_id=?1 AND run_id=?2 AND entry_type='run_stop_requested')",params![bound.session_id,bound.run_id],|row| row.get(0))?;
            let (state,cause,reason,error) = if stopped {
                (crate::session::RUN_STATE_CANCELLED,"user_stop","cancelled","Stopped by user before recovery. An operation without a committed result has an unknown outcome.")
            } else {
                (RUN_STATE_INTERRUPTED_BY_RESTART,"process_restart","incomplete","Agent restarted before completion. An operation without a committed result has an unknown outcome.")
            };
            let mut terminal=SessionEntry::run_terminal(&run,state,0,0,Some(error));
            terminal.content.as_mut().context("terminal content missing")?["interruption_cause"] = json!(cause);
            let timestamp=terminal.timestamp.to_rfc3339();
            crate::session::sqlite_store::insert_transcript_entries(&tx,&session,vec![serde_json::to_value(terminal)?])?;
            let scope=StreamIdentity {session_id:session.clone(),run_id:run,epoch};
            append_boundary(&tx,&scope,"agent_end",json!({"timestamp":timestamp,"run_sequence":sequence,"data":{"type":"agent_end","state":state,"reason":reason,"interruption_cause":cause}}))?;
            tx.execute("UPDATE sessions SET revision=revision+1 WHERE id=?1",[bound.session_id])?;
            tx.commit()?;Ok(())
        })
    }

    /// Only an active interrupted run's scoped staging owners are visited.
    /// Each job examines at most 64 metadata rows; published change ranges and
    /// recovered entry references remain authoritative. No historical sweep.
    fn release_unpublished_stream_owners(
        &self,
        stream: &StreamIdentity,
        cancellation: &crate::service::StartupCancellation,
    ) -> Result<()> {
        let mut after = -1i64;
        loop {
            cancellation.check()?;
            let scope = stream.clone();
            let page=self.db.call(move|db| {
                let tx=crate::session::database::begin_write_transaction(db)?;
                let scope=scope.bind(&tx)?;
                let owners:Vec<(i64,i64,i64)>={
                    let mut query=tx.prepare("SELECT r.id,b.key_id,b.content_id FROM stream_runs r JOIN stream_body_owners b ON b.stream_id=r.id WHERE r.session_id=?1 AND r.run_id=?2 AND r.epoch=?3 AND b.key_id>?4 ORDER BY b.key_id LIMIT 64")?;
                    let rows=query.query_map(params![scope.session_id,scope.run_id,scope.epoch,after],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;rows
                };
                for (stream,key,content) in &owners {
                    let published:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM stream_content_ranges b JOIN stream_changes c ON c.id=b.change_id WHERE b.content_id=?1 AND c.stream_id=?2)",params![content,stream],|r|r.get(0))?;
                    if !published {tx.execute("DELETE FROM stream_body_owners WHERE stream_id=?1 AND key_id=?2",params![stream,key])?;}
                }
                let next=owners.last().map(|(_,key,_)|*key);tx.commit()?;Ok(next)
            })?;
            cancellation.record_progress();
            let Some(next) = page else {
                break;
            };
            after = next;
        }
        self.db.request_space_reclamation();
        Ok(())
    }

    fn entry_exists(&self, session: &str, entry: &str) -> Result<bool> {
        let (session, entry) = (session.to_owned(), entry.to_owned());
        self.db.read(move |db| {
            // Native UUIDs are BLOB keys; binding their wire spelling as TEXT
            // misses an already committed entry and recovery conflicts with it.
            let entry = crate::session::identity::BoundIdentity::entry(db, &session, &entry)?;
            let session = crate::session::identity::BoundIdentity::session(db, &session)?;
            Ok(db.query_row(
                "SELECT EXISTS(SELECT 1 FROM entries WHERE session_id=?1 AND entry_id=?2)",
                params![session, entry],
                |row| row.get(0),
            )?)
        })
    }

    fn publish_recovered(&self, session: &str, draft: Draft) -> Result<()> {
        if draft.blocks.is_empty() {
            return Ok(());
        }
        // Reference preparation is bounded and retryable. The transcript header
        // becomes visible only after every range is owned by this exact entry.
        for links in draft.links.chunks(64) {
            let (session, entry, links) = (session.to_owned(), draft.id.clone(), links.to_vec());
            self.db.call(move |db| {
                let tx = crate::session::database::begin_write_transaction(db)?;
                for (field, range, encoding) in links {
                    super::bind_encoded(&tx, &session, "entry", &entry, &field, range, encoding)
                        .context(InvalidRecoveryData(
                            "invalid recovered entry content reference",
                        ))?;
                }
                tx.commit()?;
                Ok(())
            })?;
        }
        let session = session.to_owned();
        self.db.call(move |db| {
            let tx = crate::session::database::begin_write_transaction(db)?;
            crate::session::sqlite_store::insert_transcript_entries(
                &tx,
                &session,
                vec![draft.value()],
            )
            .context(InvalidRecoveryData(
                "recovered entry conflicts with committed history",
            ))?;
            let session = crate::session::identity::BoundIdentity::session(&tx, &session)?;
            tx.execute(
                "UPDATE sessions SET revision=revision+1 WHERE id=?1",
                [session],
            )?;
            tx.commit()?;
            Ok(())
        })
    }
}

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;
