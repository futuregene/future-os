//! Ordered event batches, durable flush barriers and persistence health.
use super::BROADCAST_RING_CAPACITY;
use crate::session::sqlite_store::is_event_rejected;
pub(super) type AppendReceipt = super::journal_control::WorkerReceipt<Result<(), AppendFailure>>;

#[cfg(test)]
pub(super) const EVENT_BATCH_WINDOW: std::time::Duration = std::time::Duration::from_millis(100);
/// How long a batch that could not be committed waits before the writer tries
/// the same events again. A storage failure is usually contention on a shared
/// SQLite database (another session's long transaction, a checkpoint of a large
/// WAL), and such a window opens and closes on its own — so the writer keeps the
/// events it already accepted and retries them instead of waiting for the next
/// append. A successful retry is also what clears the journal health flag
/// (see `flush_event_batch`), which is the only self-healing path a session with
/// no run can take.
pub(super) const EVENT_RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);
const EVENT_BATCH_MAX_EVENTS: usize = 128;
const EVENT_BATCH_MAX_BYTES: usize = 64 * 1024;
const EVENT_BATCH_QUEUE_CAPACITY: usize = BROADCAST_RING_CAPACITY;

/// Production has a single event encoder. The legacy variant is only for
/// historical fixtures exercising the old replay/compaction contract.
enum JournalWriter {
    Canonical(crate::session::content::journal::Writer),
    #[cfg(test)]
    Legacy,
}

impl JournalWriter {
    fn new(
        store: &crate::session::sqlite_store::SqliteStore,
        session: &str,
    ) -> anyhow::Result<Self> {
        match store.storage_format(session)? {
            crate::session::content::FORMAT_VERSION => Ok(Self::Canonical(Default::default())),
            #[cfg(test)]
            _ => Ok(Self::Legacy),
            #[cfg(not(test))]
            _ => {
                anyhow::bail!("conversation must finish migration before binding its event writer")
            }
        }
    }

    fn persist<'a>(
        &mut self,
        store: &crate::session::sqlite_store::SqliteStore,
        session: &str,
        events: impl IntoIterator<Item = &'a serde_json::Value>,
    ) -> anyhow::Result<()> {
        match self {
            Self::Canonical(writer) => writer.persist(store, session, events),
            #[cfg(test)]
            Self::Legacy => store.append_events(session, events.into_iter().cloned().collect()),
        }
    }

    fn window(&self) -> std::time::Duration {
        match self {
            Self::Canonical(_) => std::time::Duration::from_millis(25),
            #[cfg(test)]
            Self::Legacy => EVENT_BATCH_WINDOW,
        }
    }
}

#[derive(Default)]
pub(super) struct EventJournalHealth {
    last_error: parking_lot::Mutex<Option<String>>,
    pub(super) metrics: super::journal_metrics::JournalMetrics,
    interrupt_flag: parking_lot::Mutex<Option<std::sync::Arc<std::sync::atomic::AtomicBool>>>,
    /// Test-only: make the next `n` batch commits fail the way a transient
    /// storage failure does (the batch is kept, not dropped).
    #[cfg(test)]
    injected_failures: std::sync::atomic::AtomicUsize,
}

impl EventJournalHealth {
    pub(super) fn error(&self) -> Option<String> {
        self.last_error.lock().clone()
    }

    pub(super) fn clear(&self) {
        *self.last_error.lock() = None;
    }

    #[cfg(test)]
    pub(super) fn inject_commit_failures(&self, count: usize) {
        self.injected_failures
            .store(count, std::sync::atomic::Ordering::SeqCst);
    }

    /// Consume one injected failure. Test-only and single-threaded by
    /// construction: the flag is armed before a broadcast and read by the one
    /// writer thread.
    #[cfg(test)]
    fn take_injected_failure(&self) -> bool {
        let remaining = self
            .injected_failures
            .load(std::sync::atomic::Ordering::SeqCst);
        if remaining == 0 {
            return false;
        }
        self.injected_failures
            .store(remaining - 1, std::sync::atomic::Ordering::SeqCst);
        true
    }

