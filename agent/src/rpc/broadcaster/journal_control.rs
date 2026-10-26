//! A bounded control lane, separate from body admission. A barrier carries the
//! last admitted append sequence, so priority never lets it overtake that body.
use std::sync::Arc;

type ReplyResult = Result<(), String>;
pub(super) type AsyncControlReceipt = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<ReplyResult, std::sync::mpsc::RecvError>> + Send>,
>;

pub(super) struct WorkerReceipt<T> {
    reply: crossbeam_channel::Receiver<T>,
    exited: crossbeam_channel::Receiver<()>,
}
pub(super) type ControlReceipt = WorkerReceipt<ReplyResult>;
impl<T> WorkerReceipt<T> {
    pub(super) fn recv(&self) -> Result<T, std::sync::mpsc::RecvError> {
        // Preserve an acknowledged commit even if shutdown completed at the
        // same time. An orphaned queued command must never keep this waiting.
        crossbeam_channel::select_biased! {
            recv(self.reply) -> result => result.map_err(|_| std::sync::mpsc::RecvError),
            recv(self.exited) -> _ => Err(std::sync::mpsc::RecvError),
        }
    }
}

pub(super) struct WorkerExit {
    sync: Option<crossbeam_channel::Sender<()>>,
    asynchronous: tokio::sync::watch::Sender<bool>,
}
impl Drop for WorkerExit {
    fn drop(&mut self) {
        let _ = self.asynchronous.send(true);
        self.sync.take();
    }
}

pub(super) enum ControlReply {
    Sync(crossbeam_channel::Sender<Result<(), String>>),
    Async(tokio::sync::oneshot::Sender<Result<(), String>>),
}

impl ControlReply {
    pub(super) fn send(self, result: Result<(), String>) {
        match self {
            Self::Sync(reply) => {
                let _ = reply.send(result);
            }
            Self::Async(reply) => {
                let _ = reply.send(result);
            }
        }
    }
}

pub(super) struct ControlCommand {
    pub target: u64,
    pub shutdown: bool,
    pub reply: ControlReply,
}

#[derive(Clone)]
pub(super) struct JournalControl {
    pub sender: crossbeam_channel::Sender<ControlCommand>,
    /// Serializes successful body admission with barrier capture. The worker
    /// never takes this lock, including while doing disk I/O.
    pub admitted: Arc<parking_lot::Mutex<u64>>,
    exited: crossbeam_channel::Receiver<()>,
    async_exited: tokio::sync::watch::Receiver<bool>,
}

impl JournalControl {
    pub(super) fn receipt<T>(&self, reply: crossbeam_channel::Receiver<T>) -> WorkerReceipt<T> {
        WorkerReceipt {
            reply,
            exited: self.exited.clone(),
        }
    }
    pub(super) fn new(sender: crossbeam_channel::Sender<ControlCommand>) -> (Self, WorkerExit) {
        let (sync, exited) = crossbeam_channel::bounded(0);
        let (asynchronous, async_exited) = tokio::sync::watch::channel(false);
        (
            Self {
                sender,
                admitted: Default::default(),
                exited,
                async_exited,
            },
            WorkerExit {
                sync: Some(sync),
                asynchronous,
            },
        )
    }

    pub(super) fn queue_flush(&self) -> anyhow::Result<ControlReceipt> {
        self.queue_sync(false)
    }

    pub(super) fn queue_shutdown(&self) -> anyhow::Result<ControlReceipt> {
        self.queue_sync(true)
    }

    fn queue_sync(&self, shutdown: bool) -> anyhow::Result<ControlReceipt> {
        let target = *self.admitted.lock();
        let (reply, receiver) = crossbeam_channel::bounded(1);
        // Sync lifecycle callers wait outside broadcaster locks. A full body
        // queue cannot consume any of these reserved command slots.
        self.sender
            .send(ControlCommand {
                target,
                shutdown,
                reply: ControlReply::Sync(reply),
            })
            .map_err(|_| anyhow::anyhow!("event persistence worker is unavailable"))?;
        Ok(self.receipt(receiver))
    }

    pub(super) fn queue_async_flush(&self) -> anyhow::Result<AsyncControlReceipt> {
        let target = *self.admitted.lock();
        let (reply, receiver) = tokio::sync::oneshot::channel();
        self.sender
            .try_send(ControlCommand {
                target,
                shutdown: false,
                reply: ControlReply::Async(reply),
            })
            .map_err(|error| {
                anyhow::anyhow!(match error {
                    crossbeam_channel::TrySendError::Full(_) =>
                        "event persistence control queue is full",
                    crossbeam_channel::TrySendError::Disconnected(_) =>
                        "event persistence worker is unavailable",
                })
            })?;
        let mut exited = self.async_exited.clone();
        Ok(Box::pin(async move {
            tokio::select! {
                biased;
                result = receiver => result.map_err(|_| std::sync::mpsc::RecvError),
                _ = async {
                    let already_exited = *exited.borrow();
                    if !already_exited { let _ = exited.changed().await; }
                } => Err(std::sync::mpsc::RecvError),
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_orphaned_shutdown_reply_cannot_hold_a_sync_waiter() {
        let (sender, receiver) = crossbeam_channel::bounded(1);
        let (control, worker_exit) = JournalControl::new(sender);
        let reply = control.queue_shutdown().unwrap();
        drop(receiver); // crossbeam keeps queued messages while sender exists
        drop(worker_exit);
        assert!(reply.recv().is_err());
    }

    #[test]
    fn a_committed_ack_wins_over_concurrent_worker_exit() {
        let (sender, receiver) = crossbeam_channel::bounded(1);
        let (control, worker_exit) = JournalControl::new(sender);
        let reply = control.queue_flush().unwrap();
        receiver.recv().unwrap().reply.send(Ok(()));
        drop(receiver);
        drop(worker_exit);
        assert_eq!(reply.recv().unwrap(), Ok(()));
    }

    #[test]
    fn an_orphaned_body_reply_cannot_hold_a_waiter() {
        let (sender, _receiver) = crossbeam_channel::bounded(1);
        let (control, worker_exit) = JournalControl::new(sender);
        let (body_sender, body_receiver) = crossbeam_channel::bounded::<Result<(), u8>>(1);
        let reply = control.receipt(body_receiver);
        drop(worker_exit);
        assert!(reply.recv().is_err());
        // Keep the orphaned sender alive through the assertion: the worker
        // exit, not reply-channel disconnection, must release the waiter.
        drop(body_sender);
    }

    #[tokio::test]
    async fn orphaned_async_fences_end_when_the_actual_worker_exits() {
        let (sender, receiver) = crossbeam_channel::bounded(1);
        let (control, worker_exit) = JournalControl::new(sender);
        let reply = control.queue_async_flush().unwrap();
        drop(receiver);
        drop(worker_exit);
        assert!(reply.await.is_err());
    }

    #[tokio::test]
    async fn asynchronous_ack_is_preserved_after_exit() {
        let (sender, receiver) = crossbeam_channel::bounded(1);
        let (control, worker_exit) = JournalControl::new(sender);
        let reply = control.queue_async_flush().unwrap();
        receiver.recv().unwrap().reply.send(Ok(()));
        drop(receiver);
        drop(worker_exit);
        assert_eq!(reply.await.unwrap(), Ok(()));
    }
}
