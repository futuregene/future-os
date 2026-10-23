//! Admission and work ownership shared by every client of one Agent instance.
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::sync::watch;

#[derive(Default)]
struct WorkState {
    closed: bool,
    retired: bool,
    revision: u64,
    commands: usize,
    background: usize,
    auxiliary: usize,
}

pub struct ServiceWork {
    state: Mutex<WorkState>,
    streams: watch::Sender<bool>,
    listener: watch::Sender<bool>,
}

impl Default for ServiceWork {
    fn default() -> Self {
        Self {
            state: Mutex::new(WorkState::default()),
            streams: watch::channel(false).0,
            listener: watch::channel(false).0,
        }
    }
}

impl ServiceWork {
    pub(crate) fn enter(self: &Arc<Self>) -> anyhow::Result<WorkGuard> {
        self.register(WorkKind::Command)
    }

    pub(crate) fn background(self: &Arc<Self>) -> anyhow::Result<WorkGuard> {
        self.register(WorkKind::Background)
    }

    // An already owned run may still need a blocking finalizer after admission
    // closes. Its child must remain counted even if its parent panics/aborts.
    fn child(self: &Arc<Self>, kind: WorkKind) -> Option<WorkGuard> {
        let mut state = self.state.lock();
        if state.retired {
            return None;
        }
        state.revision = state.revision.wrapping_add(1);
        *kind.count(&mut state) += 1;
        Some(WorkGuard {
            work: self.clone(),
            kind,
        })
    }

    fn register(self: &Arc<Self>, kind: WorkKind) -> anyhow::Result<WorkGuard> {
        let mut state = self.state.lock();
        anyhow::ensure!(!state.closed, "Agent service is stopping");
        state.revision = state.revision.wrapping_add(1);
        *kind.count(&mut state) += 1;
        Ok(WorkGuard {
            work: self.clone(),
            kind,
        })
    }

    pub(crate) fn close(&self) {
        // The same mutex linearizes admission and the shutdown boundary.
        let mut state = self.state.lock();
        state.closed = true;
        state.revision = state.revision.wrapping_add(1);
    }

    pub(crate) fn close_if_revision(&self, revision: u64) -> bool {
        let mut state = self.state.lock();
        if state.closed || state.revision != revision {
            return false;
        }
        state.closed = true;
        state.revision = state.revision.wrapping_add(1);
        true
    }

    pub(crate) fn retire(&self) -> anyhow::Result<()> {
        let mut state = self.state.lock();
        anyhow::ensure!(
            state.closed && state.commands == 0 && state.background == 0 && state.auxiliary == 0,
            "Agent work is not drained; instance ownership retained"
        );
        state.retired = true;
        Ok(())
    }

    pub(crate) fn is_retired(&self) -> bool {
        self.state.lock().retired
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.state.lock().closed
    }

    pub(crate) fn revision(&self) -> u64 {
        self.state.lock().revision
    }

    pub(crate) fn counts(&self) -> (usize, usize) {
        let state = self.state.lock();
        (state.commands, state.background + state.auxiliary)
    }

    pub(crate) fn activity_counts(&self) -> (usize, usize) {
        let state = self.state.lock();
        (state.commands, state.background)
    }

    pub(crate) fn close_streams(&self) {
        self.streams.send_replace(true);
    }
    pub(crate) fn stop_listener(&self) {
        self.listener.send_replace(true);
    }
    pub(crate) fn stream_shutdown(&self) -> impl std::future::Future<Output = ()> + Send + 'static {
        wait_closed(self.streams.subscribe())
    }
    pub(crate) fn listener_shutdown(
        &self,
    ) -> impl std::future::Future<Output = ()> + Send + 'static {
        wait_closed(self.listener.subscribe())
    }
}

async fn wait_closed(mut receiver: watch::Receiver<bool>) {
    while !*receiver.borrow_and_update() {
        if receiver.changed().await.is_err() {
            break;
        }
    }
}

enum WorkKind {
    Command,
    Background,
    Auxiliary,
}
impl WorkKind {
    fn count<'a>(&self, state: &'a mut WorkState) -> &'a mut usize {
        match self {
            Self::Command => &mut state.commands,
            Self::Background => &mut state.background,
            Self::Auxiliary => &mut state.auxiliary,
        }
    }
}

pub(crate) struct WorkGuard {
    work: Arc<ServiceWork>,
    kind: WorkKind,
}
impl Drop for WorkGuard {
    fn drop(&mut self) {
        let mut state = self.work.state.lock();
        state.revision = state.revision.wrapping_add(1);
        *self.kind.count(&mut state) -= 1;
    }
}

// Registry refreshes are also triggered by sessionless configuration helpers.
// One scoped instance is allowed in a process; a weak reference avoids owning
// the service, and the registration guard follows a detached native thread.
struct ActiveService {
    work: std::sync::Weak<ServiceWork>,
    home: std::path::PathBuf,
}
static ACTIVE: Mutex<Option<ActiveService>> = Mutex::new(None);