    pub(super) fn fail(&self, message: String) {
        // Publish the error and interrupt as one health transition. Readers
        // that observe the error must also observe the cancellation signal.
        let mut error = self.last_error.lock();
        *error = Some(message);
        if let Some(flag) = self.interrupt_flag.lock().as_ref() {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    pub(super) fn set_interrupt(&self, flag: std::sync::Arc<std::sync::atomic::AtomicBool>) {
        // Use the same lock order as fail so installing the signal cannot
        // race an error between the health check and flag registration.
        let error = self.last_error.lock();
        if error.is_some() {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        *self.interrupt_flag.lock() = Some(flag);
    }

    pub(super) fn clear_interrupt(&self) {
        *self.interrupt_flag.lock() = None;
    }
}

/// Why one event did not reach the journal. The two arms are not
/// interchangeable to a caller: a refusal cannot be retried, while a storage
/// failure can — and only the storage failure makes the journal unhealthy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum AppendFailure {
    /// The journal will never accept this event (see `EventRejected`).
    Rejected(String),
    /// The journal could not be written now; the writer keeps the event.
    Unavailable(String),
}

impl AppendFailure {
    fn unavailable(message: impl Into<String>) -> Self {
        Self::Unavailable(message.into())
    }
}

/// Renders the inner message, so a caller that only wants to report the failure
/// (and the tests that assert on its text) keeps working unchanged.
impl std::fmt::Display for AppendFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected(message) | Self::Unavailable(message) => f.write_str(message),
        }
    }
}

pub(super) enum EventWriteCommand {
    Append {
        sequence: u64,
        event: serde_json::Value,
        bytes: usize,
        reservation: Option<super::admission::Reservation>,
        reply: Option<crossbeam_channel::Sender<Result<(), AppendFailure>>>,
    },
    /// Test-only gate: the worker blocks inside the command loop until the
    /// test releases the barrier. It lets a test act on the command channel
    /// (queue an event without flushing it, drop the last sender) at a known
    /// point in the worker's lifecycle instead of racing a wall-clock batch
    /// window. Compiled out of production builds entirely.
    #[cfg(test)]
    Hold(std::sync::Arc<std::sync::Barrier>),
}

struct PendingEvent {
    event: serde_json::Value,
    bytes: usize,
    _reservation: Option<super::admission::Reservation>,
    reply: Option<crossbeam_channel::Sender<Result<(), AppendFailure>>>,
}

