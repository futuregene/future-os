//! GUI/legacy entry wiring for the common host task registry.
pub(crate) use future_host::runtime::{
    cancel_all_tasks, freeze_tasks, set, spawn_native, TaskHandle,
};

#[cfg(feature = "gui")]
pub(crate) use future_host::runtime::{record_cleanup_error, tasks_frozen};

#[track_caller]
pub(crate) fn spawn<F>(future: F) -> TaskHandle
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    #[cfg(feature = "gui")]
    set(tauri::async_runtime::handle().inner().clone());
    #[cfg(test)]
    install_test_runtime();
    future_host::runtime::spawn(future)
}

#[cfg(test)]
pub(crate) fn cancel_test_tasks() {
    future_host::runtime::cancel_test_tasks();
    crate::desktop_shutdown::reset_for_test();
}

/// Retained entry fixtures install their own ambient runtime before Host calls.
#[cfg(test)]
pub(crate) fn install_test_runtime() {
    #[cfg(feature = "gui")]
    set(tauri::async_runtime::handle().inner().clone());
    #[cfg(not(feature = "gui"))]
    {
        static RUNTIME: std::sync::LazyLock<tokio::runtime::Runtime> =
            std::sync::LazyLock::new(|| {
                tokio::runtime::Runtime::new().expect("test entry runtime")
            });
        set(RUNTIME.handle().clone());
    }
}
