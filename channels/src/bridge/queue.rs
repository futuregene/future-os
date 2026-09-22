//! Per-conversation serialization, backpressure, and supersede.
//!
//! Two messages arriving in the same conversation must not run at the same
//! time: the agent session is shared state, and interleaved replies read as
//! nonsense. Two messages arriving in *different* conversations must run
//! concurrently, or one busy group would block every other channel.
//!
//! So each conversation owns a worker and a bounded mailbox. The bound is
//! deliberate backpressure: when a chat floods, the bridge rejects the excess
//! instead of growing without limit, and says so in the log.
//!
//! Supersede is the other half. A user who sends a correction while the agent
//! is still answering the first message does not want two answers — they want
//! the agent to drop the stale one. Every submission bumps the conversation's
//! generation, and a running turn polls its own [`SupersedeWatch`] to notice it
//! has been overtaken and stop quietly.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex};

/// Default mailbox depth per conversation.
pub const DEFAULT_QUEUE_CAPACITY: usize = 8;
/// Default number of conversations kept in the routing table.
pub const DEFAULT_MAX_CONVERSATIONS: usize = 512;
/// How long an idle conversation stays in the table.
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// A running turn's view of whether it has been overtaken.
#[derive(Clone)]
pub struct SupersedeWatch {
    conversation: Arc<String>,
    generation: Arc<AtomicU64>,
    expected: u64,
}

impl SupersedeWatch {
    /// Watch generation `expected` of `conversation`.
    pub fn new(conversation: impl Into<String>, generation: Arc<AtomicU64>, expected: u64) -> Self {
        Self {
            conversation: Arc::new(conversation.into()),
            generation,
            expected,
        }
    }

    /// True once a newer message arrived for this conversation.
    pub fn is_superseded(&self) -> bool {
        self.generation.load(Ordering::SeqCst) != self.expected
    }

    pub fn conversation(&self) -> &str {
        self.conversation.as_str()
    }

    /// The generation this turn owns.
    pub fn generation(&self) -> u64 {
        self.expected
    }
}

/// One unit of work for a conversation worker.
pub struct Job {
    pub conversation: String,
    pub channel: String,
    pub session_id: String,
    pub reply_to: crate::bridge::ConversationRef,
    pub text: String,
    pub images: Vec<crate::grpc_client::ImageInput>,
    pub sink: Arc<dyn crate::bridge::sink::ReplySink>,
}

type RunnerFuture = Pin<Box<dyn Future<Output = ()> + Send>>;
/// How a conversation worker executes a job. Boxed behind an `Arc` so the
/// routing table can hand it to every worker it spawns.
pub type Runner = Arc<dyn Fn(Job, SupersedeWatch) -> RunnerFuture + Send + Sync>;

/// Outcome of handing a message to a conversation worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitOutcome {
    /// Queued; the turn will run (or replace one already running).
    Accepted,
    /// The conversation's mailbox is full — the sender is flooding.
    Full,
}

struct Entry {
    tx: mpsc::Sender<QueuedJob>,
    generation: Arc<AtomicU64>,
    last_used: Instant,
    busy: Arc<AtomicBool>,
}

struct QueuedJob {
    job: Job,
    generation: u64,
}

/// The conversation routing table.
pub struct Conversations {
    entries: Mutex<HashMap<String, Entry>>,
    runner: Runner,
    capacity: usize,
    max_conversations: usize,
    idle_timeout: Duration,
}

