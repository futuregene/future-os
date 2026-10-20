//! Shared task ownership and bounded cleanup. Entry points install the runtime.
use std::sync::OnceLock;

static HANDLE: OnceLock<tokio::runtime::Handle> = OnceLock::new();

/// Install the entry point's process-lifetime runtime before starting services.
/// Repeated GUI adapter calls retain the first installed handle.
pub fn set(handle: tokio::runtime::Handle) {
    let _ = HANDLE.set(handle);
}

pub type TaskHandle = tokio::task::JoinHandle<()>;

fn spawn_untracked<F>(future: F) -> TaskHandle
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    #[cfg(test)]
    let handle = HANDLE.get_or_init(|| {
        // Unit fixtures need tasks to survive individual test runtimes.
        static RUNTIME: std::sync::LazyLock<tokio::runtime::Runtime> =
            std::sync::LazyLock::new(|| tokio::runtime::Runtime::new().expect("test runtime"));
        RUNTIME.handle().clone()
    });
    #[cfg(not(test))]
    let handle = HANDLE
        .get()
        .expect("entry point must install the host runtime");
    handle.spawn(future)
}

/// Freeze registration before shutdown and wait for the actual future drops.
/// This also lets test fixtures safely switch HOME after cancelling observers.
type RegisteredTask = (
    futures::future::AbortHandle,
    std::sync::mpsc::Receiver<()>,
    &'static std::panic::Location<'static>,
);

#[derive(Default)]
struct Registry {
    closed: bool,
    tasks: Vec<RegisteredTask>,
    errors: Vec<String>,
}
impl Registry {
    fn prune_finished(&mut self) {
        self.tasks
            .retain(|(_, completed, _)| completed.try_recv().is_err());
    }
}
static TASKS: std::sync::LazyLock<std::sync::Mutex<Registry>> =
    std::sync::LazyLock::new(Default::default);

pub fn freeze_tasks() {
    TASKS.lock().unwrap_or_else(|p| p.into_inner()).closed = true;
}
pub fn tasks_frozen() -> bool {
    TASKS.lock().unwrap_or_else(|p| p.into_inner()).closed
}
pub fn record_cleanup_error(error: String) {
    TASKS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .errors
        .push(error);
}

struct TaskCompletion(std::sync::mpsc::Sender<()>);

impl Drop for TaskCompletion {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

fn tracked_future<F>(
    future: F,
    registration: futures::future::AbortRegistration,
    done: std::sync::mpsc::Sender<()>,
) -> impl std::future::Future<Output = ()> + Send
where
    F: std::future::Future<Output = ()> + Send,
{
    // Own the guard before the first poll. Aborting an unpolled task must also
    // signal completion, not just disconnect the receiver used by HOME cleanup.
    let completion = TaskCompletion(done);
    async move {
        let _completion = completion;
        let _ = futures::future::Abortable::new(future, registration).await;
    }
}

#[track_caller]
pub fn spawn<F>(future: F) -> TaskHandle
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let (abort, registration) = futures::future::AbortHandle::new_pair();
    let (done, completed) = std::sync::mpsc::channel();
    let mut registry = TASKS.lock().unwrap_or_else(|p| p.into_inner());
    if registry.closed {
        drop(registry);
        drop(future);
        return spawn_untracked(async {});
    }
    registry.prune_finished();
    let task = spawn_untracked(tracked_future(future, registration, done));
    registry
        .tasks
        .push((abort, completed, std::panic::Location::caller()));
    task
}

/// Stop every process-lifetime test task and wait until its future has been
/// dropped. Call this before changing HOME: otherwise a task created by an
/// earlier test can resolve the newly published HOME and lock its database.
#[cfg(any(test, feature = "test-support"))]
pub fn cancel_test_tasks() {
    cancel_all_tasks(std::time::Duration::from_secs(5))
        .expect("test tasks must stop before changing HOME");
    *TASKS.lock().unwrap_or_else(|p| p.into_inner()) = Registry::default();
}

pub fn cancel_all_tasks(timeout: std::time::Duration) -> Result<(), crate::Error> {
    let tasks = {
        let mut registry = TASKS.lock().unwrap_or_else(|p| p.into_inner());
        registry.closed = true;
        std::mem::take(&mut registry.tasks)
    };
    for (abort, _, _) in &tasks {
        abort.abort();
    }
    let deadline = std::time::Instant::now() + timeout;
    for (_, completed, origin) in tasks {
        if completed
            .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            .is_err()
        {
            let error = format!("Desktop background work did not stop before cleanup deadline (registered at {origin})");
            record_cleanup_error(error.clone());
            return Err(error.into());
        }
    }
    let registry = TASKS.lock().unwrap_or_else(|p| p.into_inner());
    if registry.errors.is_empty() {
        Ok(())
    } else {
        Err(registry.errors.join("; ").into())
    }
}

