//! Ordered event batches, durable flush barriers and persistence health.
use super::BROADCAST_RING_CAPACITY;
use crate::session::sqlite_store::is_event_rejected;
pub(super) type AppendReceipt = super::journal_control::WorkerReceipt<Result<(), AppendFailure>>;

pub(super) const EVENT_BATCH_WINDOW: std::time::Duration = std::time::Duration::from_millis(100);
const EVENT_BATCH_MAX_EVENTS: usize = 128;
const EVENT_BATCH_MAX_BYTES: usize = 64 * 1024;
const EVENT_BATCH_QUEUE_CAPACITY: usize = BROADCAST_RING_CAPACITY;

#[derive(Default)]
pub(super) struct EventJournalHealth {
    last_error: parking_lot::Mutex<Option<String>>,
    pub(super) metrics: super::journal_metrics::JournalMetrics,
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
            .map_err(|_| anyhow::anyhow!("event shutdown acknowledgement lost"))?
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
    let mut canonical = match store.storage_format(&session_id) {
        Ok(crate::session::content::FORMAT_VERSION) => {
            Some(crate::session::content::journal::Writer::default())
        }
        Ok(_) => None,
        Err(error) => {
            health.fail(format!("cannot determine journal format: {error:#}"));
            return;
        }
    };
    let window = if canonical.is_some() {
        std::time::Duration::from_millis(25)
    } else {
        EVENT_BATCH_WINDOW
    };
    let mut pending = Vec::<PendingEvent>::new();
    let mut pending_bytes = 0usize;
    let mut deadline = None::<std::time::Instant>;
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
            let ok = flush_event_batch(
                &store,
                &session_id,
                &health,
                &mut pending,
                &mut canonical,
                #[cfg(test)]
                &commits,
            );
            failed = !ok;
            pending_bytes = pending.iter().map(|item| item.bytes).sum();
            deadline = (!pending.is_empty() && !failed).then(|| std::time::Instant::now() + window);
            command.reply.send(if ok {
                Ok(())
            } else {
                Err(health
                    .error()
                    .unwrap_or_else(|| "event persistence is unavailable".into()))
            });
            if command.shutdown {
                break;
            }
            continue;
        }
        let timer = if !pending.is_empty() && !failed {
            crossbeam_channel::after(
                deadline
                    .unwrap_or_else(std::time::Instant::now)
                    .saturating_duration_since(std::time::Instant::now()),
            )
        } else {
            crossbeam_channel::never()
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
            failed = !flush_event_batch(
                &store,
                &session_id,
                &health,
                &mut pending,
                &mut canonical,
                #[cfg(test)]
                &commits,
            );
            pending_bytes = pending.iter().map(|item| item.bytes).sum();
            deadline = (!pending.is_empty() && !failed).then(|| std::time::Instant::now() + window);
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
                deadline.get_or_insert_with(|| std::time::Instant::now() + window);
                if durable
                    || pending.len() >= EVENT_BATCH_MAX_EVENTS
                    || pending_bytes >= EVENT_BATCH_MAX_BYTES
                {
                    failed = !flush_event_batch(
                        &store,
                        &session_id,
                        &health,
                        &mut pending,
                        &mut canonical,
                        #[cfg(test)]
                        &commits,
                    );
                    pending_bytes = pending.iter().map(|item| item.bytes).sum();
                    deadline = (!pending.is_empty() && !failed)
                        .then(|| std::time::Instant::now() + window);
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

fn flush_event_batch(
    store: &crate::session::sqlite_store::SqliteStore,
    session_id: &str,
    health: &EventJournalHealth,
    pending: &mut Vec<PendingEvent>,
    canonical: &mut Option<crate::session::content::journal::Writer>,
    #[cfg(test)] commits: &std::sync::atomic::AtomicU64,
) -> bool {
    if pending.is_empty() {
        return true;
    }
    let started = std::time::Instant::now();
    let result = match canonical.as_mut() {
        Some(writer) => writer.persist(store, session_id, pending.iter().map(|item| &item.event)),
        None => store.append_events(
            session_id,
            pending.iter().map(|item| item.event.clone()).collect(),
        ),
    };
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
