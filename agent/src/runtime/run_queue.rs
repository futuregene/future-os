use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use super::{BusyPolicy, RunAcceptedState, RunAck};

const QUEUE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DurableRunRequest {
    pub session_id: String,
    pub run_id: String,
    pub run_sequence: u64,
    pub client_request_id: String,
    pub request_digest: String,
    pub busy_policy: BusyPolicy,
    pub payload: Value,
    pub accepted_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunTerminalState {
    Completed,
    Failed,
    Aborted,
    Superseded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueuedCancellationReason {
    Cancelled,
    Superseded,
    SessionDeleted,
}

#[derive(Debug, Error)]
pub enum QueueError {
    #[error("queue persistence failed: {0}")]
    Io(#[from] io::Error),
    #[error("queue journal is corrupt at line {line}: {message}")]
    Corrupt { line: usize, message: String },
    #[error("client_request_id must not be empty")]
    MissingClientRequestId,
    #[error("client_request_id `{0}` was already used with a different request")]
    DuplicateRequestConflict(String),
    #[error("run_id `{0}` already exists")]
    DuplicateRunId(String),
    #[error("run `{actual}` cannot start; FIFO head is `{expected}`")]
    NotQueueHead { expected: String, actual: String },
    #[error("run `{0}` is not queued")]
    RunNotQueued(String),
    #[error("run `{actual}` cannot become terminal; active run is {expected:?}")]
    RunNotActive {
        expected: Option<String>,
        actual: String,
    },
    #[error("session already has active run `{0}`")]
    ActiveRunExists(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum QueueRecord {
    RunAccepted {
        schema_version: u32,
        run: DurableRunRequest,
    },
    RunStarted {
        schema_version: u32,
        run_id: String,
        epoch: u64,
    },
    RunTerminal {
        schema_version: u32,
        run_id: String,
        state: RunTerminalState,
    },
    QueuedRunsCancelled {
        schema_version: u32,
        run_ids: Vec<String>,
        reason: QueuedCancellationReason,
    },
}

#[derive(Debug, Clone)]
struct RequestIdentity {
    digest: String,
    run_id: String,
}

#[derive(Debug, Default)]
struct QueueState {
    next_sequence: u64,
    active: Option<(DurableRunRequest, u64)>,
    queued: VecDeque<DurableRunRequest>,
    requests: HashMap<String, RequestIdentity>,
    run_ids: HashSet<String>,
}

/// Append-only, fsync-before-ack scheduler journal for one session.
#[derive(Debug)]
pub struct DurableRunQueue {
    path: PathBuf,
    session_id: String,
    state: Mutex<QueueState>,
}

impl DurableRunQueue {
    pub fn open(
        path: impl Into<PathBuf>,
        session_id: impl Into<String>,
    ) -> Result<Self, QueueError> {
        let path = path.into();
        let session_id = session_id.into();
        let state = replay(&path, &session_id)?;
        Ok(Self {
            path,
            session_id,
            state: Mutex::new(state),
        })
    }

    /// Durably accepts a request. A retry with the same request id and payload
    /// returns the original run identity; a changed payload fails closed.
    pub fn accept(
        &self,
        client_request_id: &str,
        requested_run_id: Option<&str>,
        busy_policy: BusyPolicy,
        payload: Value,
    ) -> Result<RunAck, QueueError> {
        if client_request_id.trim().is_empty() {
            return Err(QueueError::MissingClientRequestId);
        }

        let digest = request_digest(requested_run_id, busy_policy, &payload);
        let mut state = self.state.lock();
        if let Some(identity) = state.requests.get(client_request_id) {
            if identity.digest != digest {
                return Err(QueueError::DuplicateRequestConflict(
                    client_request_id.to_string(),
                ));
            }
            return Ok(existing_ack(&state, &identity.run_id));
        }

        let run_id = requested_run_id
            .map(str::to_owned)
            .unwrap_or_else(|| format!("run_{}", Uuid::new_v4().simple()));
        if state.run_ids.contains(&run_id) {
            return Err(QueueError::DuplicateRunId(run_id));
        }
        let run_sequence = state.next_sequence.max(1);
        let run = DurableRunRequest {
            session_id: self.session_id.clone(),
            run_id: run_id.clone(),
            run_sequence,
            client_request_id: client_request_id.to_string(),
            request_digest: digest.clone(),
            busy_policy,
            payload,
            accepted_at: chrono::Utc::now().to_rfc3339(),
        };
        append_synced(
            &self.path,
            &QueueRecord::RunAccepted {
                schema_version: QUEUE_SCHEMA_VERSION,
                run: run.clone(),
            },
        )?;

        state.next_sequence = run_sequence + 1;
        state.requests.insert(
            client_request_id.to_string(),
            RequestIdentity {
                digest,
                run_id: run_id.clone(),
            },
        );
        state.run_ids.insert(run_id.clone());
        state.queued.push_back(run);
        Ok(RunAck::queued(
            run_id,
            run_sequence,
            state.queued.len() as u64,
        ))
    }

    pub fn mark_started(&self, run_id: &str, epoch: u64) -> Result<DurableRunRequest, QueueError> {
        let mut state = self.state.lock();
        if let Some((active, _)) = &state.active {
            return Err(QueueError::ActiveRunExists(active.run_id.clone()));
        }
        let Some(head) = state.queued.front() else {
            return Err(QueueError::RunNotQueued(run_id.to_string()));
        };
        if head.run_id != run_id {
            return Err(QueueError::NotQueueHead {
                expected: head.run_id.clone(),
                actual: run_id.to_string(),
            });
        }
        append_synced(
            &self.path,
            &QueueRecord::RunStarted {
                schema_version: QUEUE_SCHEMA_VERSION,
                run_id: run_id.to_string(),
                epoch,
            },
        )?;
        let run = state.queued.pop_front().expect("FIFO head checked");
        state.active = Some((run.clone(), epoch));
        Ok(run)
    }

    pub fn mark_terminal(
        &self,
        run_id: &str,
        terminal_state: RunTerminalState,
    ) -> Result<(), QueueError> {
        let mut state = self.state.lock();
        let active_id = state.active.as_ref().map(|(run, _)| run.run_id.clone());
        if active_id.as_deref() != Some(run_id) {
            return Err(QueueError::RunNotActive {
                expected: active_id,
                actual: run_id.to_string(),
            });
        }
        append_synced(
            &self.path,
            &QueueRecord::RunTerminal {
                schema_version: QUEUE_SCHEMA_VERSION,
                run_id: run_id.to_string(),
                state: terminal_state,
            },
        )?;
        state.active = None;
        Ok(())
    }

    /// Cancels one queued run at a durable boundary. The active run is never
    /// affected by this operation.
    pub fn cancel_queued(
        &self,
        run_id: &str,
        reason: QueuedCancellationReason,
    ) -> Result<DurableRunRequest, QueueError> {
        let mut state = self.state.lock();
        let Some(index) = state.queued.iter().position(|run| run.run_id == run_id) else {
            return Err(QueueError::RunNotQueued(run_id.to_string()));
        };
        append_synced(
            &self.path,
            &QueueRecord::QueuedRunsCancelled {
                schema_version: QUEUE_SCHEMA_VERSION,
                run_ids: vec![run_id.to_string()],
                reason,
            },
        )?;
        Ok(state
            .queued
            .remove(index)
            .expect("queued run index checked"))
    }

    /// Atomically cancels the complete queued tail. This is the queue half of
    /// `supersede_session`; the replacement request is accepted only after
    /// this record has reached disk.
    pub fn cancel_all_queued(
        &self,
        reason: QueuedCancellationReason,
    ) -> Result<Vec<DurableRunRequest>, QueueError> {
        let mut state = self.state.lock();
        if state.queued.is_empty() {
            return Ok(Vec::new());
        }
        let run_ids = state.queued.iter().map(|run| run.run_id.clone()).collect();
        append_synced(
            &self.path,
            &QueueRecord::QueuedRunsCancelled {
                schema_version: QUEUE_SCHEMA_VERSION,
                run_ids,
                reason,
            },
        )?;
        Ok(state.queued.drain(..).collect())
    }

    pub fn active(&self) -> Option<(DurableRunRequest, u64)> {
        self.state.lock().active.clone()
    }

    pub fn queued(&self) -> Vec<DurableRunRequest> {
        self.state.lock().queued.iter().cloned().collect()
    }
}

fn existing_ack(state: &QueueState, run_id: &str) -> RunAck {
    if let Some((run, epoch)) = &state.active {
        if run.run_id == run_id {
            let mut ack = RunAck::existing(run_id.to_string(), *epoch);
            ack.run_sequence = Some(run.run_sequence);
            return ack;
        }
    }
    if let Some((index, run)) = state
        .queued
        .iter()
        .enumerate()
        .find(|(_, run)| run.run_id == run_id)
    {
        let mut ack = RunAck::existing(run_id.to_string(), 0);
        ack.run_sequence = Some(run.run_sequence);
        ack.queue_position = Some(index as u64 + 1);
        return ack;
    }
    RunAck::existing(run_id.to_string(), 0)
}

fn append_synced(path: &Path, record: &QueueRecord) -> Result<(), QueueError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut bytes = serde_json::to_vec(record).map_err(|error| QueueError::Corrupt {
        line: 0,
        message: error.to_string(),
    })?;
    bytes.push(b'\n');
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(&bytes)?;
    file.sync_data()?;
    Ok(())
}

fn replay(path: &Path, session_id: &str) -> Result<QueueState, QueueError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(QueueState::default()),
        Err(error) => return Err(error.into()),
    };
    let mut state = QueueState::default();
    let mut line_number = 0;
    for raw in bytes.split_inclusive(|byte| *byte == b'\n') {
        line_number += 1;
        if !raw.ends_with(b"\n") {
            break; // The only tolerated corruption is a torn final append.
        }
        let line = &raw[..raw.len() - 1];
        if line.iter().all(|byte| byte.is_ascii_whitespace()) {
            continue;
        }
        let record: QueueRecord =
            serde_json::from_slice(line).map_err(|error| QueueError::Corrupt {
                line: line_number,
                message: error.to_string(),
            })?;
        apply_record(&mut state, session_id, record).map_err(|message| QueueError::Corrupt {
            line: line_number,
            message,
        })?;
    }
    Ok(state)
}

fn apply_record(
    state: &mut QueueState,
    session_id: &str,
    record: QueueRecord,
) -> Result<(), String> {
    match record {
        QueueRecord::RunAccepted {
            schema_version,
            run,
        } => {
            check_schema(schema_version)?;
            if run.session_id != session_id {
                return Err(format!(
                    "record belongs to session `{}`, expected `{session_id}`",
                    run.session_id
                ));
            }
            if state.run_ids.contains(&run.run_id)
                || state.requests.contains_key(&run.client_request_id)
            {
                return Err("duplicate durable run identity".to_string());
            }
            if run.run_sequence < state.next_sequence.max(1) {
                return Err("run_sequence is not monotonic".to_string());
            }
            state.next_sequence = run.run_sequence + 1;
            state.requests.insert(
                run.client_request_id.clone(),
                RequestIdentity {
                    digest: run.request_digest.clone(),
                    run_id: run.run_id.clone(),
                },
            );
            state.run_ids.insert(run.run_id.clone());
            state.queued.push_back(run);
        }
        QueueRecord::RunStarted {
            schema_version,
            run_id,
            epoch,
        } => {
            check_schema(schema_version)?;
            if state.active.is_some() {
                return Err("run_started while another run is active".to_string());
            }
            let Some(head) = state.queued.pop_front() else {
                return Err("run_started without a queued run".to_string());
            };
            if head.run_id != run_id {
                return Err(format!(
                    "run_started violates FIFO: expected `{}`, got `{run_id}`",
                    head.run_id
                ));
            }
            state.active = Some((head, epoch));
        }
        QueueRecord::RunTerminal {
            schema_version,
            run_id,
            ..
        } => {
            check_schema(schema_version)?;
            let Some((active, _)) = &state.active else {
                return Err("terminal record without an active run".to_string());
            };
            if active.run_id != run_id {
                return Err(format!(
                    "terminal run `{run_id}` does not match active run `{}`",
                    active.run_id
                ));
            }
            state.active = None;
        }
        QueueRecord::QueuedRunsCancelled {
            schema_version,
            run_ids,
            ..
        } => {
            check_schema(schema_version)?;
            if run_ids.is_empty() {
                return Err("queued cancellation record is empty".to_string());
            }
            let cancelled: HashSet<_> = run_ids.iter().collect();
            if cancelled.len() != run_ids.len() {
                return Err("queued cancellation record contains duplicate run ids".to_string());
            }
            if run_ids
                .iter()
                .any(|run_id| !state.queued.iter().any(|run| &run.run_id == run_id))
            {
                return Err("queued cancellation references a non-queued run".to_string());
            }
            state.queued.retain(|run| !cancelled.contains(&run.run_id));
        }
    }
    Ok(())
}

fn check_schema(version: u32) -> Result<(), String> {
    if version == QUEUE_SCHEMA_VERSION {
        Ok(())
    } else {
        Err(format!("unsupported queue schema version {version}"))
    }
}

fn request_digest(
    requested_run_id: Option<&str>,
    busy_policy: BusyPolicy,
    payload: &Value,
) -> String {
    let canonical = canonicalize(payload);
    let envelope = serde_json::json!({
        "busy_policy": busy_policy.as_str(),
        "payload": canonical,
        "requested_run_id": requested_run_id,
    });
    let bytes = serde_json::to_vec(&envelope).expect("JSON value serialization cannot fail");
    let hash = Sha256::digest(bytes);
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn canonicalize(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), canonicalize(value)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(canonicalize).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn queue(dir: &tempfile::TempDir) -> DurableRunQueue {
        DurableRunQueue::open(dir.path().join("queue.jsonl"), "session-a").unwrap()
    }

    #[test]
    fn preserves_fifo_and_monotonic_sequence() {
        let dir = tempfile::tempdir().unwrap();
        let queue = queue(&dir);
        let first = queue
            .accept(
                "request-1",
                Some("run-1"),
                BusyPolicy::RejectIfBusy,
                serde_json::json!({"text":"a"}),
            )
            .unwrap();
        let second = queue
            .accept(
                "request-2",
                Some("run-2"),
                BusyPolicy::EnqueueIfBusy,
                serde_json::json!({"text":"b"}),
            )
            .unwrap();
        assert_eq!(first.run_sequence, Some(1));
        assert_eq!(first.queue_position, Some(1));
        assert_eq!(second.run_sequence, Some(2));
        assert_eq!(second.queue_position, Some(2));
        let error = queue.mark_started("run-2", 1).unwrap_err();
        assert!(matches!(error, QueueError::NotQueueHead { .. }));
        assert_eq!(queue.mark_started("run-1", 4).unwrap().run_id, "run-1");
        queue
            .mark_terminal("run-1", RunTerminalState::Completed)
            .unwrap();
        assert_eq!(queue.mark_started("run-2", 5).unwrap().run_id, "run-2");
    }

    #[test]
    fn replay_restores_active_queue_and_idempotency() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.jsonl");
        {
            let queue = DurableRunQueue::open(&path, "session-a").unwrap();
            queue
                .accept(
                    "request-1",
                    Some("run-1"),
                    BusyPolicy::RejectIfBusy,
                    serde_json::json!({"x":1}),
                )
                .unwrap();
            queue
                .accept(
                    "request-2",
                    Some("run-2"),
                    BusyPolicy::EnqueueIfBusy,
                    serde_json::json!({"x":2}),
                )
                .unwrap();
            queue.mark_started("run-1", 9).unwrap();
        }
        let queue = DurableRunQueue::open(&path, "session-a").unwrap();
        assert_eq!(queue.active().unwrap().1, 9);
        assert_eq!(queue.queued()[0].run_id, "run-2");
        let retry = queue
            .accept(
                "request-2",
                Some("run-2"),
                BusyPolicy::EnqueueIfBusy,
                serde_json::json!({"x":2}),
            )
            .unwrap();
        assert_eq!(retry.accepted_state, RunAcceptedState::Existing);
        assert_eq!(retry.run_id, "run-2");
        assert_eq!(retry.queue_position, Some(1));
    }

    #[test]
    fn idempotency_rejects_changed_payload_but_ignores_object_key_order() {
        let dir = tempfile::tempdir().unwrap();
        let queue = queue(&dir);
        queue
            .accept(
                "request-1",
                Some("run-1"),
                BusyPolicy::RejectIfBusy,
                serde_json::json!({"a":1,"b":2}),
            )
            .unwrap();
        queue
            .accept(
                "request-1",
                Some("run-1"),
                BusyPolicy::RejectIfBusy,
                serde_json::json!({"b":2,"a":1}),
            )
            .unwrap();
        let error = queue
            .accept(
                "request-1",
                Some("run-1"),
                BusyPolicy::RejectIfBusy,
                serde_json::json!({"a":2,"b":2}),
            )
            .unwrap_err();
        assert!(matches!(error, QueueError::DuplicateRequestConflict(_)));
    }

    #[test]
    fn ignores_only_a_torn_final_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.jsonl");
        let queue = DurableRunQueue::open(&path, "session-a").unwrap();
        queue
            .accept(
                "request-1",
                Some("run-1"),
                BusyPolicy::RejectIfBusy,
                Value::Null,
            )
            .unwrap();
        drop(queue);
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{\"kind\":\"run_")
            .unwrap();
        let restored = DurableRunQueue::open(&path, "session-a").unwrap();
        assert_eq!(restored.queued().len(), 1);
    }

    #[test]
    fn rejects_corrupt_complete_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.jsonl");
        fs::write(&path, b"not-json\n").unwrap();
        let error = DurableRunQueue::open(&path, "session-a").unwrap_err();
        assert!(matches!(error, QueueError::Corrupt { line: 1, .. }));
    }

    #[test]
    fn queued_cancellation_is_durable_and_does_not_touch_active() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.jsonl");
        let queue = DurableRunQueue::open(&path, "session-a").unwrap();
        for number in 1..=3 {
            queue
                .accept(
                    &format!("request-{number}"),
                    Some(&format!("run-{number}")),
                    BusyPolicy::EnqueueIfBusy,
                    Value::Null,
                )
                .unwrap();
        }
        queue.mark_started("run-1", 7).unwrap();
        queue
            .cancel_queued("run-2", QueuedCancellationReason::Cancelled)
            .unwrap();
        assert_eq!(queue.active().unwrap().0.run_id, "run-1");
        assert_eq!(queue.queued()[0].run_id, "run-3");

        drop(queue);
        let restored = DurableRunQueue::open(&path, "session-a").unwrap();
        assert_eq!(restored.active().unwrap().0.run_id, "run-1");
        assert_eq!(restored.queued()[0].run_id, "run-3");
        let retry = restored
            .accept(
                "request-2",
                Some("run-2"),
                BusyPolicy::EnqueueIfBusy,
                Value::Null,
            )
            .unwrap();
        assert_eq!(retry.accepted_state, RunAcceptedState::Existing);
    }

    #[test]
    fn cancel_all_queued_is_one_replayable_transition() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.jsonl");
        let queue = DurableRunQueue::open(&path, "session-a").unwrap();
        for number in 1..=2 {
            queue
                .accept(
                    &format!("request-{number}"),
                    Some(&format!("run-{number}")),
                    BusyPolicy::EnqueueIfBusy,
                    Value::Null,
                )
                .unwrap();
        }
        let cancelled = queue
            .cancel_all_queued(QueuedCancellationReason::Superseded)
            .unwrap();
        assert_eq!(cancelled.len(), 2);
        assert!(queue.queued().is_empty());
        drop(queue);
        assert!(DurableRunQueue::open(&path, "session-a")
            .unwrap()
            .queued()
            .is_empty());
    }
}
