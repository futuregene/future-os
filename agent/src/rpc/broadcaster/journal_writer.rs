//! Ordered event batches, durable flush barriers and persistence health.
use super::BROADCAST_RING_CAPACITY;
use crate::session::sqlite_store::is_event_rejected;

pub(super) const EVENT_BATCH_WINDOW: std::time::Duration = std::time::Duration::from_millis(100);
const EVENT_BATCH_MAX_EVENTS: usize = 128;
const EVENT_BATCH_MAX_BYTES: usize = 64 * 1024;
const EVENT_BATCH_QUEUE_CAPACITY: usize = BROADCAST_RING_CAPACITY;

#[derive(Default)]
pub(super) struct EventJournalHealth {
    last_error: parking_lot::Mutex<Option<String>>,
    interrupt_flag: parking_lot::Mutex<Option<std::sync::Arc<std::sync::atomic::AtomicBool>>>,
}

impl EventJournalHealth {
    pub(super) fn error(&self) -> Option<String> {
        self.last_error.lock().clone()
    }

    pub(super) fn clear(&self) {
        *self.last_error.lock() = None;
    }

    pub(super) fn fail(&self, message: String) {
        *self.last_error.lock() = Some(message);
        if let Some(flag) = self.interrupt_flag.lock().as_ref() {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    pub(super) fn set_interrupt(&self, flag: std::sync::Arc<std::sync::atomic::AtomicBool>) {
        if self.error().is_some() {
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
        event: serde_json::Value,
        reply: Option<std::sync::mpsc::SyncSender<Result<(), AppendFailure>>>,
    },
    Flush(std::sync::mpsc::SyncSender<Result<(), String>>),
    Shutdown(std::sync::mpsc::SyncSender<Result<(), String>>),
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
    reply: Option<std::sync::mpsc::SyncSender<Result<(), AppendFailure>>>,
}

pub(super) struct EventBatchWriter {
    pub(super) sender: std::sync::mpsc::SyncSender<EventWriteCommand>,
    pub(super) thread: Option<std::thread::JoinHandle<()>>,
    pub(super) health: std::sync::Arc<EventJournalHealth>,
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
            std::sync::mpsc::sync_channel::<EventWriteCommand>(EVENT_BATCH_QUEUE_CAPACITY);
        #[cfg(test)]
        let commits = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        #[cfg(test)]
        let worker_commits = commits.clone();
        let worker_health = health.clone();
        let thread = std::thread::Builder::new()
            .name("agent-event-batch".into())
            .spawn(move || {
                run_event_batch_writer(
                    receiver,
                    store,
                    session_id,
                    worker_health,
                    #[cfg(test)]
                    worker_commits,
                );
            })?;
        Ok(Self {
            sender,
            thread: Some(thread),
            health,
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
            let (sender, receiver) = std::sync::mpsc::sync_channel(1);
            (Some(sender), Some(receiver))
        } else {
            (None, None)
        };
        // The bounded queue applies backpressure, not data loss. A transient
        // full queue is not a storage failure: wait for the dedicated writer
        // (which never acquires broadcaster/session locks) to make space.
        // As at a durable boundary, actual writer failure remains fail-closed.
        self.sender
            .send(EventWriteCommand::Append { event, reply })
            .map_err(|_| {
                let message = "event persistence worker is unavailable".to_string();
                self.health.fail(message.clone());
                AppendFailure::Unavailable(message)
            })?;
        if let Some(receiver) = receiver {
            receiver.recv().map_err(|_| {
                AppendFailure::unavailable("event persistence worker is unavailable")
            })?
        } else {
            Ok(())
        }
    }

    pub(super) fn queue_flush(
        sender: &std::sync::mpsc::SyncSender<EventWriteCommand>,
    ) -> anyhow::Result<std::sync::mpsc::Receiver<Result<(), String>>> {
        let (reply, receiver) = std::sync::mpsc::sync_channel(1);
        sender
            .send(EventWriteCommand::Flush(reply))
            .map_err(|_| anyhow::anyhow!("event persistence worker is unavailable"))?;
        Ok(receiver)
    }

    pub(super) fn shutdown(&mut self) -> anyhow::Result<()> {
        if self.thread.is_none() {
            return Ok(());
        }
        let (reply, receiver) = std::sync::mpsc::sync_channel(1);
        self.sender
            .send(EventWriteCommand::Shutdown(reply))
            .map_err(|_| anyhow::anyhow!("event persistence worker unavailable at shutdown"))?;
        let result = receiver
            .recv()
            .map_err(|_| anyhow::anyhow!("event shutdown acknowledgement lost"))?
            .map_err(anyhow::Error::msg);
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| anyhow::anyhow!("event persistence worker panicked"))?;
        }
        result
    }

    pub(super) fn flush(&self) -> anyhow::Result<()> {
        Self::queue_flush(&self.sender)?
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
        let (dead, receiver) = std::sync::mpsc::sync_channel(1);
        drop(receiver);
        let sender = std::mem::replace(&mut self.sender, dead);
        drop(sender);
    }

    /// Test-only: retire the worker through its real `Shutdown` handshake and
    /// join the thread, so "the worker is gone" is an observed fact rather
    /// than a sleep-and-hope wait. A later `append` must then fail closed.
    #[cfg(test)]
    pub(super) fn stop_worker_for_test(&mut self) {
        let (reply, receiver) = std::sync::mpsc::sync_channel(1);
        if self.sender.send(EventWriteCommand::Shutdown(reply)).is_ok() {
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
        let (reply, receiver) = std::sync::mpsc::sync_channel(1);
        if self.sender.send(EventWriteCommand::Shutdown(reply)).is_ok() {
            let _ = receiver.recv();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run_event_batch_writer(
    receiver: std::sync::mpsc::Receiver<EventWriteCommand>,
    store: crate::session::sqlite_store::SqliteStore,
    session_id: String,
    health: std::sync::Arc<EventJournalHealth>,
    #[cfg(test)] commits: std::sync::Arc<std::sync::atomic::AtomicU64>,
) {
    let mut pending = Vec::<PendingEvent>::new();
    let mut pending_bytes = 0usize;
    let mut deadline = None::<std::time::Instant>;
    let mut failed = false;
    loop {
        let command = if pending.is_empty() || failed {
            match receiver.recv() {
                Ok(command) => command,
                Err(_) => break,
            }
        } else {
            let remaining = deadline
                .unwrap_or_else(std::time::Instant::now)
                .saturating_duration_since(std::time::Instant::now());
            match receiver.recv_timeout(remaining) {
                Ok(command) => command,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    failed = !flush_event_batch(
                        &store,
                        &session_id,
                        &health,
                        &mut pending,
                        #[cfg(test)]
                        &commits,
                    );
                    pending_bytes = pending
                        .iter()
                        .map(|item| item.event.to_string().len())
                        .sum();
                    deadline = (!pending.is_empty() && !failed)
                        .then(|| std::time::Instant::now() + EVENT_BATCH_WINDOW);
                    continue;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        };

        match command {
            EventWriteCommand::Append { event, reply } => {
                if failed && reply.is_some() {
                    if let Some(reply) = reply {
                        // Reached only when a storage failure is still blocking
                        // the journal (`failed`), so it is an availability
                        // problem, not a refusal of this event.
                        let _ = reply.send(Err(AppendFailure::Unavailable(
                            health
                                .error()
                                .unwrap_or_else(|| "event persistence is unavailable".to_string()),
                        )));
                    }
                    continue;
                }
                pending_bytes = pending_bytes.saturating_add(event.to_string().len());
                let durable = reply.is_some();
                pending.push(PendingEvent { event, reply });
                deadline.get_or_insert_with(|| std::time::Instant::now() + EVENT_BATCH_WINDOW);
                if durable
                    || pending.len() >= EVENT_BATCH_MAX_EVENTS
                    || pending_bytes >= EVENT_BATCH_MAX_BYTES
                {
                    failed = !flush_event_batch(
                        &store,
                        &session_id,
                        &health,
                        &mut pending,
                        #[cfg(test)]
                        &commits,
                    );
                    pending_bytes = pending
                        .iter()
                        .map(|item| item.event.to_string().len())
                        .sum();
                    deadline = (!pending.is_empty() && !failed)
                        .then(|| std::time::Instant::now() + EVENT_BATCH_WINDOW);
                }
            }
            EventWriteCommand::Flush(reply) => {
                let ok = flush_event_batch(
                    &store,
                    &session_id,
                    &health,
                    &mut pending,
                    #[cfg(test)]
                    &commits,
                );
                failed = !ok;
                pending_bytes = pending
                    .iter()
                    .map(|item| item.event.to_string().len())
                    .sum();
                deadline = (!pending.is_empty() && !failed)
                    .then(|| std::time::Instant::now() + EVENT_BATCH_WINDOW);
                let result = if ok {
                    Ok(())
                } else {
                    Err(health
                        .error()
                        .unwrap_or_else(|| "event persistence is unavailable".to_string()))
                };
                let _ = reply.send(result);
            }
            EventWriteCommand::Shutdown(reply) => {
                let ok = flush_event_batch(
                    &store,
                    &session_id,
                    &health,
                    &mut pending,
                    #[cfg(test)]
                    &commits,
                );
                let result = if ok {
                    Ok(())
                } else {
                    Err(health
                        .error()
                        .unwrap_or_else(|| "event persistence is unavailable".to_string()))
                };
                let _ = reply.send(result);
                break;
            }
            #[cfg(test)]
            EventWriteCommand::Hold(gate) => {
                gate.wait(); // arrived inside the command loop
                gate.wait(); // released by the test
            }
        }
    }
}

fn flush_event_batch(
    store: &crate::session::sqlite_store::SqliteStore,
    session_id: &str,
    health: &EventJournalHealth,
    pending: &mut Vec<PendingEvent>,
    #[cfg(test)] commits: &std::sync::atomic::AtomicU64,
) -> bool {
    if pending.is_empty() {
        return true;
    }
    let events = pending
        .iter()
        .map(|item| item.event.clone())
        .collect::<Vec<_>>();
    match store.append_events(session_id, events) {
        Ok(()) => {
            #[cfg(test)]
            commits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
                reply: std::sync::mpsc::SyncSender<Result<(), AppendFailure>>,
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
                match store.append_events(session_id, vec![item.event.clone()]) {
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
            for item in pending.drain(..) {
                if let Some(reply) = item.reply {
                    let _ = reply.send(Err(AppendFailure::Unavailable(message.clone())));
                } else {
                    retry.push(item);
                }
            }
            *pending = retry;
            false
        }
    }
}
