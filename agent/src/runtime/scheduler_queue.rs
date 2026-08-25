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

impl QueuedCancellationReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::Superseded => "superseded",
            Self::SessionDeleted => "session_deleted",
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TerminalRunAck {
    pub run_id: String,
    pub run_sequence: u64,
    pub client_request_id: String,
    pub state: String,
    pub reason: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RunQueueError {
    #[error("session is being deleted")]
    Deleting,
    #[error("session persistence is unavailable: {0}")]
    PersistenceUnavailable(String),
    #[error("run_id `{0}` contains unsafe characters")]
    InvalidRunId(String),
    #[error("client_request_id must not be empty")]
    MissingClientRequestId,
    #[error("client_request_id `{0}` was already used with a different request")]
    DuplicateRequestConflict(String),
    #[error("run_id `{0}` already exists in this Agent instance")]
    DuplicateRunId(String),
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
    terminal_acks: VecDeque<TerminalRunAck>,
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
    recent_ack_limit: usize,
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
            256,
            std::sync::Arc::new(GlobalQueueBudget::new(usize::MAX, usize::MAX)),
        )
    }

    pub fn with_limits(
        session_id: impl Into<String>,
        next_sequence: u64,
        capacity: usize,
        max_queue_bytes: usize,
        max_request_bytes: usize,
        recent_ack_limit: usize,
    ) -> Self {
        Self::with_limits_and_global(
            session_id,
            next_sequence,
            capacity,
            max_queue_bytes,
            max_request_bytes,
            recent_ack_limit,
            std::sync::Arc::new(GlobalQueueBudget::new(usize::MAX, usize::MAX)),
        )
    }

    pub fn with_limits_and_global(
        session_id: impl Into<String>,
        next_sequence: u64,
        capacity: usize,
        max_queue_bytes: usize,
        max_request_bytes: usize,
        recent_ack_limit: usize,
        global_budget: std::sync::Arc<GlobalQueueBudget>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            capacity,
            max_queue_bytes,
            max_request_bytes,
            global_budget,
            recent_ack_limit,
            state: Mutex::new(QueueState {
                next_sequence: next_sequence.max(1),
                active: None,
                queued: VecDeque::new(),
                queued_bytes: 0,
                requests: HashMap::new(),
                run_ids: HashSet::new(),
                terminal_acks: VecDeque::new(),
            }),
        }
    }

    /// Accept a request into the process-local FIFO. The payload freezes run
    /// configuration and attachment metadata; path-backed attachment contents
    /// remain live and are resolved only when the run consumes them.
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

        // reject_if_busy is gone: a busy session simply enqueues (the default
        // follow-up policy). Supersede is never honored through plain accept;
        // it must go through the atomic `supersede` operation.
        if busy_policy == BusyPolicy::SupersedeSession {
            return Err(RunQueueError::SupersedeRequiresSessionOperation);
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
        if state.next_sequence.checked_add(1).is_none() {
            return Err(RunQueueError::SequenceExhausted);
        }
        self.global_budget.reserve(payload_bytes)?;
        let run_sequence = next_sequence(&mut state).expect("sequence capacity preflighted");
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
        if state.next_sequence.checked_add(1).is_none() {
            return Err(RunQueueError::SequenceExhausted);
        }

        let released_count = state.queued.len();
        let released_bytes = state.queued_bytes;
        self.global_budget
            .replace_queued(released_count, released_bytes, payload_bytes)?;

        let cancelled: Vec<_> = state.queued.drain(..).collect();
        state.queued_bytes = 0;
        for request in &cancelled {
            remember_terminal(
                &mut state,
                request,
                "cancelled",
                "superseded",
                self.recent_ack_limit,
            );
        }
        let run_sequence = next_sequence(&mut state).expect("sequence capacity preflighted");
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
        remember_terminal(
            &mut state,
            &finished,
            "terminal",
            "settled",
            self.recent_ack_limit,
        );
        Ok(())
    }

    pub fn cancel_queued(
        &self,
        run_id: &str,
        reason: QueuedCancellationReason,
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
        remember_terminal(
            &mut state,
            &cancelled,
            "cancelled",
            reason.as_str(),
            self.recent_ack_limit,
        );
        Ok(cancelled)
    }

    pub fn cancel_all_queued(&self, reason: QueuedCancellationReason) -> Vec<ScheduledRunRequest> {
        let mut state = self.state.lock();
        let cancelled: Vec<_> = state.queued.drain(..).collect();
        state.queued_bytes = 0;
        for request in &cancelled {
            self.global_budget.release(request.payload_bytes);
        }
        for request in &cancelled {
            remember_terminal(
                &mut state,
                request,
                "cancelled",
                reason.as_str(),
                self.recent_ack_limit,
            );
        }
        cancelled
    }

    pub fn active(&self) -> Option<(ScheduledRunRequest, u64)> {
        self.state.lock().active.clone()
    }

    pub fn release_active_payload(&self, run_id: &str) {
        let mut state = self.state.lock();
        if let Some((active, _)) = state
            .active
            .as_mut()
            .filter(|(active, _)| active.run_id == run_id)
        {
            active.payload = Value::Null;
            active.payload_bytes = 0;
        }
    }

    pub fn queued(&self) -> Vec<ScheduledRunRequest> {
        self.state.lock().queued.iter().cloned().collect()
    }

    /// Drain every queued run EXCEPT the front one, recording them terminal
    /// with reason "merged". The front run is about to start with the merged
    /// message, so the folded runs must not start separately and their
    /// identities must be released (so a retry returns `existing`). Returns the
    /// folded requests (for the caller to drop their execution snapshots).
    pub fn drain_queued_after_first(&self) -> Vec<ScheduledRunRequest> {
        let mut state = self.state.lock();
        let folded: Vec<ScheduledRunRequest> = state.queued.iter().skip(1).cloned().collect();
        if folded.is_empty() {
            return Vec::new();
        }
        let front = state.queued.pop_front().expect("front queued run");
        state.queued.clear();
        state.queued.push_front(front);
        for request in &folded {
            state.queued_bytes = state.queued_bytes.saturating_sub(request.payload_bytes);
            self.global_budget.release(request.payload_bytes);
            remember_terminal(
                &mut state,
                request,
                "terminal",
                "merged",
                self.recent_ack_limit,
            );
        }
        folded
    }

    /// Test-only seam: push a copy of the ACTIVE request back onto the
    /// queue, forging the active+queued-same-run-id state that `accept`
    /// makes impossible through the public API (`DuplicateRunId`). The
    /// session's dequeue error handling defends against that inconsistency;
    /// this lets tests reach the arm.
    #[cfg(test)]
    pub fn test_requeue_active_duplicate(&self) {
        let mut state = self.state.lock();
        if let Some(active) = state.active.as_ref().map(|(active, _)| active.clone()) {
            state.queued.push_front(active);
        }
    }

    /// Test-only: move the front queued run to the back, so `start_next`
    /// pops a different run than the one `start_next_scheduled` just peeked
    /// — reaching the scheduler FIFO-mismatch defensive arm.
    #[cfg(test)]
    pub fn test_move_front_to_back(&self) {
        let mut state = self.state.lock();
        if let Some(front) = state.queued.pop_front() {
            state.queued.push_back(front);
        }
    }

    pub fn queued_bytes(&self) -> usize {
        self.state.lock().queued_bytes
    }

    pub fn knows_request(&self, client_request_id: &str) -> bool {
        self.state.lock().requests.contains_key(client_request_id)
    }

    pub fn recent_terminal_acks(&self) -> Vec<TerminalRunAck> {
        self.state.lock().terminal_acks.iter().cloned().collect()
    }
}

