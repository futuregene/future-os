//! Shared process-lifetime task runtime, including synchronous store entry points.
#[cfg(all(feature = "gui", test))]
pub(crate) use tauri::async_runtime::JoinHandle;
#[cfg(feature = "gui")]
pub(crate) use tauri::async_runtime::{set, spawn};

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

#[cfg(not(feature = "gui"))]
pub(crate) fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
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

#[cfg(test)]
mod tests {
    #[test]
    fn background_task_outlives_its_callers_runtime() {
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
        drop(task);
    }
}