/// Native startup/maintenance jobs cannot be aborted as Rust futures. Keep
/// their real completion in the same registry so reset refuses to race them.
#[track_caller]
pub fn spawn_native(job: impl FnOnce() + Send + 'static) {
    let (abort, _registration) = futures::future::AbortHandle::new_pair();
    let (done, completed) = std::sync::mpsc::channel();
    let completion = TaskCompletion(done);
    let mut registry = TASKS.lock().unwrap_or_else(|p| p.into_inner());
    if registry.closed {
        return;
    }
    registry.prune_finished();
    registry
        .tasks
        .push((abort, completed, std::panic::Location::caller()));
    std::thread::spawn(move || {
        let _completion = completion;
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).is_err() {
            record_cleanup_error("Desktop native background job panicked".into());
        }
    });
}

#[cfg(test)]
mod tests {
    use crate::TEST_HOME_LOCK as TEST_LOCK;
    struct RegistryGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
    }
    impl RegistryGuard {
        fn new() -> Self {
            let guard = Self {
                _lock: TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner()),
            };
            super::cancel_test_tasks();
            guard
        }
    }
    impl Drop for RegistryGuard {
        fn drop(&mut self) {
            super::cancel_test_tasks();
        }
    }

    #[test]
    fn unpolled_task_drop_still_reports_completion() {
        let (_abort, registration) = futures::future::AbortHandle::new_pair();
        let (done, completed) = std::sync::mpsc::channel();
        let future = super::tracked_future(std::future::pending::<()>(), registration, done);
        drop(future);
        completed
            .try_recv()
            .expect("unpolled task must report completion");
    }

    #[test]
    fn cleanup_waits_for_process_lifetime_tasks() {
        let _lock = TEST_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let _task = super::spawn(std::future::pending::<()>());
        super::cancel_test_tasks();
    }

    #[test]
    fn frozen_registry_never_polls_late_work_or_starts_native_jobs() {
        let _registry = RegistryGuard::new();
        super::freeze_tasks();
        let (tx, rx) = std::sync::mpsc::channel();
        let native = tx.clone();
        let _task = super::spawn(async move {
            tx.send(()).unwrap();
        });
        super::spawn_native(move || {
            native.send(()).unwrap();
        });
        super::cancel_all_tasks(std::time::Duration::from_secs(1)).unwrap();
        assert!(rx
            .recv_timeout(std::time::Duration::from_millis(30))
            .is_err());
    }

    #[test]
    fn cleanup_waits_for_native_jobs_and_reports_persistence_failure() {
        let _registry = RegistryGuard::new();
        let (tx, rx) = std::sync::mpsc::channel();
        super::spawn_native(move || {
            std::thread::sleep(std::time::Duration::from_millis(30));
            tx.send(()).unwrap();
        });
        super::cancel_all_tasks(std::time::Duration::from_secs(1)).unwrap();
        rx.try_recv().unwrap();
        super::record_cleanup_error("ledger failed".into());
        assert!(super::cancel_all_tasks(std::time::Duration::from_secs(1))
            .unwrap_err()
            .to_string()
            .contains("ledger failed"));
        // This injected failure belongs to this fixture only.
        super::TASKS.lock().unwrap().errors.clear();
    }

    #[test]
    fn background_task_outlives_its_callers_runtime() {
        // Other fixtures cancel the shared registry while holding this lock.
        let _lock = TEST_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let caller = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (release, wait) = tokio::sync::oneshot::channel();
        let (done, completed) = std::sync::mpsc::channel();
        let context = caller.enter();
        let task = super::spawn(async move {
            wait.await.unwrap();
            done.send(()).unwrap();
        });
        drop(context);
        drop(caller);
        release.send(()).unwrap();
        completed
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        task.abort();
        super::cancel_test_tasks();
    }

    /// A native job is a plain thread and cannot be aborted, so a job that outlives
    /// the cleanup budget must fail the barrier *and* say which work held it —
    /// "stopped" published over still-running work is the failure this guards.
    #[test]
    fn a_native_job_that_outlives_the_budget_fails_the_cleanup_barrier() {
        let _registry = RegistryGuard::new();
        let (finished, job_done) = std::sync::mpsc::channel();
        super::spawn_native(move || {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let _ = finished.send(());
        });

        let error = super::cancel_all_tasks(std::time::Duration::from_millis(5))
            .expect_err("an overrunning native job cannot report a clean stop")
            .to_string();
        assert!(
            error.contains("did not stop before cleanup deadline"),
            "{error}"
        );
        // Let the job finish before the registry is reused by a sibling test, and
        // drop this fixture's injected failure: the shared registry would otherwise
        // report it at the next test's cleanup barrier.
        job_done
            .recv_timeout(std::time::Duration::from_secs(3))
            .expect("the native job finishes");
        super::TASKS.lock().unwrap().errors.clear();
    }

    /// A panicking native job is caught where it runs, but it must still surface:
    /// the cleanup result carries it, so a maintenance step that died is never
    /// reported as a successful stop.
    #[test]
    fn a_panicking_native_job_is_reported_at_the_barrier() {
        let _registry = RegistryGuard::new();
        super::spawn_native(|| panic!("injected native cleanup panic"));

        let error = super::cancel_all_tasks(std::time::Duration::from_secs(3))
            .expect_err("a panicked native job cannot report a clean stop")
            .to_string();
        assert!(
            error.contains("Desktop native background job panicked"),
            "{error}"
        );
        // This injected failure belongs to this fixture only.
        super::TASKS.lock().unwrap().errors.clear();
    }
}
