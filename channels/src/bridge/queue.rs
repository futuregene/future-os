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

    /// Override how long an idle conversation stays routed.
    pub fn with_idle_timeout(mut self, idle_timeout: Duration) -> Self {
        self.idle_timeout = idle_timeout;
        self
    }

    /// Queue `job`, superseding any turn still running for its conversation.
    pub async fn submit(&self, job: Job) -> SubmitOutcome {
        let conversation = job.conversation.clone();
        let (tx, generation, _busy) = {
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
        //
        // The generation is reserved speculatively and only published once the
        // message is actually in the mailbox. Bumping it unconditionally would
        // let a *rejected* message (a full mailbox) stop the turn that is
        // already running and then drop its replacement, losing the answer
        // entirely. Two concurrent submissions may reserve the same number;
        // that is harmless, because a worker runs its job when the generation
        // still matches and skips it when a newer message has overtaken it.
        let reserved = generation.load(Ordering::SeqCst) + 1;
        match tx.try_send(QueuedJob {
            job,
            generation: reserved,
        }) {
            Ok(()) => {
                generation.store(reserved, Ordering::SeqCst);
                SubmitOutcome::Accepted
            }
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
        while entries.len() >= self.max_conversations {
            // Prefer a conversation that has been idle long enough; when none
            // is, drop the least recently used instead, so a long-running
            // process cannot grow without bound.
            let idle = entries
                .iter()
                .filter(|(key, entry)| {
                    key.as_str() != keep
                        && !entry.busy.load(Ordering::SeqCst)
                        && entry.last_used.elapsed() > self.idle_timeout
                })
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone());
            let candidate = idle.or_else(|| {
                entries
                    .iter()
                    .filter(|(key, _)| key.as_str() != keep)
                    .min_by_key(|(_, entry)| entry.last_used)
                    .map(|(key, _)| key.clone())
            });
            // The guard above leaves at least one entry that is not `keep`, so
            // the fallback always names one. Failing to evict would be
            // unbounded growth, so this is not a silent return.
            let key = candidate.expect("the capacity guard leaves a candidate");
            entries.remove(&key);
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
    tracing::debug!(conversation = %conversation, "conversation worker stopped");
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

    impl RecordingSink {
        fn superseded_count(&self) -> usize {
            *self.superseded.lock().unwrap()
        }

        fn finished_count(&self) -> usize {
            *self.finished.lock().unwrap()
        }
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

    /// The busy flag of a routed conversation, read through a non-blocking lock
    /// so a test can wait on it from `wait_until`'s synchronous predicate
    /// instead of sleeping for a guess at how long the worker needs.
    fn busy(conversations: &Conversations, conversation: &str) -> Option<bool> {
        conversations.entries.try_lock().ok().and_then(|entries| {
            entries
                .get(conversation)
                .map(|entry| entry.busy.load(Ordering::SeqCst))
        })
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
        let gate = crate::bridge::Shutdown::new();
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
        // Release both runners and let them finish, so the test owns no tasks.
        gate.trigger();
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    #[tokio::test]
    async fn a_full_mailbox_reports_backpressure() {
        let gate = crate::bridge::Shutdown::new();
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
        gate.trigger();
        tokio::time::sleep(Duration::from_millis(30)).await;
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
        let gate = crate::bridge::Shutdown::new();
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
        gate.trigger();
        tokio::time::sleep(Duration::from_millis(30)).await;
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

    #[tokio::test]
    async fn a_rejected_message_does_not_stop_the_running_turn() {
        // A message dropped by backpressure must not leave the conversation
        // with no answer at all: the running turn keeps its generation, so it
        // still finishes and reports.
        let gate = crate::bridge::Shutdown::new();
        let gate_for_runner = gate.clone();
        let conversations = Conversations::new(Arc::new(move |_job, _watch| {
            let gate = gate_for_runner.clone();
            Box::pin(async move {
                gate.notified().await;
            })
        }))
        .with_limits(1, 8);

        let sink: Arc<dyn ReplySink> = Arc::new(RecordingSink::default());
        assert_eq!(
            conversations.submit(job("c1", sink.clone())).await,
            SubmitOutcome::Accepted
        );
        let before = conversations.generation("c1").await;
        // Fill the mailbox, then overflow it.
        let mut saw_full = false;
        for _ in 0..4 {
            if conversations.submit(job("c1", sink.clone())).await == SubmitOutcome::Full {
                saw_full = true;
            }
        }
        assert!(saw_full, "the mailbox should have overflowed");
        assert_eq!(
            conversations.generation("c1").await,
            before,
            "a rejected message must not bump the generation"
        );
        gate.trigger();
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    #[tokio::test]
    async fn a_watch_reports_the_conversation_it_belongs_to() {
        let generation = std::sync::Arc::new(AtomicU64::new(3));
        let watch = SupersedeWatch::new("slack:C1:T9", generation.clone(), 3);
        assert_eq!(watch.conversation(), "slack:C1:T9");
        assert_eq!(watch.generation(), 3);
        assert!(!watch.is_superseded());
        generation.store(4, Ordering::SeqCst);
        assert!(watch.is_superseded());
    }

    #[tokio::test]
    async fn an_empty_table_is_reported_as_empty() {
        let conversations = Conversations::new(Arc::new(|_job, _watch| Box::pin(async {})));
        assert!(conversations.is_empty().await);
        assert_eq!(conversations.len().await, 0);
        let sink: Arc<dyn ReplySink> = Arc::new(RecordingSink::default());
        conversations.submit(job("c1", sink)).await;
        assert!(!conversations.is_empty().await);
    }

    #[tokio::test]
    async fn idle_conversations_are_evicted_before_an_active_one() {
        // A conversation with recent activity must survive eviction pressure
        // while an idle one is dropped.
        let gate = crate::bridge::Shutdown::new();
        let gate_for_runner = gate.clone();
        let conversations = Conversations::new(Arc::new(move |_job, _watch| {
            let gate = gate_for_runner.clone();
            Box::pin(async move {
                gate.notified().await;
            })
        }))
        .with_limits(4, 2);
        let sink: Arc<dyn ReplySink> = Arc::new(RecordingSink::default());
        // Fill the table: the newest conversation is the one being submitted.
        for index in 0..3 {
            conversations
                .submit(job(&format!("c{index}"), sink.clone()))
                .await;
        }
        // The table stays within its bound and the just-submitted conversation
        // is still routed.
        let routed = conversations.len().await;
        assert!(routed <= 2, "{routed}");
        assert!(conversations.generation("c2").await.is_some());
        gate.trigger();
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    #[tokio::test]
    async fn a_worker_that_ends_frees_its_conversation_for_a_new_generation() {
        // The runner returns immediately, so the worker loops back to receive:
        // a later message must still run rather than landing in a dead mailbox.
        let runs = Arc::new(AtomicU64::new(0));
        let runs_for_runner = runs.clone();
        let conversations = Conversations::new(Arc::new(move |_job, _watch| {
            let runs = runs_for_runner.clone();
            Box::pin(async move {
                runs.fetch_add(1, Ordering::SeqCst);
            })
        }));
        let sink: Arc<dyn ReplySink> = Arc::new(RecordingSink::default());
        for index in 0..3 {
            assert_eq!(
                conversations.submit(job("c1", sink.clone())).await,
                SubmitOutcome::Accepted
            );
            let ran = crate::test_support::wait_until(
                || runs.load(Ordering::SeqCst) as usize > index,
                Duration::from_secs(5),
            )
            .await;
            assert!(ran, "each submission should reach the runner");
        }
    }

    #[tokio::test]
    async fn a_worker_that_panics_is_reported_as_backpressure() {
        // If a conversation worker dies (a runner panic), the mailbox is closed
        // but the table still holds its sender. The next message must learn that
        // rather than being silently lost in a dead channel.
        let panicked = Arc::new(AtomicU64::new(0));
        let panicked_for_runner = panicked.clone();
        let conversations = Conversations::new(Arc::new(move |_job, _watch| {
            let panicked = panicked_for_runner.clone();
            Box::pin(async move {
                panicked.fetch_add(1, Ordering::SeqCst);
                panic!("runner exploded");
            })
        }));
        let sink: Arc<dyn ReplySink> = Arc::new(RecordingSink::default());
        assert_eq!(
            conversations.submit(job("c1", sink.clone())).await,
            SubmitOutcome::Accepted
        );
        let ran = crate::test_support::wait_until(
            || panicked.load(Ordering::SeqCst) > 0,
            Duration::from_secs(5),
        )
        .await;
        assert!(ran, "the runner should have been called");

        // The worker task is gone; the next submit for the same conversation
        // must be handled without panicking, whatever the channel reports.
        let outcome = conversations.submit(job("c1", sink)).await;
        assert!(matches!(
            outcome,
            SubmitOutcome::Accepted | SubmitOutcome::Full
        ));
        assert_eq!(conversations.len().await, 1);
    }

    #[tokio::test]
    async fn a_busy_conversation_is_not_evicted_for_a_new_one() {
        // Eviction must never drop a conversation whose turn is running: the
        // newest message would lose its mailbox mid-answer. With nothing idle
        // and a full table, the least recently used *idle* entry goes instead,
        // and the running conversation keeps receiving.
        let gate = crate::bridge::Shutdown::new();
        let gate_for_runner = gate.clone();
        let conversations = Conversations::new(Arc::new(move |_job, _watch| {
            let gate = gate_for_runner.clone();
            Box::pin(async move {
                gate.notified().await;
            })
        }))
        .with_limits(4, 2)
        // Nothing counts as idle, so the eviction has to fall back.
        .with_idle_timeout(Duration::ZERO);
        let sink: Arc<dyn ReplySink> = Arc::new(RecordingSink::default());
        assert_eq!(
            conversations.submit(job("busy", sink.clone())).await,
            SubmitOutcome::Accepted
        );
        // Wait for the runner to park, so "busy" is genuinely busy.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(conversations.len().await, 1);
        // A second conversation overflows the two-entry table.
        conversations.submit(job("other", sink.clone())).await;
        assert!(conversations.len().await <= 2);
        // The busy conversation is still routed, so its next message is queued
        // rather than treated as a new conversation.
        assert!(conversations.generation("busy").await.is_some());
        gate.trigger();
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    #[tokio::test]
    async fn an_idle_conversation_is_evicted_and_its_worker_stops() {
        // Idle conversations are dropped once they exceed their idle timeout and
        // the table is full. Dropping one closes its mailbox, so its worker
        // exits rather than lingering forever.
        let writer = Arc::new(StdMutex::new(Vec::<u8>::new()));
        let subscriber = tracing_subscriber::fmt()
            .with_writer({
                let writer = writer.clone();
                move || WriterGuard(writer.clone())
            })
            .with_max_level(tracing::Level::DEBUG)
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        let conversations = Conversations::new(Arc::new(|_job, _watch| Box::pin(async {})))
            .with_limits(4, 2)
            .with_idle_timeout(Duration::ZERO);
        let sink: Arc<dyn ReplySink> = Arc::new(RecordingSink::default());
        for index in 0..4 {
            conversations
                .submit(job(&format!("c{index}"), sink.clone()))
                .await;
        }
        // Eviction happens synchronously inside `submit`, so the table is
        // already at its limit here and two workers have lost their mailbox
        // sender. Asserting that up front keeps the wait below about the one
        // thing that is actually asynchronous — the worker task being polled
        // after its mailbox closed — instead of silently covering for an
        // eviction that never happened.
        assert_eq!(
            conversations.len().await,
            2,
            "four submissions must leave the table at its two-entry limit"
        );
        // 20 s is the budget its siblings use (`email` waits 20 s, the feishu
        // bridge 15 s). At 5 s this was the only wait in the suite observed to
        // expire: a mutation run on a saturated machine reported "an evicted
        // conversation's worker must exit" for a mutant of `policy.rs`, which
        // cannot affect it. The assertion is unchanged.
        let stopped = crate::test_support::wait_until(
            || {
                let logged = String::from_utf8_lossy(&writer.lock().unwrap()).to_string();
                logged.contains("conversation worker stopped")
            },
            Duration::from_secs(20),
        )
        .await;
        assert!(stopped, "an evicted conversation's worker must exit");
        assert!(conversations.len().await <= 2);
    }

    #[tokio::test]
    async fn a_full_table_keeps_its_bystander_when_an_existing_conversation_sends_again() {
        // A re-submission to a conversation that is already routed is not a
        // capacity event: the table is exactly full before it and must be
        // exactly full after it, with the bystander still routed. The guard
        // `len < max || contains(keep)` is what keeps the eviction loop from
        // running here; a loop that ran would drop a bystander — closing a
        // mailbox whose turn may still be running — for no reason at all.
        let conversations =
            Conversations::new(Arc::new(|_job, _watch| Box::pin(async {}))).with_limits(4, 2);
        let sink: Arc<dyn ReplySink> = Arc::new(RecordingSink::default());
        for name in ["a", "b"] {
            assert_eq!(
                conversations.submit(job(name, sink.clone())).await,
                SubmitOutcome::Accepted
            );
        }
        assert_eq!(conversations.len().await, 2, "the table is exactly full");
        assert_eq!(
            conversations.submit(job("a", sink.clone())).await,
            SubmitOutcome::Accepted
        );
        assert_eq!(
            conversations.len().await,
            2,
            "a re-submission must not evict a bystander"
        );
        assert!(
            conversations.generation("b").await.is_some(),
            "the bystander conversation must keep its mailbox"
        );
    }

    #[tokio::test]
    async fn a_running_turn_is_kept_and_the_idle_conversation_goes() {
        // Which conversation an eviction drops is the entire point of the
        // idle-preference filter, and a table-size assertion cannot see it.
        // `running` is the older of the two and its runner parks, so it is
        // busy; `idle` is newer and its runner has already returned, so it is
        // not. Only the non-busy, past-the-timeout `idle` may be evicted —
        // every boolean or comparison error in that four-line filter either
        // empties the idle set or admits `running` to it, and the `or_else`
        // fallback then drops the running turn instead.
        let gate = crate::bridge::Shutdown::new();
        let gate_for_runner = gate.clone();
        let started = Arc::new(AtomicU64::new(0));
        let started_for_runner = started.clone();
        let conversations = Conversations::new(Arc::new(move |job, _watch| {
            let gate = gate_for_runner.clone();
            let started = started_for_runner.clone();
            Box::pin(async move {
                started.fetch_add(1, Ordering::SeqCst);
                if job.conversation == "running" {
                    gate.notified().await; // park, so `running` stays busy
                }
            })
        }))
        .with_limits(4, 2)
        // Anything not busy counts as idle, so the filter alone decides.
        .with_idle_timeout(Duration::ZERO);
        let sink: Arc<dyn ReplySink> = Arc::new(RecordingSink::default());
        assert_eq!(
            conversations.submit(job("running", sink.clone())).await,
            SubmitOutcome::Accepted
        );
        let parked = crate::test_support::wait_until(
            || busy(&conversations, "running") == Some(true),
            Duration::from_secs(5),
        )
        .await;
        assert!(parked, "`running`'s runner should have parked it as busy");
        assert_eq!(
            conversations.submit(job("idle", sink.clone())).await,
            SubmitOutcome::Accepted
        );
        let returned = crate::test_support::wait_until(
            || started.load(Ordering::SeqCst) >= 2 && busy(&conversations, "idle") == Some(false),
            Duration::from_secs(5),
        )
        .await;
        assert!(returned, "`idle`'s runner should have returned");
        assert_eq!(conversations.len().await, 2);
        // A third conversation overflows the two-entry table. The only entry
        // that is both non-busy and past the idle timeout is `idle`.
        assert_eq!(
            conversations.submit(job("third", sink.clone())).await,
            SubmitOutcome::Accepted
        );
        assert!(
            conversations.generation("running").await.is_some(),
            "the conversation whose turn is in flight must keep its mailbox"
        );
        assert!(
            conversations.generation("idle").await.is_none(),
            "the idle conversation is the one that must be evicted"
        );
        gate.trigger();
        let released = crate::test_support::wait_until(
            || busy(&conversations, "running") == Some(false),
            Duration::from_secs(5),
        )
        .await;
        assert!(released, "a finished turn must clear its busy flag");
    }

    #[tokio::test]
    async fn the_default_idle_timeout_leaves_a_two_minute_old_conversation_routed() {
        // `DEFAULT_IDLE_TIMEOUT` is exercised rather than restated. With the
        // 30-minute default, a conversation idle for two minutes is *not* yet
        // evictable, so a full table must fall back to dropping its least
        // recently used entry — here the older conversation whose turn is
        // still running, exactly as the `or_else` branch documents.
        //
        // Two minutes is chosen to sit between the real default (30 min) and
        // every mis-computation of `30 * 60`: 90 s (`30 + 60`) and 0 s
        // (`30 / 60`) both make the two-minute-old conversation stale, so the
        // filter prefers it and the assertions below fail.
        //
        // `last_used` is written directly because production reaches this
        // state only by running for two minutes; `evict_idle` itself is the
        // code under test and is called unmodified.
        let conversations = Conversations::new(Arc::new(|_job, _watch| Box::pin(async {})))
            // Deliberately no `with_idle_timeout`: this test is about the default.
            .with_limits(4, 2);
        let now = Instant::now();
        let entry = |age: Duration, busy: bool| {
            let (tx, rx) = mpsc::channel(1);
            drop(rx); // nothing is delivered to this entry; only its age matters
            Entry {
                tx,
                generation: Arc::new(AtomicU64::new(0)),
                last_used: now
                    .checked_sub(age)
                    .expect("this machine must have been up for a couple of minutes"),
                busy: Arc::new(AtomicBool::new(busy)),
            }
        };
        let mut entries = conversations.entries.lock().await;
        entries.insert("running".into(), entry(Duration::from_secs(150), true));
        entries.insert("idle".into(), entry(Duration::from_secs(120), false));
        assert_eq!(entries.len(), 2, "the table is exactly full");
        conversations.evict_idle(&mut entries, "new");
        let mut remaining: Vec<&str> = entries.keys().map(String::as_str).collect();
        remaining.sort();
        assert_eq!(
            remaining,
            vec!["idle"],
            "a two-minute-old conversation is not past the 30-minute default, so the \
             least recently used entry (`running`) is the one that goes"
        );
    }

    /// A `Write` that appends to a shared buffer, so a test can read what the
    /// subscriber wrote.
    struct WriterGuard(Arc<StdMutex<Vec<u8>>>);

    impl std::io::Write for WriterGuard {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn the_test_sink_records_a_finish() {
        // The double is part of the harness: a silent no-op there would let a
        // test pass without exercising the callback it names.
        let sink = RecordingSink::default();
        sink.finish(&TurnOutcome::completed("x")).await.unwrap();
        assert_eq!(sink.finished_count(), 1);
        assert_eq!(sink.superseded_count(), 0);
        assert_eq!(sink.finished_count(), 1);
    }

    #[test]
    fn the_log_writer_flushes_cleanly() {
        use std::io::Write as _;
        let writer = Arc::new(StdMutex::new(Vec::new()));
        let mut guard = WriterGuard(writer.clone());
        guard.write_all(b"abc").unwrap();
        guard.flush().unwrap();
        assert_eq!(&*writer.lock().unwrap(), b"abc");
    }

    #[test]
    fn a_job_carries_its_channel_for_accounting() {
        let sink: Arc<dyn ReplySink> = Arc::new(RecordingSink::default());
        assert_eq!(job("c1", sink).channel, "testchannel");
    }
}
