use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use parking_lot::Mutex;
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use super::{BusyPolicy, RunAck};

pub const DEFAULT_SESSION_QUEUE_CAPACITY: usize = 128;
pub const DEFAULT_SESSION_QUEUE_BYTES: usize = 64 * 1024 * 1024;
pub const DEFAULT_REQUEST_BYTES: usize = 16 * 1024 * 1024;
pub const DEFAULT_GLOBAL_QUEUE_CAPACITY: usize = 4_096;
pub const DEFAULT_GLOBAL_QUEUE_BYTES: usize = 512 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledRunRequest {
    pub session_id: String,
    pub run_id: String,
    pub run_sequence: u64,
    pub client_request_id: String,
    pub request_digest: String,
    pub busy_policy: BusyPolicy,
    pub payload: Value,
    pub accepted_at: String,
    pub payload_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueuedCancellationReason {
    Cancelled,
    Superseded,
    SessionDeleted,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RunQueueError {
    #[error("session is being deleted")]
    Deleting,
    #[error("session persistence is unavailable: {0}")]
    PersistenceUnavailable(String),
    #[error("attachment `{path}` is unavailable: {reason}")]
    AttachmentUnavailable { path: String, reason: String },
    #[error("run_id `{0}` contains unsafe characters")]
    InvalidRunId(String),
    #[error("client_request_id must not be empty")]
    MissingClientRequestId,
    #[error("client_request_id `{0}` was already used with a different request")]
    DuplicateRequestConflict(String),
    #[error("run_id `{0}` already exists in this Agent instance")]
    DuplicateRunId(String),
    #[error("session already has pending work")]
    Busy,
    #[error("supersede_session requires the atomic session scheduler operation")]
    SupersedeRequiresSessionOperation,
    #[error("session queue is full (limit {limit})")]
    QueueFull { limit: usize },
    #[error("request snapshot is too large ({actual} bytes; limit {limit})")]
    RequestTooLarge { actual: usize, limit: usize },
    #[error("session queue memory limit exceeded ({actual} bytes; limit {limit})")]
    QueueBytesExceeded { actual: usize, limit: usize },
    #[error("global queue is full (limit {limit})")]
    GlobalQueueFull { limit: usize },
    #[error("global queue memory limit exceeded ({actual} bytes; limit {limit})")]
    GlobalQueueBytesExceeded { actual: usize, limit: usize },
    #[error("session already has active run `{0}`")]
    ActiveRunExists(String),
    #[error("there is no queued run to start")]
    QueueEmpty,
    #[error("run `{0}` is not queued")]
    RunNotQueued(String),
    #[error("run `{actual}` cannot become terminal; active run is {expected:?}")]
    RunNotActive {
        expected: Option<String>,
        actual: String,
    },
    #[error("run sequence space is exhausted for this Agent instance")]
    SequenceExhausted,
}

#[derive(Debug, Clone)]
struct RequestIdentity {
    digest: String,
    run_id: String,
    run_sequence: u64,
}

#[derive(Debug)]
struct QueueState {
    next_sequence: u64,
    active: Option<(ScheduledRunRequest, u64)>,
    queued: VecDeque<ScheduledRunRequest>,
    queued_bytes: usize,
    requests: HashMap<String, RequestIdentity>,
    run_ids: HashSet<String>,
}

#[derive(Debug, Default)]
struct GlobalBudgetState {
    count: usize,
    bytes: usize,
}

#[derive(Debug)]
pub struct GlobalQueueBudget {
    max_count: usize,
    max_bytes: usize,
    state: Mutex<GlobalBudgetState>,
}

impl GlobalQueueBudget {
    pub fn new(max_count: usize, max_bytes: usize) -> Self {
        Self {
            max_count,
            max_bytes,
            state: Mutex::new(GlobalBudgetState::default()),
        }
    }

    pub fn defaults() -> Self {
        Self::new(DEFAULT_GLOBAL_QUEUE_CAPACITY, DEFAULT_GLOBAL_QUEUE_BYTES)
    }

    fn reserve(&self, bytes: usize) -> Result<(), RunQueueError> {
        let mut state = self.state.lock();
        if state.count >= self.max_count {
            return Err(RunQueueError::GlobalQueueFull {
                limit: self.max_count,
            });
        }
        let new_bytes = state.bytes.saturating_add(bytes);
        if new_bytes > self.max_bytes {
            return Err(RunQueueError::GlobalQueueBytesExceeded {
                actual: new_bytes,
                limit: self.max_bytes,
            });
        }
        state.count += 1;
        state.bytes = new_bytes;
        Ok(())
    }

    fn release(&self, bytes: usize) {
        let mut state = self.state.lock();
        state.count = state.count.saturating_sub(1);
        state.bytes = state.bytes.saturating_sub(bytes);
    }

    fn replace_queued(
        &self,
        released_count: usize,
        released_bytes: usize,
        new_bytes: usize,
    ) -> Result<(), RunQueueError> {
        let mut state = self.state.lock();
        let base_count = state.count.saturating_sub(released_count);
        let base_bytes = state.bytes.saturating_sub(released_bytes);
        if base_count >= self.max_count {
            return Err(RunQueueError::GlobalQueueFull {
                limit: self.max_count,
            });
        }
        let projected_bytes = base_bytes.saturating_add(new_bytes);
        if projected_bytes > self.max_bytes {
            return Err(RunQueueError::GlobalQueueBytesExceeded {
                actual: projected_bytes,
                limit: self.max_bytes,
            });
        }
        state.count = base_count + 1;
        state.bytes = projected_bytes;
        Ok(())
    }

    pub fn usage(&self) -> (usize, usize) {
        let state = self.state.lock();
        (state.count, state.bytes)
    }
}

/// Process-local FIFO for one session.
///
/// This deliberately has no persistence hooks. GUI/TUI reconnects to the same
/// Agent instance can query it; an Agent restart drops queued work and changes
/// the process `agent_instance_id` exposed by the control plane.
#[derive(Debug)]
pub struct InMemoryRunQueue {
    session_id: String,
    capacity: usize,
    max_queue_bytes: usize,
    max_request_bytes: usize,
    global_budget: std::sync::Arc<GlobalQueueBudget>,
    state: Mutex<QueueState>,
}

impl InMemoryRunQueue {
    pub fn new(session_id: impl Into<String>, next_sequence: u64) -> Self {
        Self::with_limits_and_global(
            session_id,
            next_sequence,
            DEFAULT_SESSION_QUEUE_CAPACITY,
            DEFAULT_SESSION_QUEUE_BYTES,
            DEFAULT_REQUEST_BYTES,
            0,
            std::sync::Arc::new(GlobalQueueBudget::new(usize::MAX, usize::MAX)),
        )
    }

    pub fn with_limits(
        session_id: impl Into<String>,
        next_sequence: u64,
        capacity: usize,
        max_queue_bytes: usize,
        max_request_bytes: usize,
        _recent_ack_limit: usize,
    ) -> Self {
        Self::with_limits_and_global(
            session_id,
            next_sequence,
            capacity,
            max_queue_bytes,
            max_request_bytes,
            _recent_ack_limit,
            std::sync::Arc::new(GlobalQueueBudget::new(usize::MAX, usize::MAX)),
        )
    }

    pub fn with_limits_and_global(
        session_id: impl Into<String>,
        next_sequence: u64,
        capacity: usize,
        max_queue_bytes: usize,
        max_request_bytes: usize,
        _recent_ack_limit: usize,
        global_budget: std::sync::Arc<GlobalQueueBudget>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            capacity,
            max_queue_bytes,
            max_request_bytes,
            global_budget,
            state: Mutex::new(QueueState {
                next_sequence: next_sequence.max(1),
                active: None,
                queued: VecDeque::new(),
                queued_bytes: 0,
                requests: HashMap::new(),
                run_ids: HashSet::new(),
            }),
        }
    }

    /// Accept a request into the process-local FIFO. The payload must already
    /// contain an immutable snapshot of any path-backed attachment.
    pub fn accept(
        &self,
        client_request_id: &str,
        requested_run_id: Option<&str>,
        busy_policy: BusyPolicy,
        payload: Value,
    ) -> Result<RunAck, RunQueueError> {
        if client_request_id.trim().is_empty() {
            return Err(RunQueueError::MissingClientRequestId);
        }
        let digest = request_digest(requested_run_id, busy_policy, &payload);
        let payload_bytes = serde_json::to_vec(&payload)
            .expect("JSON value serialization cannot fail")
            .len();
        let mut state = self.state.lock();

        if let Some(identity) = state.requests.get(client_request_id) {
            if identity.digest != digest {
                return Err(RunQueueError::DuplicateRequestConflict(
                    client_request_id.to_string(),
                ));
            }
            return Ok(existing_ack(&state, identity));
        }

        let busy = state.active.is_some() || !state.queued.is_empty();
        match (busy, busy_policy) {
            (true, BusyPolicy::RejectIfBusy) => return Err(RunQueueError::Busy),
            (_, BusyPolicy::SupersedeSession) => {
                return Err(RunQueueError::SupersedeRequiresSessionOperation)
            }
            _ => {}
        }
        if payload_bytes > self.max_request_bytes {
            return Err(RunQueueError::RequestTooLarge {
                actual: payload_bytes,
                limit: self.max_request_bytes,
            });
        }
        if state.queued.len() >= self.capacity {
            return Err(RunQueueError::QueueFull {
                limit: self.capacity,
            });
        }
        let new_bytes = state.queued_bytes.saturating_add(payload_bytes);
        if new_bytes > self.max_queue_bytes {
            return Err(RunQueueError::QueueBytesExceeded {
                actual: new_bytes,
                limit: self.max_queue_bytes,
            });
        }
        let run_id = requested_run_id
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("run_{}", Uuid::new_v4().simple()));
        if run_id.len() > 128
            || run_id.is_empty()
            || !run_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(RunQueueError::InvalidRunId(run_id));
        }
        if state.run_ids.contains(&run_id) {
            return Err(RunQueueError::DuplicateRunId(run_id));
        }
        self.global_budget.reserve(payload_bytes)?;
        let run_sequence = next_sequence(&mut state)?;
        let request = ScheduledRunRequest {
            session_id: self.session_id.clone(),
            run_id: run_id.clone(),
            run_sequence,
            client_request_id: client_request_id.to_string(),
            request_digest: digest.clone(),
            busy_policy,
            payload,
            accepted_at: chrono::Utc::now().to_rfc3339(),
            payload_bytes,
        };
        state.requests.insert(
            client_request_id.to_string(),
            RequestIdentity {
                digest,
                run_id: run_id.clone(),
                run_sequence,
            },
        );
        state.run_ids.insert(run_id.clone());
        state.queued_bytes = new_bytes;
        state.queued.push_back(request);

        Ok(RunAck::queued(
            run_id,
            run_sequence,
            state.queued.len() as u64,
        ))
    }

    pub fn start_next(&self, epoch: u64) -> Result<(ScheduledRunRequest, RunAck), RunQueueError> {
        let mut state = self.state.lock();
        if let Some((active, _)) = &state.active {
            return Err(RunQueueError::ActiveRunExists(active.run_id.clone()));
        }
        let request = state.queued.pop_front().ok_or(RunQueueError::QueueEmpty)?;
        state.queued_bytes = state.queued_bytes.saturating_sub(request.payload_bytes);
        self.global_budget.release(request.payload_bytes);
        let ack = RunAck {
            run_id: request.run_id.clone(),
            run_epoch: epoch,
            accepted_state: super::RunAcceptedState::Running,
            run_sequence: Some(request.run_sequence),
            queue_position: None,
        };
        state.active = Some((request.clone(), epoch));
        Ok((request, ack))
    }

    /// Atomically replace every queued request with one successor while
    /// leaving the current active request owned until cooperative abort
    /// completes. Validation and quota projection happen before mutation.
    pub fn supersede(
        &self,
        client_request_id: &str,
        requested_run_id: Option<&str>,
        payload: Value,
    ) -> Result<(RunAck, Vec<ScheduledRunRequest>, Option<String>), RunQueueError> {
        if client_request_id.trim().is_empty() {
            return Err(RunQueueError::MissingClientRequestId);
        }
        let policy = BusyPolicy::SupersedeSession;
        let digest = request_digest(requested_run_id, policy, &payload);
        let payload_bytes = serde_json::to_vec(&payload)
            .expect("JSON value serialization cannot fail")
            .len();
        let mut state = self.state.lock();
        if let Some(identity) = state.requests.get(client_request_id) {
            if identity.digest != digest {
                return Err(RunQueueError::DuplicateRequestConflict(
                    client_request_id.to_string(),
                ));
            }
            return Ok((existing_ack(&state, identity), Vec::new(), None));
        }
        if payload_bytes > self.max_request_bytes || payload_bytes > self.max_queue_bytes {
            return Err(RunQueueError::RequestTooLarge {
                actual: payload_bytes,
                limit: self.max_request_bytes.min(self.max_queue_bytes),
            });
        }
        let run_id = requested_run_id
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("run_{}", Uuid::new_v4().simple()));
        if run_id.len() > 128
            || run_id.is_empty()
            || !run_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(RunQueueError::InvalidRunId(run_id));
        }
        if state.run_ids.contains(&run_id) {
            return Err(RunQueueError::DuplicateRunId(run_id));
        }

        let released_count = state.queued.len();
        let released_bytes = state.queued_bytes;
        self.global_budget
            .replace_queued(released_count, released_bytes, payload_bytes)?;

        let cancelled: Vec<_> = state.queued.drain(..).collect();
        state.queued_bytes = 0;
        for request in &cancelled {
            remember_terminal(&mut state, &request.client_request_id);
        }
        let run_sequence = next_sequence(&mut state)?;
        let request = ScheduledRunRequest {
            session_id: self.session_id.clone(),
            run_id: run_id.clone(),
            run_sequence,
            client_request_id: client_request_id.to_string(),
            request_digest: digest.clone(),
            busy_policy: policy,
            payload,
            accepted_at: chrono::Utc::now().to_rfc3339(),
            payload_bytes,
        };
        state.requests.insert(
            client_request_id.to_string(),
            RequestIdentity {
                digest,
                run_id: run_id.clone(),
                run_sequence,
            },
        );
        state.run_ids.insert(run_id.clone());
        state.queued_bytes = payload_bytes;
        state.queued.push_back(request);
        let active_run_id = state.active.as_ref().map(|(run, _)| run.run_id.clone());
        Ok((
            RunAck::queued(run_id, run_sequence, 1),
            cancelled,
            active_run_id,
        ))
    }

    pub fn finish_active(&self, run_id: &str) -> Result<(), RunQueueError> {
        let mut state = self.state.lock();
        let active_id = state.active.as_ref().map(|(run, _)| run.run_id.clone());
        if active_id.as_deref() != Some(run_id) {
            return Err(RunQueueError::RunNotActive {
                expected: active_id,
                actual: run_id.to_string(),
            });
        }
        let (finished, _) = state.active.take().expect("active run checked");
        remember_terminal(&mut state, &finished.client_request_id);
        Ok(())
    }

    pub fn cancel_queued(
        &self,
        run_id: &str,
        _reason: QueuedCancellationReason,
    ) -> Result<ScheduledRunRequest, RunQueueError> {
        let mut state = self.state.lock();
        let Some(index) = state.queued.iter().position(|run| run.run_id == run_id) else {
            return Err(RunQueueError::RunNotQueued(run_id.to_string()));
        };
        let cancelled = state
            .queued
            .remove(index)
            .expect("queued run index checked");
        state.queued_bytes = state.queued_bytes.saturating_sub(cancelled.payload_bytes);
        self.global_budget.release(cancelled.payload_bytes);
        remember_terminal(&mut state, &cancelled.client_request_id);
        Ok(cancelled)
    }

    pub fn cancel_all_queued(&self, _reason: QueuedCancellationReason) -> Vec<ScheduledRunRequest> {
        let mut state = self.state.lock();
        let cancelled: Vec<_> = state.queued.drain(..).collect();
        state.queued_bytes = 0;
        for request in &cancelled {
            self.global_budget.release(request.payload_bytes);
        }
        for request in &cancelled {
            remember_terminal(&mut state, &request.client_request_id);
        }
        cancelled
    }

    pub fn active(&self) -> Option<(ScheduledRunRequest, u64)> {
        self.state.lock().active.clone()
    }

    pub fn queued(&self) -> Vec<ScheduledRunRequest> {
        self.state.lock().queued.iter().cloned().collect()
    }

    pub fn queued_bytes(&self) -> usize {
        self.state.lock().queued_bytes
    }
}