fn remember_terminal(
    state: &mut QueueState,
    request: &ScheduledRunRequest,
    terminal_state: &str,
    reason: &str,
    limit: usize,
) {
    state.terminal_acks.push_back(TerminalRunAck {
        run_id: request.run_id.clone(),
        run_sequence: request.run_sequence,
        client_request_id: request.client_request_id.clone(),
        state: terminal_state.to_string(),
        reason: reason.to_string(),
    });
    while state.terminal_acks.len() > limit {
        if let Some(evicted) = state.terminal_acks.pop_front() {
            state.requests.remove(&evicted.client_request_id);
            state.run_ids.remove(&evicted.run_id);
        }
    }
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
    fn busy_accept_enqueues_when_a_run_is_pending() {
        // With reject_if_busy removed, the default (enqueue) policy appends a
        // new request behind the already-accepted one instead of rejecting it.
        let queue = queue();
        queue
            .accept(
                "request-1",
                Some("run-1"),
                BusyPolicy::EnqueueIfBusy,
                Value::Null,
            )
            .unwrap();
        let second = queue
            .accept(
                "request-2",
                Some("run-2"),
                BusyPolicy::EnqueueIfBusy,
                Value::Null,
            )
            .unwrap();
        assert_eq!(second.accepted_state, RunAcceptedState::Queued);
        assert_eq!(queue.queued().len(), 2);
    }

    #[test]
    fn drain_queued_after_first_folds_trailing_runs_as_merged() {
        let queue = queue();
        for (req, run) in [
            ("request-1", "run-1"),
            ("request-2", "run-2"),
            ("request-3", "run-3"),
        ] {
            queue
                .accept(req, Some(run), BusyPolicy::EnqueueIfBusy, Value::Null)
                .unwrap();
        }
        assert_eq!(queue.queued().len(), 3);

        let folded = queue.drain_queued_after_first();
        // The front run stays; the two trailing runs are folded.
        assert_eq!(
            folded.iter().map(|r| r.run_id.as_str()).collect::<Vec<_>>(),
            vec!["run-2", "run-3"]
        );
        assert_eq!(queue.queued().len(), 1);
        assert_eq!(queue.queued()[0].run_id, "run-1");
        // The folded runs are recorded terminal with reason "merged".
        let acks = queue.recent_terminal_acks();
        assert_eq!(acks.len(), 2);
        assert!(acks
            .iter()
            .all(|a| a.state == "terminal" && a.reason == "merged"));
        // A single-queued queue drains nothing.
        assert!(queue.drain_queued_after_first().is_empty());
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

    #[test]
    fn terminal_ack_lru_is_bounded_and_releases_old_request_identity() {
        let queue = InMemoryRunQueue::with_limits("s", 1, 8, 1024, 1024, 2);
        for n in 1..=3 {
            queue
                .accept(
                    &format!("request-{n}"),
                    Some(&format!("run-{n}")),
                    BusyPolicy::EnqueueIfBusy,
                    serde_json::json!({"n":n}),
                )
                .unwrap();
            let (run, _) = queue.start_next(n).unwrap();
            queue.finish_active(&run.run_id).unwrap();
        }
        let recent = queue.recent_terminal_acks();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].run_id, "run-2");
        assert!(!queue.knows_request("request-1"));
        assert!(queue.knows_request("request-3"));
    }

    // ─── coverage batch: error arms ────────────────────────────────────────

    #[test]
    fn cancellation_reason_names() {
        assert_eq!(QueuedCancellationReason::Cancelled.as_str(), "cancelled");
        assert_eq!(QueuedCancellationReason::Superseded.as_str(), "superseded");
        assert_eq!(
            QueuedCancellationReason::SessionDeleted.as_str(),
            "session_deleted"
        );
    }

    #[test]
    fn accept_requires_client_request_id() {
        let queue = queue();
        let result = queue.accept("  ", None, BusyPolicy::EnqueueIfBusy, serde_json::json!({}));
        assert!(matches!(
            result.unwrap_err(),
            RunQueueError::MissingClientRequestId
        ));
    }

    #[test]
    fn accept_rejects_supersede_policy_via_plain_accept() {
        let queue = queue();
        let result = queue.accept(
            "r1",
            None,
            BusyPolicy::SupersedeSession,
            serde_json::json!({}),
        );
        assert!(matches!(
            result.unwrap_err(),
            RunQueueError::SupersedeRequiresSessionOperation
        ));
    }

    #[test]
    fn accept_reports_queue_bytes_exceeded() {
        let queue = InMemoryRunQueue::with_limits("s", 1, 128, 60, 64, 256);
        let big = serde_json::json!({"message": "x".repeat(32)});
        let first = queue.accept("r1", None, BusyPolicy::EnqueueIfBusy, big.clone());
        assert!(first.is_ok());
        let second = queue.accept("r2", None, BusyPolicy::EnqueueIfBusy, big);
        assert!(matches!(
            second.unwrap_err(),
            RunQueueError::QueueBytesExceeded { .. }
        ));
    }

    #[test]
    fn accept_reports_duplicate_run_id() {
        let queue = queue();
        queue
            .accept(
                "r1",
                Some("run-x"),
                BusyPolicy::EnqueueIfBusy,
                serde_json::json!({}),
            )
            .unwrap();
        let result = queue.accept(
            "r2",
            Some("run-x"),
            BusyPolicy::EnqueueIfBusy,
            serde_json::json!({}),
        );
        assert!(matches!(
            result.unwrap_err(),
            RunQueueError::DuplicateRunId(id) if id == "run-x"
        ));
    }

    #[test]
    fn start_next_reports_active_and_empty() {
        let queue = queue();
        assert!(matches!(
            queue.start_next(1).unwrap_err(),
            RunQueueError::QueueEmpty
        ));
        queue
            .accept(
                "r1",
                Some("run-1"),
                BusyPolicy::EnqueueIfBusy,
                serde_json::json!({}),
            )
            .unwrap();
        queue.start_next(1).unwrap();
        assert!(matches!(
            queue.start_next(2).unwrap_err(),
            RunQueueError::ActiveRunExists(id) if id == "run-1"
        ));
    }

    #[test]
    fn finish_active_rejects_unknown_or_stale_run() {
        let queue = queue();
        let result = queue.finish_active("run-ghost");
        assert!(matches!(
            result.unwrap_err(),
            RunQueueError::RunNotActive { expected: None, actual } if actual == "run-ghost"
        ));
    }

    #[test]
    fn supersede_validates_request_and_run_ids() {
        let queue = queue();
        // Empty request id.
        let result = queue.supersede(" ", None, serde_json::json!({}));
        assert!(matches!(
            result.unwrap_err(),
            RunQueueError::MissingClientRequestId
        ));
        // Unsafe requested run id.
        let result = queue.supersede("r1", Some("bad id!"), serde_json::json!({}));
        assert!(matches!(
            result.unwrap_err(),
            RunQueueError::InvalidRunId(_)
        ));
        // Conflicting reuse of a client_request_id.
        queue
            .supersede("r1", None, serde_json::json!({"message": "one"}))
            .unwrap();
        let result = queue.supersede("r1", None, serde_json::json!({"message": "two"}));
        assert!(matches!(
            result.unwrap_err(),
            RunQueueError::DuplicateRequestConflict(_)
        ));
        // Identical reuse returns the existing ack.
        let (ack, cancelled, _) = queue
            .supersede("r1", None, serde_json::json!({"message": "one"}))
            .unwrap();
        assert!(cancelled.is_empty());
        assert_eq!(ack.accepted_state, RunAcceptedState::Existing);
        // Duplicate requested run id across different requests.
        queue
            .supersede("r2", Some("run-dup"), serde_json::json!({}))
            .unwrap();
        let result = queue.supersede("r3", Some("run-dup"), serde_json::json!({}));
        assert!(matches!(
            result.unwrap_err(),
            RunQueueError::DuplicateRunId(_)
        ));
    }

    #[test]
    fn supersede_reports_oversized_request() {
        let queue = InMemoryRunQueue::with_limits("s", 1, 128, 1024, 8, 256);
        let big = serde_json::json!({"message": "x".repeat(64)});
        let result = queue.supersede("r1", None, big);
        assert!(matches!(
            result.unwrap_err(),
            RunQueueError::RequestTooLarge { .. }
        ));
    }

    #[test]
    fn supersede_respects_global_budget_on_replace() {
        let budget = std::sync::Arc::new(GlobalQueueBudget::new(1, usize::MAX));
        let queue =
            InMemoryRunQueue::with_limits_and_global("s", 1, 128, 65536, 65536, 256, budget);
        queue
            .accept(
                "r1",
                None,
                BusyPolicy::EnqueueIfBusy,
                serde_json::json!({"m": "a"}),
            )
            .unwrap();
        // The global budget is exhausted by the queued run; supersede's
        // replace path must still work (it releases first) — and the count
        // arm fires when the replacement would exceed the count limit.
        let result = queue.supersede("r2", None, serde_json::json!({"m": "b"}));
        assert!(result.is_ok());
    }

    #[test]
    fn existing_ack_for_queued_request_reports_position() {
        let queue = queue();
        // An active run makes subsequent accepts queue with positions.
        queue
            .accept(
                "r1",
                Some("run-1"),
                BusyPolicy::EnqueueIfBusy,
                serde_json::json!({}),
            )
            .unwrap();
        queue.start_next(1).unwrap();
        queue
            .accept(
                "r2",
                Some("run-2"),
                BusyPolicy::EnqueueIfBusy,
                serde_json::json!({"m": 1}),
            )
            .unwrap();
        // Same payload + request id → the existing ack carries the position.
        let ack = queue
            .accept(
                "r2",
                Some("run-2"),
                BusyPolicy::EnqueueIfBusy,
                serde_json::json!({"m": 1}),
            )
            .unwrap();
        assert_eq!(ack.accepted_state, RunAcceptedState::Existing);
        assert!(ack.queue_position.is_some());
    }

    #[test]
    fn replace_queued_respects_global_count_and_byte_limits() {
        // Count limit: base_count (after subtracting the releases) is full.
        let budget = GlobalQueueBudget::new(1, 1_000);
        budget.reserve(10).unwrap();
        let err = budget.replace_queued(0, 0, 10).unwrap_err();
        assert_eq!(err, RunQueueError::GlobalQueueFull { limit: 1 });

        // Byte limit: count headroom remains but the projection overflows.
        let budget = GlobalQueueBudget::new(10, 100);
        budget.reserve(60).unwrap();
        let err = budget.replace_queued(0, 0, 50).unwrap_err();
        assert_eq!(
            err,
            RunQueueError::GlobalQueueBytesExceeded {
                actual: 110,
                limit: 100
            }
        );

        // Releases are subtracted before the comparison.
        let budget = GlobalQueueBudget::new(2, 100);
        budget.reserve(60).unwrap();
        budget.reserve(30).unwrap();
        budget.replace_queued(1, 30, 40).unwrap();
    }

    #[test]
    fn accept_reports_sequence_exhausted() {
        let queue = queue();
        queue.state.lock().next_sequence = u64::MAX;
        let err = queue
            .accept(
                "request-1",
                Some("run-1"),
                BusyPolicy::EnqueueIfBusy,
                Value::Null,
            )
            .unwrap_err();
        assert_eq!(err, RunQueueError::SequenceExhausted);
    }

    #[test]
    fn supersede_reports_sequence_exhausted() {
        let queue = queue();
        queue.state.lock().next_sequence = u64::MAX;
        let err = queue
            .supersede("request-1", Some("run-1"), Value::Null)
            .unwrap_err();
        assert_eq!(err, RunQueueError::SequenceExhausted);
    }

    #[test]
    fn retry_of_terminal_run_returns_plain_existing_ack() {
        // After the run goes terminal, the request identity is remembered but
        // the run is neither active nor queued — the fallback existing-ack.
        let queue = queue();
        queue
            .accept(
                "request-1",
                Some("run-1"),
                BusyPolicy::EnqueueIfBusy,
                serde_json::json!({"text": "a"}),
            )
            .unwrap();
        queue.start_next(7).unwrap();
        queue.finish_active("run-1").unwrap();

        let ack = queue
            .accept(
                "request-1",
                Some("run-1"),
                BusyPolicy::EnqueueIfBusy,
                serde_json::json!({"text": "a"}),
            )
            .unwrap();
        assert_eq!(ack.accepted_state, RunAcceptedState::Existing);
        assert_eq!(ack.run_sequence, Some(1));
        assert!(ack.queue_position.is_none());
    }
}