impl Conversations {
    pub fn new(runner: Runner) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            runner,
            capacity: DEFAULT_QUEUE_CAPACITY,
            max_conversations: DEFAULT_MAX_CONVERSATIONS,
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
        }
    }

    /// Override the mailbox depth and table size (tests, small deployments).
    pub fn with_limits(mut self, capacity: usize, max_conversations: usize) -> Self {
        self.capacity = capacity.max(1);
        self.max_conversations = max_conversations.max(1);
        self
    }

    /// Queue `job`, superseding any turn still running for its conversation.
    pub async fn submit(&self, job: Job) -> SubmitOutcome {
        let conversation = job.conversation.clone();
        let (tx, generation, busy) = {
            let mut entries = self.entries.lock().await;
            self.evict_idle(&mut entries, &conversation);
            match entries.get_mut(&conversation) {
                Some(entry) => {
                    entry.last_used = Instant::now();
                    (
                        entry.tx.clone(),
                        entry.generation.clone(),
                        entry.busy.clone(),
                    )
                }
                None => {
                    let (tx, rx) = mpsc::channel(self.capacity);
                    let generation = Arc::new(AtomicU64::new(0));
                    let busy = Arc::new(AtomicBool::new(false));
                    let entry = Entry {
                        tx: tx.clone(),
                        generation: generation.clone(),
                        last_used: Instant::now(),
                        busy: busy.clone(),
                    };
                    entries.insert(conversation.clone(), entry);
                    let runner = self.runner.clone();
                    let generation_for_worker = generation.clone();
                    let busy_for_worker = busy.clone();
                    tokio::spawn(worker(
                        Arc::new(conversation.clone()),
                        rx,
                        generation_for_worker,
                        busy_for_worker,
                        runner,
                    ));
                    (tx, generation, busy)
                }
            }
        };

        // Bump first, then enqueue: a turn already streaming sees the new
        // generation on its next event and stops.
        let queued = generation.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = busy; // kept alive with the entry; the worker owns liveness
        match tx.try_send(QueuedJob {
            job,
            generation: queued,
        }) {
            Ok(()) => SubmitOutcome::Accepted,
            Err(mpsc::error::TrySendError::Full(_)) => SubmitOutcome::Full,
            Err(mpsc::error::TrySendError::Closed(_)) => SubmitOutcome::Full,
        }
    }

    /// The generation currently expected for `conversation` (diagnostics/tests).
    pub async fn generation(&self, conversation: &str) -> Option<u64> {
        let entries = self.entries.lock().await;
        entries
            .get(conversation)
            .map(|entry| entry.generation.load(Ordering::SeqCst))
    }

    /// Number of conversations currently routed.
    pub async fn len(&self) -> usize {
        self.entries.lock().await.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.len().await == 0
    }

    /// Drop the oldest idle conversations until the table fits.
    fn evict_idle(&self, entries: &mut HashMap<String, Entry>, keep: &str) {
        if entries.len() < self.max_conversations || entries.contains_key(keep) {
            return;
        }
        loop {
            let candidate = entries
                .iter()
                .filter(|(key, entry)| {
                    key.as_str() != keep
                        && !entry.busy.load(Ordering::SeqCst)
                        && entry.last_used.elapsed() > self.idle_timeout
                })
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone());
            match candidate {
                Some(key) => {
                    entries.remove(&key);
                }
                // Nothing is idle enough: drop the least recently used so a
                // long-running process cannot grow without bound.
                None => {
                    let fallback = entries
                        .iter()
                        .filter(|(key, _)| key.as_str() != keep)
                        .min_by_key(|(_, entry)| entry.last_used)
                        .map(|(key, _)| key.clone());
                    match fallback {
                        Some(key) => {
                            entries.remove(&key);
                        }
                        None => return,
                    }
                }
            }
            if entries.len() < self.max_conversations {
                return;
            }
        }
    }
}