pub(crate) fn install(work: &Arc<ServiceWork>, home: &std::path::Path) -> anyhow::Result<()> {
    let mut active = ACTIVE.lock();
    if let Some(previous) = active.as_ref() {
        if let Some(work) = previous.work.upgrade() {
            anyhow::ensure!(work.state.lock().retired, "another Agent service still owns this process; restart is forbidden until it stops");
        }
        // Catalog/config helpers retain process caches. A clean restart may
        // reuse the same home; switching roots requires a new host process.
        anyhow::ensure!(
            previous.home == home,
            "Agent data root cannot change within this process"
        );
    }
    *active = Some(ActiveService {
        work: Arc::downgrade(work),
        home: home.to_path_buf(),
    });
    Ok(())
}

pub(crate) fn process_background() -> anyhow::Result<Option<WorkGuard>> {
    match ACTIVE
        .lock()
        .as_ref()
        .and_then(|active| active.work.upgrade())
    {
        Some(work) => work.background().map(Some),
        None => Ok(None),
    }
}

pub(crate) fn process_child_work() -> Option<WorkGuard> {
    ACTIVE
        .lock()
        .as_ref()
        .and_then(|active| active.work.upgrade())
        .and_then(|work| work.child(WorkKind::Background))
}

pub(crate) fn process_auxiliary_work() -> Option<WorkGuard> {
    ACTIVE
        .lock()
        .as_ref()
        .and_then(|active| active.work.upgrade())
        .and_then(|work| work.child(WorkKind::Auxiliary))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_idle_update_reservation_refuses_a_changed_admission_revision() {
        let work = Arc::new(ServiceWork::default());
        let before = work.revision();
        let command = work.enter().unwrap();
        assert!(!work.close_if_revision(before));
        assert!(!work.is_closed());
        drop(command);
        assert!(work.close_if_revision(work.revision()));
        assert!(work.enter().is_err());
    }

    #[test]
    fn accepted_work_remains_owned_after_admission_closes() {
        let work = Arc::new(ServiceWork::default());
        let command = work.enter().unwrap();
        let native = work.background().unwrap();
        work.close();
        assert!(work.enter().is_err());
        assert!(work.background().is_err());
        assert_eq!(work.counts(), (1, 1));
        drop(command);
        drop(native);
        assert_eq!(work.counts(), (0, 0));
    }

    #[test]
    fn idle_workers_are_owned_without_making_user_activity_busy() {
        let work = Arc::new(ServiceWork::default());
        let scheduler = work.child(WorkKind::Auxiliary).unwrap();
        work.close();
        let finalizer = work.child(WorkKind::Background).unwrap();
        assert_eq!(work.activity_counts(), (0, 1));
        assert_eq!(work.counts(), (0, 2));
        drop(finalizer);
        assert_eq!(work.activity_counts(), (0, 0));
        assert_eq!(work.counts(), (0, 1));
        drop(scheduler);
        assert_eq!(work.counts(), (0, 0));
    }

    #[test]
    fn retirement_excludes_late_children_and_requires_every_guard_to_finish() {
        let work = Arc::new(ServiceWork::default());
        assert!(work.retire().is_err());
        let child = work.child(WorkKind::Background).unwrap();
        work.close();
        assert!(work.retire().is_err());
        drop(child);
        work.retire().unwrap();
        assert!(work.is_retired());
        assert!(work.child(WorkKind::Background).is_none());
        assert!(work.child(WorkKind::Auxiliary).is_none());
        assert!(work.background().is_err());
    }

    #[tokio::test]
    async fn late_stream_waiter_observes_shutdown_without_lost_notification() {
        let work = ServiceWork::default();
        work.close_streams();
        tokio::time::timeout(std::time::Duration::from_secs(1), work.stream_shutdown())
            .await
            .unwrap();
    }
}

#[cfg(test)]
mod listener_shutdown_tests {
    use super::*;

    /// The shutdown futures resolve when their watch flag flips — and also when
    /// the owner disappears first (a dropped `ServiceWork` drops the sender), so
    /// a host that abandons the instance never waits forever on a signal that
    /// can no longer arrive.
    #[test]
    fn a_shutdown_future_resolves_when_its_owner_is_dropped() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let work = ServiceWork::default();
            let mut streams = std::pin::pin!(work.stream_shutdown());
            let mut listener = std::pin::pin!(work.listener_shutdown());
            // Nothing has been signalled yet, so neither future can be ready.
            assert!(
                futures::FutureExt::now_or_never(streams.as_mut()).is_none(),
                "an unsignalled shutdown future must wait"
            );
            assert!(futures::FutureExt::now_or_never(listener.as_mut()).is_none());

            // The owner goes away: both senders drop, and each future resolves
            // instead of hanging on a signal that can never come.
            drop(work);
            streams.await;
            listener.await;

            // The explicit signal path also resolves (the happy path the service
            // uses), so the drop arm above is not the only way out.
            let work = ServiceWork::default();
            work.close_streams();
            work.stream_shutdown().await;
        });
    }
}
