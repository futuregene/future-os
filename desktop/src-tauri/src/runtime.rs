//! Shared process-lifetime task runtime, including synchronous store entry points.
#[cfg(all(feature = "gui", test))]
pub(crate) use tauri::async_runtime::{set, JoinHandle};
#[cfg(all(feature = "gui", not(test)))]
pub(crate) use tauri::async_runtime::{set, JoinHandle};

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

fn spawn_untracked<F>(future: F) -> JoinHandle<()>
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

#[cfg(not(test))]
pub(crate) fn spawn<F>(future: F) -> JoinHandle<()>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    spawn_untracked(future)
}

/// Process-lifetime tasks outlive the `#[tokio::test]` runtime that started
/// them. Keep their cancellation handles in one registry so a test fixture can
/// prove none still touches the process-global HOME or SQLite pool before it
/// switches to another fixture.
#[cfg(test)]
type TestTask = (futures::future::AbortHandle, std::sync::mpsc::Receiver<()>);

#[cfg(test)]
static TEST_TASKS: std::sync::LazyLock<std::sync::Mutex<Vec<TestTask>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

#[cfg(test)]
struct TestTaskCompletion(std::sync::mpsc::Sender<()>);

#[cfg(test)]
impl Drop for TestTaskCompletion {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

#[cfg(test)]
fn tracked_test_future<F>(
    future: F,
    registration: futures::future::AbortRegistration,
    done: std::sync::mpsc::Sender<()>,
) -> impl std::future::Future<Output = ()> + Send
where
    F: std::future::Future<Output = ()> + Send,
{
    // Own the guard before the first poll. Aborting an unpolled task must also
    // signal completion, not just disconnect the receiver used by HOME cleanup.
    let completion = TestTaskCompletion(done);
    async move {
        let _completion = completion;
        let _ = futures::future::Abortable::new(future, registration).await;
    }
}

#[cfg(test)]
pub(crate) fn spawn<F>(future: F) -> JoinHandle<()>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let (abort, registration) = futures::future::AbortHandle::new_pair();
    let (done, completed) = std::sync::mpsc::channel();
    let task = spawn_untracked(tracked_test_future(future, registration, done));
    TEST_TASKS
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .push((abort, completed));
    task
}

/// Stop every process-lifetime test task and wait until its future has been
/// dropped. Call this before changing HOME: otherwise a task created by an
/// earlier test can resolve the newly published HOME and lock its database.
#[cfg(test)]
pub(crate) fn cancel_test_tasks() {
    let tasks = std::mem::take(
        &mut *TEST_TASKS
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()),
    );
    for (abort, _) in &tasks {
        abort.abort();
    }
    for (_, completed) in tasks {
        completed
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("process-lifetime test task must stop before changing HOME");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn unpolled_task_drop_still_reports_completion() {
        let (_abort, registration) = futures::future::AbortHandle::new_pair();
        let (done, completed) = std::sync::mpsc::channel();
        let future = super::tracked_test_future(std::future::pending::<()>(), registration, done);
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