fn remember_terminal(_state: &mut QueueState, _client_request_id: &str) {
    // Request identities intentionally live for the Agent process lifetime.
    // Retrying an ambiguous RPC after a run settles must return its original
    // run id, never schedule a second turn. Queue contents remain restart-
    // volatile by product decision; this is only an in-process guarantee.
}

fn next_sequence(state: &mut QueueState) -> Result<u64, RunQueueError> {
    let sequence = state.next_sequence;
    state.next_sequence = sequence
        .checked_add(1)
        .ok_or(RunQueueError::SequenceExhausted)?;
    Ok(sequence)
}

fn existing_ack(state: &QueueState, identity: &RequestIdentity) -> RunAck {
    if let Some((run, epoch)) = &state.active {
        if run.run_id == identity.run_id {
            let mut ack = RunAck::existing(identity.run_id.clone(), *epoch);
            ack.run_sequence = Some(identity.run_sequence);
            return ack;
        }
    }
    if let Some((index, _)) = state
        .queued
        .iter()
        .enumerate()
        .find(|(_, run)| run.run_id == identity.run_id)
    {
        let mut ack = RunAck::existing(identity.run_id.clone(), 0);
        ack.run_sequence = Some(identity.run_sequence);
        ack.queue_position = Some(index as u64 + 1);
        return ack;
    }
    let mut ack = RunAck::existing(identity.run_id.clone(), 0);
    ack.run_sequence = Some(identity.run_sequence);
    ack
}