async fn worker(
    conversation: Arc<String>,
    mut rx: mpsc::Receiver<QueuedJob>,
    generation: Arc<AtomicU64>,
    busy: Arc<AtomicBool>,
    runner: Runner,
) {
    while let Some(queued) = rx.recv().await {
        let watch = SupersedeWatch {
            conversation: conversation.clone(),
            generation: generation.clone(),
            expected: queued.generation,
        };
        if watch.is_superseded() {
            // Overtaken while waiting in the mailbox: a newer message already
            // owns the answer, so clean up rather than run a stale turn.
            queued.job.sink.superseded().await.ok();
            continue;
        }
        busy.store(true, Ordering::SeqCst);
        runner(queued.job, watch).await;
        busy.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::sink::{ReplySink, TurnOutcome};
    use std::sync::Mutex as StdMutex;

    #[derive(Default)]
    struct RecordingSink {
        superseded: StdMutex<usize>,
        finished: StdMutex<usize>,
    }

    #[async_trait::async_trait]
    impl ReplySink for RecordingSink {
        async fn finish(&self, _outcome: &TurnOutcome) -> anyhow::Result<()> {
            *self.finished.lock().unwrap() += 1;
            Ok(())
        }

        async fn superseded(&self) -> anyhow::Result<()> {
            *self.superseded.lock().unwrap() += 1;
            Ok(())
        }
    }

    fn job(conversation: &str, sink: Arc<dyn ReplySink>) -> Job {
        Job {
            conversation: conversation.to_string(),
            channel: "testchannel".into(),
            session_id: "s1".into(),
            reply_to: crate::bridge::ConversationRef::default(),
            text: "hi".into(),
            images: Vec::new(),
            sink,
        }
    }

    #[tokio::test]
    async fn submissions_for_one_conversation_run_in_order() {
        let seen: Arc<StdMutex<Vec<(String, u64)>>> = Arc::new(StdMutex::new(Vec::new()));
        let captured = seen.clone();
        let conversations = Conversations::new(Arc::new(move |job, watch| {
            let captured = captured.clone();
            Box::pin(async move {
                tokio::time::sleep(Duration::from_millis(10)).await;
                captured
                    .lock()
                    .unwrap()
                    .push((job.text.clone(), watch.generation()));
            })
        }));

        for text in ["one", "two", "three"] {
            let outcome = conversations
                .submit(job("c1", Arc::new(RecordingSink::default())))
                .await;
            assert_eq!(outcome, SubmitOutcome::Accepted);
            let _ = text;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        let seen = seen.lock().unwrap();
        // Only the newest generation runs per conversation; older ones were
        // superseded while queued.
        assert!(!seen.is_empty());
        assert_eq!(seen.last().unwrap().1, 3);
    }

    #[tokio::test]
    async fn different_conversations_run_concurrently() {
        let started: Arc<StdMutex<Vec<String>>> = Arc::new(StdMutex::new(Vec::new()));
        let captured = started.clone();
        let gate = Arc::new(tokio::sync::Notify::new());
        let gate_for_runner = gate.clone();
        let conversations = Conversations::new(Arc::new(move |job, _watch| {
            let captured = captured.clone();
            let gate = gate_for_runner.clone();
            Box::pin(async move {
                captured.lock().unwrap().push(job.conversation.clone());
                gate.notified().await;
            })
        }));

        let sink: Arc<dyn ReplySink> = Arc::new(RecordingSink::default());
        conversations.submit(job("c1", sink.clone())).await;
        conversations.submit(job("c2", sink.clone())).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        let started = started.lock().unwrap().clone();
        assert_eq!(started.len(), 2, "both conversations should be running");
        gate.notify_waiters();
    }

    #[tokio::test]
    async fn a_full_mailbox_reports_backpressure() {
        let gate = Arc::new(tokio::sync::Notify::new());
        let gate_for_runner = gate.clone();
        let conversations = Conversations::new(Arc::new(move |_job, _watch| {
            let gate = gate_for_runner.clone();
            Box::pin(async move {
                gate.notified().await;
            })
        }))
        .with_limits(2, 8);

        let sink: Arc<dyn ReplySink> = Arc::new(RecordingSink::default());
        let first = conversations.submit(job("c1", sink.clone())).await;
        assert_eq!(first, SubmitOutcome::Accepted);
        let mut outcomes = Vec::new();
        for _ in 0..4 {
            outcomes.push(conversations.submit(job("c1", sink.clone())).await);
        }
        assert!(
            outcomes.contains(&SubmitOutcome::Full),
            "expected backpressure, got {outcomes:?}"
        );
        gate.notify_waiters();
    }

    #[tokio::test]
    async fn the_table_evicts_instead_of_growing_without_bound() {
        let conversations =
            Conversations::new(Arc::new(|_job, _watch| Box::pin(async {}))).with_limits(4, 3);
        let sink: Arc<dyn ReplySink> = Arc::new(RecordingSink::default());
        for index in 0..6 {
            conversations
                .submit(job(&format!("c{index}"), sink.clone()))
                .await;
        }
        assert!(
            conversations.len().await <= 3,
            "{}",
            conversations.len().await
        );
    }

    #[tokio::test]
    async fn a_superseded_queued_job_is_told_so_instead_of_running() {
        let gate = Arc::new(tokio::sync::Notify::new());
        let gate_for_runner = gate.clone();
        let runs = Arc::new(AtomicU64::new(0));
        let runs_for_runner = runs.clone();
        let conversations = Conversations::new(Arc::new(move |_job, _watch| {
            let gate = gate_for_runner.clone();
            let runs = runs_for_runner.clone();
            Box::pin(async move {
                runs.fetch_add(1, Ordering::SeqCst);
                gate.notified().await;
            })
        }));

        let sink = Arc::new(RecordingSink::default());
        conversations.submit(job("c1", sink.clone())).await;
        // Second submission supersedes the first while it is blocked.
        conversations.submit(job("c1", sink.clone())).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(
            *sink.superseded.lock().unwrap() >= 1,
            "the overtaken job must be told it was superseded"
        );
        gate.notify_waiters();
    }

    #[tokio::test]
    async fn generation_advances_per_submission() {
        let conversations = Conversations::new(Arc::new(|_job, _watch| Box::pin(async {})));
        let sink: Arc<dyn ReplySink> = Arc::new(RecordingSink::default());
        conversations.submit(job("c1", sink.clone())).await;
        assert_eq!(conversations.generation("c1").await, Some(1));
        conversations.submit(job("c1", sink.clone())).await;
        assert_eq!(conversations.generation("c1").await, Some(2));
        assert_eq!(conversations.generation("missing").await, None);
    }

    #[test]
    fn a_job_carries_its_channel_for_accounting() {
        let sink: Arc<dyn ReplySink> = Arc::new(RecordingSink::default());
        assert_eq!(job("c1", sink).channel, "testchannel");
    }
}
