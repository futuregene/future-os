use super::test_support::{MemoryHost, TokioExecutor};
use super::*;
use crate::types::RemotePhase;
use std::time::Duration;

#[derive(Default)]
pub(super) struct QueueExecutor {
    futures: Mutex<Vec<BoxFuture<'static, ()>>>,
    pub(super) native: Mutex<Vec<Box<dyn FnOnce() + Send + 'static>>>,
    reject: AtomicBool,
}
impl TaskExecutor for QueueExecutor {
    fn spawn(&self, future: BoxFuture<'static, ()>) {
        if !self.reject.load(Ordering::Acquire) {
            self.futures.lock().unwrap().push(future);
        }
    }
    fn spawn_native(&self, job: Box<dyn FnOnce() + Send + 'static>) {
        self.native.lock().unwrap().push(job);
    }
}
impl QueueExecutor {
    pub(super) fn run_native_reverse(&self) {
        let mut jobs = std::mem::take(&mut *self.native.lock().unwrap());
        while let Some(job) = jobs.pop() {
            job();
        }
    }
}
struct Dropped(Arc<AtomicBool>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[tokio::test]
async fn supervisor_instances_keep_control_recovery_and_presence_separate() {
    let first_host = Arc::new(MemoryHost::default());
    let second_host = Arc::new(MemoryHost::default());
    let first = Supervisor::new(first_host.clone(), Arc::new(TokioExecutor));
    let second = Supervisor::new(second_host.clone(), Arc::new(TokioExecutor));
    first.remember_device("pair_first");
    second.remember_device("pair_second");
    first.claim_control("pair_first");
    assert_eq!(
        first_host.active.lock().unwrap().as_deref(),
        Some("pair_first")
    );
    assert!(second_host.active.lock().unwrap().is_none());
    assert!(first.in_control("pair_first"));
    assert!(!second.in_control("pair_first"));
    first.start_requested.store(true, Ordering::Release);
    first.start_retry_running.store(true, Ordering::Release);
    first.start_retry_attempts.store(3, Ordering::Release);
    *first.last_error_code.lock().unwrap() = Some("network".into());
    let a = first.project_status(None, false).status;
    let b = second.project_status(None, false).status;
    assert_eq!(a.phase, RemotePhase::Reconnecting);
    assert_eq!(a.recovery.unwrap().attempt, 3);
    assert_eq!(b.phase, RemotePhase::Stopped);
    assert!(b.recovery.is_none());
    assert_eq!(
        first.light_presence_payload("pair_first", "bridge_first")["control"],
        "active"
    );
    assert_eq!(
        second.light_presence_payload("pair_first", "bridge_second")["control"],
        "standby"
    );
}

#[tokio::test]
async fn queued_control_writes_and_late_claims_cannot_reverse_stop() {
    let host = Arc::new(MemoryHost::default());
    let executor = Arc::new(QueueExecutor::default());
    let supervisor = Supervisor::new(host.clone(), executor.clone());
    let epoch = supervisor.access.current();
    supervisor.claim_control_for("pair_first", Some(epoch));
    assert_eq!(supervisor.active_pair().as_deref(), Some("pair_first"));
    supervisor.access.invalidate(|| {
        supervisor.reset_devices();
        supervisor.cancel_tasks();
    });
    supervisor.claim_control_for("pair_late", Some(epoch));
    assert!(supervisor.active_pair().is_none());
    assert_eq!(executor.native.lock().unwrap().len(), 1);
    assert!(supervisor
        .wait_tasks(Duration::from_millis(10))
        .await
        .is_err());
    assert_eq!(
        supervisor.native_tasks.lock().unwrap().len(),
        1,
        "timeout retains unfinished ownership"
    );
    executor.run_native_reverse();
    supervisor.wait_tasks(Duration::from_secs(1)).await.unwrap();
    assert!(host.active.lock().unwrap().is_none());
}

#[tokio::test]
async fn a_newer_takeover_wins_even_when_native_jobs_run_out_of_order() {
    let host = Arc::new(MemoryHost::default());
    let executor = Arc::new(QueueExecutor::default());
    let supervisor = Supervisor::new(host.clone(), executor.clone());
    supervisor.remember_device("pair_first");
    supervisor.remember_device("pair_second");
    let epoch = supervisor.access.current();
    supervisor.claim_control_for("pair_first", Some(epoch));
    supervisor.claim_control_for("pair_second", Some(epoch));
    executor.run_native_reverse();
    supervisor.wait_tasks(Duration::from_secs(1)).await.unwrap();
    assert_eq!(host.active.lock().unwrap().as_deref(), Some("pair_second"));
    assert!(supervisor.in_control("pair_second"));
    assert!(!supervisor.in_control("pair_first"));
}

#[tokio::test]
async fn cleanup_waits_for_the_running_future_to_drop() {
    let supervisor = Supervisor::new(Arc::new(MemoryHost::default()), Arc::new(TokioExecutor));
    supervisor.start_requested.store(true, Ordering::Release);
    let dropped = Arc::new(AtomicBool::new(false));
    let guard = Dropped(dropped.clone());
    let (entered, observed) = tokio::sync::oneshot::channel();
    supervisor.spawn(async move {
        let _guard = guard;
        let _ = entered.send(());
        std::future::pending::<()>().await;
    });
    observed.await.unwrap();
    supervisor.start_requested.store(false, Ordering::Release);
    supervisor.cancel_tasks();
    supervisor.wait_tasks(Duration::from_secs(1)).await.unwrap();
    assert!(dropped.load(Ordering::Acquire));
}

#[tokio::test]
async fn rejected_unpolled_work_and_timeout_retries_keep_real_completion() {
    let executor = Arc::new(QueueExecutor::default());
    let supervisor = Supervisor::new(Arc::new(MemoryHost::default()), executor.clone());
    supervisor.start_requested.store(true, Ordering::Release);
    let rejected = Arc::new(AtomicBool::new(false));
    let guard = Dropped(rejected.clone());
    executor.reject.store(true, Ordering::Release);
    supervisor.spawn(async move {
        let _guard = guard;
        std::future::pending::<()>().await;
    });
    assert!(rejected.load(Ordering::Acquire));
    supervisor.cancel_tasks();
    supervisor.wait_tasks(Duration::from_secs(1)).await.unwrap();
    executor.reject.store(false, Ordering::Release);
    let held = Arc::new(AtomicBool::new(false));
    let guard = Dropped(held.clone());
    supervisor.spawn(async move {
        let _guard = guard;
        std::future::pending::<()>().await;
    });
    supervisor.cancel_tasks();
    assert!(supervisor
        .wait_tasks(Duration::from_millis(10))
        .await
        .is_err());
    assert!(!held.load(Ordering::Acquire));
    executor.futures.lock().unwrap().clear();
    supervisor.wait_tasks(Duration::from_secs(1)).await.unwrap();
    assert!(held.load(Ordering::Acquire));
}
