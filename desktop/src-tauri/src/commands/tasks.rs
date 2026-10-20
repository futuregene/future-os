//! Task IPC adaptation for the shared task application services.
use future_host::tasks::service::{DepView, RevisionView, RunView, TaskInput, TaskView};

#[tauri::command]
pub fn list_tasks() -> Result<Vec<TaskView>, crate::AppError> {
    future_host::tasks::service::list_tasks()
}

#[tauri::command]
pub fn create_task(input: TaskInput) -> Result<TaskView, crate::AppError> {
    future_host::tasks::service::create_task(input)
}

#[tauri::command]
pub fn update_task(id: String, input: TaskInput) -> Result<TaskView, crate::AppError> {
    future_host::tasks::service::update_task(id, input)
}

#[tauri::command]
pub fn delete_task(id: String) -> Result<(), crate::AppError> {
    future_host::tasks::service::delete_task(id)
}

#[tauri::command]
pub fn set_task_enabled(id: String, enabled: bool) -> Result<TaskView, crate::AppError> {
    future_host::tasks::service::set_task_enabled(id, enabled)
}

#[tauri::command]
pub fn run_task_now(id: String) -> Result<TaskView, crate::AppError> {
    future_host::tasks::service::run_task_now(id)
}

#[tauri::command]
pub fn list_task_runs(id: String, limit: Option<i64>) -> Result<Vec<RunView>, crate::AppError> {
    future_host::tasks::service::list_task_runs(id, limit)
}

#[tauri::command]
pub fn list_task_deps(_id: String) -> Result<Vec<DepView>, crate::AppError> {
    future_host::tasks::service::list_task_deps(_id)
}

#[tauri::command]
pub fn set_task_dep(
    _id: String,
    _upstream_task_id: String,
    _on: Option<String>,
) -> Result<(), crate::AppError> {
    future_host::tasks::service::set_task_dep(_id, _upstream_task_id, _on)
}

#[tauri::command]
pub fn remove_task_dep(_id: String, _upstream_task_id: String) -> Result<(), crate::AppError> {
    future_host::tasks::service::remove_task_dep(_id, _upstream_task_id)
}

#[tauri::command]
pub fn list_task_revisions(id: String) -> Result<Vec<RevisionView>, crate::AppError> {
    future_host::tasks::service::list_task_revisions(id)
}

#[tauri::command]
pub fn apply_task_revision(id: String, revision_id: String) -> Result<TaskView, crate::AppError> {
    future_host::tasks::service::apply_task_revision(id, revision_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn async_command_wrappers_reject_malformed_bodies() {
        crate::commands::ipc_harness::assert_all_reject_bad_body(
            tauri::generate_handler![
                create_task,
                update_task,
                delete_task,
                set_task_enabled,
                run_task_now,
                list_task_runs,
                list_task_deps,
                set_task_dep,
                remove_task_dep,
                list_task_revisions,
                apply_task_revision
            ],
            &[
                "create_task",
                "update_task",
                "delete_task",
                "set_task_enabled",
                "run_task_now",
                "list_task_runs",
                "list_task_deps",
                "set_task_dep",
                "remove_task_dep",
                "list_task_revisions",
                "apply_task_revision",
            ],
        );
        // `update_task` takes two arguments, so fail the *second* one to reach the
        // error arm the empty body does not (see the harness doc).
        crate::commands::ipc_harness::assert_all_reject_bodies(
            tauri::generate_handler![update_task],
            &[("update_task", serde_json::json!({ "id": "tsk_1" }))],
        );
    }
}