pub(super) struct EventBatchWriter {
    pub(super) sender: crossbeam_channel::Sender<EventWriteCommand>,
    pub(super) control: super::journal_control::JournalControl,
    pub(super) thread: Option<std::thread::JoinHandle<()>>,
    pub(super) health: std::sync::Arc<EventJournalHealth>,
    pub(super) admission: std::sync::Arc<super::admission::Admission>,
    #[cfg(test)]
    pub(super) commits: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl EventBatchWriter {
    pub(super) fn new(
        store: crate::session::sqlite_store::SqliteStore,
        session_id: String,
        health: std::sync::Arc<EventJournalHealth>,
    ) -> anyhow::Result<Self> {
        let (sender, receiver) =
            crossbeam_channel::bounded::<EventWriteCommand>(EVENT_BATCH_QUEUE_CAPACITY);
        let (control_sender, controls) = crossbeam_channel::bounded(64);
        let (control, worker_exit) = super::journal_control::JournalControl::new(control_sender);
        #[cfg(test)]
        let commits = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        #[cfg(test)]
        let worker_commits = commits.clone();
        let worker_health = health.clone();
        let thread = std::thread::Builder::new()
            .name("agent-event-batch".into())
            .spawn(move || {
                let _worker_exit = worker_exit;
                run_event_batch_writer(
                    receiver,
                    controls,
                    store,
                    session_id,
                    worker_health,
                    #[cfg(test)]
                    worker_commits,
                );
            })?;
        Ok(Self {
            sender,
            control,
            thread: Some(thread),
            health,
            admission: Default::default(),
            #[cfg(test)]
            commits,
        })
    }

    pub(super) fn append(
        &self,
        event: serde_json::Value,
        durable: bool,
    ) -> Result<(), AppendFailure> {
        if let Some(error) = self.health.error() {
            return Err(AppendFailure::Unavailable(error));
        }
        let (reply, receiver) = if durable {
            let (sender, receiver) = crossbeam_channel::bounded(1);
            (Some(sender), Some(self.control.receipt(receiver)))
        } else {
            (None, None)
        };
        // The bounded queue applies backpressure, not data loss. A transient
        // full queue is not a storage failure: wait for the dedicated writer
        // (which never acquires broadcaster/session locks) to make space.
        // As at a durable boundary, actual writer failure remains fail-closed.
        let bytes = event.to_string().len();
        let mut admitted = self.control.admitted.lock();
        let sequence = admitted
            .checked_add(1)
            .ok_or_else(|| AppendFailure::unavailable("event sequence exhausted"))?;
        self.sender
            .send(EventWriteCommand::Append {
                sequence,
                event,
                bytes,
                reply,
                reservation: None,
            })
            .map_err(|_| {
                let message = "event persistence worker is unavailable".to_string();
                self.health.fail(message.clone());
                AppendFailure::Unavailable(message)
            })?;
        *admitted = sequence;
        drop(admitted);
        if let Some(receiver) = receiver {
            receiver.recv().map_err(|_| {
                AppendFailure::unavailable("event persistence worker is unavailable")
            })?
        } else {
            Ok(())
        }
    }

    /// Called under the sequence lock: never waits for queue space or a commit.
    pub(super) fn enqueue(
        &self,
        event: serde_json::Value,
        durable: bool,
        bytes: usize,
    ) -> Result<Option<AppendReceipt>, AppendFailure> {
        if let Some(error) = self.health.error() {
            return Err(AppendFailure::Unavailable(error));
        }
        let reservation = self.admission.reserve(bytes)?;
        let (reply, receiver) = if durable {
            let (sender, receiver) = crossbeam_channel::bounded(1);
            (Some(sender), Some(self.control.receipt(receiver)))
        } else {
            (None, None)
        };
        let mut admitted = self.control.admitted.lock();
        let sequence = admitted
            .checked_add(1)
            .ok_or_else(|| AppendFailure::unavailable("event sequence exhausted"))?;
        self.sender
            .try_send(EventWriteCommand::Append {
                sequence,
                event,
                bytes,
                reply,
                reservation: Some(reservation),
            })
            .map_err(|error| {
                AppendFailure::Unavailable(match error {
                    crossbeam_channel::TrySendError::Full(_) => {
                        "event queue is full; generation interrupted".into()
                    }
                    crossbeam_channel::TrySendError::Disconnected(_) => {
                        "event persistence worker is unavailable".into()
                    }
                })
            })?;
        *admitted = sequence;
        Ok(receiver)
    }

    pub(super) fn shutdown(&mut self) -> anyhow::Result<()> {
        if self.thread.is_none() {
            return Ok(());
        }
        let receiver = self.control.queue_shutdown()?;
        let result = receiver
            .recv()
            .map_err(|_| {
                // The receipt hands back an answer the worker queued, so reaching
                // this arm means the worker retired without answering — and the
                // failure it recorded on its way out is what made it retire.
                // Naming it keeps a real loss diagnosable instead of anonymous.
                let error = self.health.error();
                tracing::error!(
                    event = "event_journal_shutdown_ack_lost",
                    ?error,
                    body_queue = self.sender.len(),
                    control_queue = self.control.sender.len(),
                    "Event journal worker exited without its shutdown acknowledgement"
                );
                match error {
                    Some(reason) => {
                        anyhow::anyhow!("event shutdown acknowledgement lost: {reason}")
                    }
                    None => anyhow::anyhow!("event shutdown acknowledgement lost"),
                }
            })?
            .map_err(anyhow::Error::msg);
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| anyhow::anyhow!("event persistence worker panicked"))?;
        }
        result
    }

    #[cfg(test)]
    pub(super) fn flush(&self) -> anyhow::Result<()> {
        self.control
            .queue_flush()?
            .recv()
            .map_err(|_| anyhow::anyhow!("event persistence worker is unavailable"))?
            .map_err(anyhow::Error::msg)
    }

    #[cfg(test)]
    pub(super) fn commit_count(&self) -> u64 {
        self.commits.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Test-only gate command: parks the worker inside the command loop until
    /// the barrier is released. The barrier is waited on twice ("arrived" and
    /// "released") so the test knows the worker has consumed every command
    /// queued ahead of the gate before it manipulates the channel.
    #[cfg(test)]
    pub(super) fn hold_worker_for_test(&self) -> std::sync::Arc<std::sync::Barrier> {
        let gate = std::sync::Arc::new(std::sync::Barrier::new(2));
        self.sender
            .send(EventWriteCommand::Hold(gate.clone()))
            .expect("hold command is accepted while the worker runs");
        gate
    }

    /// Test-only: drop every command sender while the worker is parked in
    /// `Hold`, so the worker's next `recv`/`recv_timeout` observes a
    /// disconnected channel. The replacement sender's receiver is already
    /// gone, which means the writer's own `Drop` handshake is skipped instead
    /// of blocking on a worker that has exited.
    #[cfg(test)]
    pub(super) fn disconnect_worker_for_test(&mut self) {
        let (dead, receiver) = crossbeam_channel::bounded(1);
        drop(receiver);
        let sender = std::mem::replace(&mut self.sender, dead);
        drop(sender);
        let (dead, receiver) = crossbeam_channel::bounded(1);
        drop(receiver);
        let sender = std::mem::replace(&mut self.control.sender, dead);
        drop(sender);
    }

    /// Test-only: retire the worker through its real `Shutdown` handshake and
    /// join the thread, so "the worker is gone" is an observed fact rather
    /// than a sleep-and-hope wait. A later `append` must then fail closed.
    #[cfg(test)]
    pub(super) fn stop_worker_for_test(&mut self) {
        if let Ok(receiver) = self.control.queue_shutdown() {
            let _ = receiver.recv();
        }
        if let Some(thread) = self.thread.take() {
            thread.join().expect("event batch worker exits on shutdown");
        }
    }
}