fn request_digest(
    requested_run_id: Option<&str>,
    busy_policy: BusyPolicy,
    payload: &Value,
) -> String {
    let envelope = serde_json::json!({
        "busy_policy": busy_policy.as_str(),
        "payload": canonicalize(payload),
        "requested_run_id": requested_run_id,
    });
    let bytes = serde_json::to_vec(&envelope).expect("JSON value serialization cannot fail");
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
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
    use super::*;
    use crate::runtime::RunAcceptedState;

    fn queue() -> InMemoryRunQueue {
        InMemoryRunQueue::new("session-a", 1)
    }

    #[test]
    fn assigns_monotonic_sequence_and_starts_fifo() {
        let queue = queue();
        let first = queue
            .accept(
                "request-1",
                Some("run-1"),
                BusyPolicy::EnqueueIfBusy,
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
        assert_eq!(second.run_sequence, Some(2));
        let (started, ack) = queue.start_next(7).unwrap();
        assert_eq!(started.run_id, "run-1");
        assert_eq!(ack.accepted_state, RunAcceptedState::Running);
        queue.finish_active("run-1").unwrap();
        assert_eq!(queue.start_next(8).unwrap().0.run_id, "run-2");
    }

    #[test]
    fn reject_if_busy_never_mutates_queue() {
        let queue = queue();
        queue
            .accept(
                "request-1",
                Some("run-1"),
                BusyPolicy::EnqueueIfBusy,
                Value::Null,
            )
            .unwrap();
        let error = queue
            .accept(
                "request-2",
                Some("run-2"),
                BusyPolicy::RejectIfBusy,
                Value::Null,
            )
            .unwrap_err();
        assert_eq!(error, RunQueueError::Busy);
        assert_eq!(queue.queued().len(), 1);
    }

    #[test]
    fn retry_returns_existing_and_changed_payload_conflicts() {
        let queue = queue();
        queue
            .accept(
                "request-1",
                Some("run-1"),
                BusyPolicy::EnqueueIfBusy,
                serde_json::json!({"a":1,"b":2}),
            )
            .unwrap();
        let retry = queue
            .accept(
                "request-1",
                Some("run-1"),
                BusyPolicy::EnqueueIfBusy,
                serde_json::json!({"b":2,"a":1}),
            )
            .unwrap();
        assert_eq!(retry.accepted_state, RunAcceptedState::Existing);
        let error = queue
            .accept(
                "request-1",
                Some("run-1"),
                BusyPolicy::EnqueueIfBusy,
                serde_json::json!({"a":2,"b":2}),
            )
            .unwrap_err();
        assert!(matches!(error, RunQueueError::DuplicateRequestConflict(_)));
    }

    #[test]
    fn queued_cancel_releases_memory_and_keeps_other_order() {
        let queue = queue();
        for number in 1..=3 {
            queue
                .accept(
                    &format!("request-{number}"),
                    Some(&format!("run-{number}")),
                    BusyPolicy::EnqueueIfBusy,
                    serde_json::json!({"text":"payload"}),
                )
                .unwrap();
        }
        let before = queue.queued_bytes();
        queue
            .cancel_queued("run-2", QueuedCancellationReason::Cancelled)
            .unwrap();
        assert!(queue.queued_bytes() < before);
        assert_eq!(
            queue
                .queued()
                .iter()
                .map(|run| run.run_id.as_str())
                .collect::<Vec<_>>(),
            vec!["run-1", "run-3"]
        );
    }

    #[test]
    fn count_and_byte_limits_fail_without_partial_acceptance() {
        let queue = InMemoryRunQueue::with_limits("session-a", 1, 1, 20, 16, 2);
        let error = queue
            .accept(
                "large",
                Some("run-large"),
                BusyPolicy::EnqueueIfBusy,
                Value::String("0123456789abcdef".into()),
            )
            .unwrap_err();
        assert!(matches!(error, RunQueueError::RequestTooLarge { .. }));
        assert!(queue.queued().is_empty());

        queue
            .accept(
                "request-1",
                Some("run-1"),
                BusyPolicy::EnqueueIfBusy,
                Value::Null,
            )
            .unwrap();
        let error = queue
            .accept(
                "request-2",
                Some("run-2"),
                BusyPolicy::EnqueueIfBusy,
                Value::Null,
            )
            .unwrap_err();
        assert!(matches!(error, RunQueueError::QueueFull { limit: 1 }));
    }

    #[test]
    fn new_agent_instance_has_no_queued_state_by_construction() {
        let old = queue();
        old.accept(
            "request-1",
            Some("run-1"),
            BusyPolicy::EnqueueIfBusy,
            Value::Null,
        )
        .unwrap();
        let restarted = InMemoryRunQueue::new("session-a", 1);
        assert_eq!(old.queued().len(), 1);
        assert!(restarted.queued().is_empty());
    }

    #[test]
    fn supersede_atomically_replaces_queued_and_preserves_active() {
        let queue = queue();
        queue
            .accept(
                "request-1",
                Some("run-1"),
                BusyPolicy::EnqueueIfBusy,
                Value::Null,
            )
            .unwrap();
        queue.start_next(1).unwrap();
        for number in 2..=3 {
            queue
                .accept(
                    &format!("request-{number}"),
                    Some(&format!("run-{number}")),
                    BusyPolicy::EnqueueIfBusy,
                    Value::Null,
                )
                .unwrap();
        }
        let (ack, cancelled, active) = queue
            .supersede(
                "request-4",
                Some("run-4"),
                serde_json::json!({"text":"latest"}),
            )
            .unwrap();
        assert_eq!(active.as_deref(), Some("run-1"));
        assert_eq!(
            cancelled
                .iter()
                .map(|run| run.run_id.as_str())
                .collect::<Vec<_>>(),
            vec!["run-2", "run-3"]
        );
        assert_eq!(
            queue
                .queued()
                .iter()
                .map(|run| run.run_id.as_str())
                .collect::<Vec<_>>(),
            vec!["run-4"]
        );
        assert_eq!(ack.run_sequence, Some(4));

        let retry = queue
            .supersede(
                "request-4",
                Some("run-4"),
                serde_json::json!({"text":"latest"}),
            )
            .unwrap();
        assert_eq!(retry.0.accepted_state, RunAcceptedState::Existing);
        assert!(retry.1.is_empty());
        assert_eq!(queue.queued().len(), 1);
    }

    #[test]
    fn global_budget_is_shared_and_released() {
        let global = std::sync::Arc::new(GlobalQueueBudget::new(1, 1_024));
        let first =
            InMemoryRunQueue::with_limits_and_global("s1", 1, 8, 1_024, 1_024, 8, global.clone());
        let second =
            InMemoryRunQueue::with_limits_and_global("s2", 1, 8, 1_024, 1_024, 8, global.clone());
        first
            .accept("a", Some("run-a"), BusyPolicy::EnqueueIfBusy, Value::Null)
            .unwrap();
        assert!(matches!(
            second.accept("b", Some("run-b"), BusyPolicy::EnqueueIfBusy, Value::Null),
            Err(RunQueueError::GlobalQueueFull { limit: 1 })
        ));
        first.cancel_all_queued(QueuedCancellationReason::Cancelled);
        second
            .accept("b", Some("run-b"), BusyPolicy::EnqueueIfBusy, Value::Null)
            .unwrap();
        assert_eq!(global.usage().0, 1);
    }

    #[test]
    fn randomized_scheduler_preserves_single_active_unique_ids_and_order() {
        use rand::{Rng, SeedableRng};
        let queue = queue();
        let mut rng = rand::rngs::StdRng::seed_from_u64(0x51_7e_55);
        let mut next = 0_u64;
        let mut last_started_sequence = 0_u64;
        for _ in 0..10_000 {
            match rng.gen_range(0..4) {
                0 | 1 => {
                    next += 1;
                    let _ = queue.accept(
                        &format!("request-{next}"),
                        Some(&format!("run-{next}")),
                        BusyPolicy::EnqueueIfBusy,
                        serde_json::json!({"n": next}),
                    );
                }
                2 if queue.active().is_none() && !queue.queued().is_empty() => {
                    let (started, _) = queue.start_next(next + 1).unwrap();
                    assert!(started.run_sequence > last_started_sequence);
                    last_started_sequence = started.run_sequence;
                }
                3 if queue.active().is_some() => {
                    let active = queue.active().unwrap().0.run_id;
                    queue.finish_active(&active).unwrap();
                }
                _ if !queue.queued().is_empty() => {
                    let run_id = queue.queued().last().unwrap().run_id.clone();
                    queue
                        .cancel_queued(&run_id, QueuedCancellationReason::Cancelled)
                        .unwrap();
                }
                _ => {}
            }
            let queued = queue.queued();
            assert!(queued
                .windows(2)
                .all(|pair| pair[0].run_sequence < pair[1].run_sequence));
            if let Some((active, _)) = queue.active() {
                assert!(queued.iter().all(|run| run.run_id != active.run_id));
            }
        }
    }
}
