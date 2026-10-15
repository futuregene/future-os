//! Shared process-lifetime task runtime, including synchronous store entry points.
//
// The re-exports mirror the feature split in `now`: the gui build forwards
// tauri's runtime, the headless build owns a tokio one.
#[cfg(all(feature = "gui", test))]
pub(crate) use tauri::async_runtime::set;
#[cfg(all(feature = "gui", not(test)))]
pub(crate) use tauri::async_runtime::set;

#[cfg(not(feature = "gui"))]
pub(crate) use tokio::task::JoinHandle;

#[cfg(not(feature = "gui"))]
static HANDLE: std::sync::OnceLock<tokio::runtime::Handle> = std::sync::OnceLock::new();

#[cfg(not(feature = "gui"))]
pub(crate) fn set(handle: tokio::runtime::Handle) {
    HANDLE
        .set(handle)
        .expect("Desktop runtime already initialized");
}

/// The handle a task spawned through [`spawn`] comes back as.
///
/// It differs by feature, so callers outside this module name it rather than
/// the two concrete types — a caller that had to know would need the feature
/// split duplicated at every spawn site.
#[cfg(feature = "gui")]
pub(crate) type TaskHandle = tauri::async_runtime::JoinHandle<()>;
#[cfg(not(feature = "gui"))]
pub(crate) type TaskHandle = JoinHandle<()>;

/// Whether a task spawned through [`spawn`] has finished.
///
/// The one thing the two handle types spell differently, so it lives here with
/// the type alias rather than at each call site. Test-only, because nothing in
/// production asks a task whether it is done.
#[cfg(test)]
pub(crate) fn task_finished(handle: &TaskHandle) -> bool {
    #[cfg(feature = "gui")]
    {
        handle.inner().is_finished()
    }
    #[cfg(not(feature = "gui"))]
    {
        handle.is_finished()
    }
}

fn spawn_untracked<F>(future: F) -> TaskHandle
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    #[cfg(feature = "gui")]
    {
        tauri::async_runtime::spawn(future)
    }
    #[cfg(not(feature = "gui"))]
    {
        // Never bind long-lived observers to an incidental caller's runtime.
        // Synchronous test cleanup waits for those observers; using a test's
        // current-thread runtime here would deadlock that cleanup.
        static FALLBACK: std::sync::LazyLock<tokio::runtime::Runtime> =
            std::sync::LazyLock::new(|| {
                tokio::runtime::Runtime::new().expect("create Desktop task runtime")
            });
        HANDLE
            .get_or_init(|| FALLBACK.handle().clone())
            .spawn(future)
    }
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

pub(crate) fn freeze_tasks() {
    TASKS.lock().unwrap_or_else(|p| p.into_inner()).closed = true;
}
#[cfg(feature = "gui")]
pub(crate) fn tasks_frozen() -> bool {
    TASKS.lock().unwrap_or_else(|p| p.into_inner()).closed
}
pub(crate) fn record_cleanup_error(error: String) {
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
pub(crate) fn spawn<F>(future: F) -> TaskHandle
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
#[cfg(test)]
pub(crate) fn cancel_test_tasks() {
    cancel_all_tasks(std::time::Duration::from_secs(5))
        .expect("test tasks must stop before changing HOME");
    *TASKS.lock().unwrap_or_else(|p| p.into_inner()) = Registry::default();
    crate::desktop_shutdown::reset_for_test();
}

pub(crate) fn cancel_all_tasks(timeout: std::time::Duration) -> Result<(), crate::AppError> {
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
pub(crate) fn spawn_native(job: impl FnOnce() + Send + 'static) {
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
        let _lock = crate::TEST_HOME_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let _task = super::spawn(std::future::pending::<()>());
        super::cancel_test_tasks();
    }

    #[test]
    fn frozen_registry_never_polls_late_work_or_starts_native_jobs() {
        let _home = crate::auth_store::test_support::HomeGuard::new("runtime-freeze");
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
        let _home = crate::auth_store::test_support::HomeGuard::new("runtime-native");
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
        let _lock = crate::TEST_HOME_LOCK
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
}