impl Drop for EventBatchWriter {
    fn drop(&mut self) {
        if self.thread.is_none() {
            return;
        }
        if let Ok(receiver) = self.control.queue_shutdown() {
            let _ = receiver.recv();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run_event_batch_writer(
    receiver: crossbeam_channel::Receiver<EventWriteCommand>,
    controls: crossbeam_channel::Receiver<super::journal_control::ControlCommand>,
    store: crate::session::sqlite_store::SqliteStore,
    session_id: String,
    health: std::sync::Arc<EventJournalHealth>,
    #[cfg(test)] commits: std::sync::Arc<std::sync::atomic::AtomicU64>,
) {
    let _storage_context = crate::session::database::request::Context::new(
        "event_journal".into(),
        session_id.clone(),
        None,
    )
    .enter();
    let mut canonical = match JournalWriter::new(&store, &session_id) {
        Ok(writer) => writer,
        Err(error) => {
            health.fail(format!("cannot determine journal format: {error:#}"));
            return;
        }
    };
    let window = if store.db.is_nfs() {
        std::time::Duration::from_millis(250)
    } else {
        canonical.window()
    };
    let mut pending = Vec::<PendingEvent>::new();
    let mut pending_bytes = 0usize;
    // When the loop must wake on its own: the batch window while the journal is
    // healthy, or the retry interval while it is not. Both are "something is
    // buffered and nothing else will wake us".
    let mut due = None::<std::time::Instant>;
    let mut failed = false;
    let mut consumed = 0u64;
    let mut control = None::<super::journal_control::ControlCommand>;
    loop {
        // Only one control is pulled out of the bounded lane at a time. Body
        // is consumed through its captured target before the reply can advance.
        if control.is_none() {
            control = controls.try_recv().ok();
        }
        if control
            .as_ref()
            .is_some_and(|command| command.target <= consumed)
        {
            let command = control.take().expect("ready control");
            let (now_failed, now_bytes, now_due) = flush_and_schedule(
                &store,
                &session_id,
                &health,
                &mut pending,
                &mut canonical,
                window,
                #[cfg(test)]
                &commits,
            );
            failed = now_failed;
            pending_bytes = now_bytes;
            due = now_due;
            command.reply.send(if failed {
                Err(health
                    .error()
                    .unwrap_or_else(|| "event persistence is unavailable".into()))
            } else {
                Ok(())
            });
            if command.shutdown {
                break;
            }
            continue;
        }
        let timer = match due {
            Some(at) => {
                crossbeam_channel::after(at.saturating_duration_since(std::time::Instant::now()))
            }
            None => crossbeam_channel::never(),
        };
        let command = if control.is_some() {
            // A fence has priority but must consume its preceding body first.
            crossbeam_channel::select_biased! {
                recv(receiver) -> command => match command { Ok(command) => Some(command), Err(_) => break },
                recv(timer) -> _ => None,
            }
        } else {
            crossbeam_channel::select_biased! {
                recv(controls) -> command => {
                    match command { Ok(command) => { control = Some(command); continue; }, Err(_) => break }
                },
                recv(receiver) -> command => match command { Ok(command) => Some(command), Err(_) => break },
                recv(timer) -> _ => None,
            }
        };
        let Some(command) = command else {
            let (now_failed, now_bytes, now_due) = flush_and_schedule(
                &store,
                &session_id,
                &health,
                &mut pending,
                &mut canonical,
                window,
                #[cfg(test)]
                &commits,
            );
            failed = now_failed;
            pending_bytes = now_bytes;
            due = now_due;
            continue;
        };

        match command {
            EventWriteCommand::Append {
                sequence,
                event,
                bytes,
                reply,
                reservation,
            } => {
                consumed = sequence;
                let mut reply = reply;
                if failed {
                    if let Some(sender) = reply.take() {
                        let _ = sender.send(Err(AppendFailure::Unavailable(
                            health
                                .error()
                                .unwrap_or_else(|| "event persistence is unavailable".into()),
                        )));
                    }
                    // Admission preceded the failure. Keep the accepted body
                    // and its memory reservation for explicit recovery.
                }
                pending_bytes = pending_bytes.saturating_add(bytes);
                let durable = reply.is_some();
                pending.push(PendingEvent {
                    event,
                    bytes,
                    reply,
                    _reservation: reservation,
                });
                due.get_or_insert_with(|| std::time::Instant::now() + window);
                if durable
                    || pending.len() >= EVENT_BATCH_MAX_EVENTS
                    || pending_bytes >= EVENT_BATCH_MAX_BYTES
                {
                    let (now_failed, now_bytes, now_due) = flush_and_schedule(
                        &store,
                        &session_id,
                        &health,
                        &mut pending,
                        &mut canonical,
                        window,
                        #[cfg(test)]
                        &commits,
                    );
                    failed = now_failed;
                    pending_bytes = now_bytes;
                    due = now_due;
                }
            }
            #[cfg(test)]
            EventWriteCommand::Hold(gate) => {
                gate.wait(); // arrived inside the command loop
                gate.wait(); // released by the test
            }
        }
    }
}

/// Flush whatever is buffered and say when the loop must wake next.
///
/// A batch that could not be committed stays in `pending` (see
/// `flush_event_batch`), so the retry writes *those* events rather than waiting
/// for a new one to arrive: the journal heals itself while nobody is running.
fn flush_and_schedule(
    store: &crate::session::sqlite_store::SqliteStore,
    session_id: &str,
    health: &EventJournalHealth,
    pending: &mut Vec<PendingEvent>,
    canonical: &mut JournalWriter,
    window: std::time::Duration,
    #[cfg(test)] commits: &std::sync::atomic::AtomicU64,
) -> (bool, usize, Option<std::time::Instant>) {
    let failed = !flush_event_batch(
        store,
        session_id,
        health,
        pending,
        canonical,
        #[cfg(test)]
        commits,
    );
    let bytes = pending.iter().map(|item| item.bytes).sum();
    // Nothing buffered means nothing to retry: a failure recorded on an empty
    // queue is left to `recover_session`/`retry_persistence`, which do not need
    // a run to be woken by.
    let due = (!pending.is_empty())
        .then(|| std::time::Instant::now() + if failed { EVENT_RETRY_INTERVAL } else { window });
    (failed, bytes, due)
}

fn flush_event_batch(
    store: &crate::session::sqlite_store::SqliteStore,
    session_id: &str,
    health: &EventJournalHealth,
    pending: &mut Vec<PendingEvent>,
    canonical: &mut JournalWriter,
    #[cfg(test)] commits: &std::sync::atomic::AtomicU64,
) -> bool {
    if pending.is_empty() {
        return true;
    }
    #[cfg(test)]
    if health.take_injected_failure() {
        // Same shape as a real transient failure: the batch is kept, health is
        // marked, and the reply stays with its event for the next attempt.
        health.fail("event batch commit failed: injected storage failure".into());
        return false;
    }
    let started = std::time::Instant::now();
    let result = canonical.persist(store, session_id, pending.iter().map(|item| &item.event));
    health.metrics.record(
        pending.len(),
        pending.iter().map(|item| item.bytes).sum(),
        started.elapsed(),
        result.is_ok(),
    );
    match result {
        Ok(()) => {
            #[cfg(test)]
            commits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // The journal just proved it is writable again. Clearing here —
            // rather than only at a run's journal rebind — is what keeps a
            // transient failure (SQLITE_BUSY under another session's long
            // transaction) from outliving it: without this, the flag is only
            // cleared by a write path that the flag itself blocks, and a
            // session with no run can never get back to writable.
            health.clear();
            for item in pending.drain(..) {
                if let Some(reply) = item.reply {
                    let _ = reply.send(Ok(()));
                }
            }
            true
        }
        // A refusal is permanent for that one event, so retrying the batch would
        // fail it forever — and every later batch queued behind it, until the
        // agent restarted. Retry the events singly to find the ones this journal
        // refuses, drop those, and keep the journal live for everything else.
        Err(error) if is_event_rejected(&error) => {
            let refused = format!("event batch commit failed: {error:#}");
            let mut retry = Vec::new();
            let mut storage_failed = false;
            // Replies are held until the batch's outcome is decided: a durable
            // caller must not observe its failure before the journal health
            // that explains it is published, or it can read the health as
            // healthy while holding the error of the batch that made it
            // unhealthy.
            struct Deferred {
                reply: crossbeam_channel::Sender<Result<(), AppendFailure>>,
                outcome: Result<(), AppendFailure>,
            }
            let mut replies: Vec<Deferred> = Vec::new();
            for mut item in pending.drain(..) {
                let event_id = item
                    .event
                    .get("event_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                let event_type = item
                    .event
                    .get("event_type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                // A canonical conversation's raw append refuses every event
                // (it has no raw journal), so the retry has to keep going
                // through the semantic writer it just used — otherwise a single
                // refusal would drop the whole batch instead of one event.
                let appended = canonical.persist(store, session_id, std::iter::once(&item.event));
                match appended {
                    Ok(()) => {
                        #[cfg(test)]
                        commits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if let Some(reply) = item.reply {
                            replies.push(Deferred {
                                reply,
                                outcome: Ok(()),
                            });
                        }
                    }
                    Err(error) if is_event_rejected(&error) => {
                        tracing::warn!(
                            session = %session_id,
                            %event_id,
                            %event_type,
                            %error,
                            "dropping an event this journal refuses; events after it still land"
                        );
                        if let Some(reply) = item.reply {
                            replies.push(Deferred {
                                reply,
                                outcome: Err(AppendFailure::Rejected(format!(
                                    "event batch commit failed: {error:#}"
                                ))),
                            });
                        }
                    }
                    // Still a storage problem: keep it for the next attempt.
                    Err(error) => {
                        storage_failed = true;
                        let unavailable = AppendFailure::Unavailable(format!(
                            "event batch commit failed: {error:#}"
                        ));
                        match item.reply.take() {
                            Some(reply) => replies.push(Deferred {
                                reply,
                                outcome: Err(unavailable),
                            }),
                            None => retry.push(item),
                        }
                    }
                }
            }
            *pending = retry;
            if storage_failed {
                health.fail(refused);
            }
            for Deferred { reply, outcome } in replies {
                let _ = reply.send(outcome);
            }
            !storage_failed
        }
        Err(error) => {
            let message = format!("event batch commit failed: {error:#}");
            health.fail(message.clone());
            let mut retry = Vec::new();
            for mut item in pending.drain(..) {
                if let Some(reply) = item.reply.take() {
                    let _ = reply.send(Err(AppendFailure::Unavailable(message.clone())));
                }
                retry.push(item);
            }
            *pending = retry;
            false
        }
    }
}
