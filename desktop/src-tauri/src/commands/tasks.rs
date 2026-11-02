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

    fn task_input(name: &str, prompt: &str) -> TaskInput {
        TaskInput {
            name: name.into(),
            prompt: prompt.into(),
            cwd: std::env::temp_dir().display().to_string(),
            model_id: Some("future/gpt-5".into()),
            thinking_level: Some("high".into()),
            timezone: Some("Asia/Shanghai".into()),
            session_policy: Some("existing".into()),
            conversation_mode: Some("chat".into()),
            session_retention: None,
            trigger_kind: Some("manual".into()),
            trigger: Some(serde_json::json!({ "mode": "manual" })),
            enabled: None,
        }
    }

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

    /// The task screen's whole command surface against one private store: every
    /// assertion is on a value the *service* derived from the argument the
    /// wrapper passed, so an argument dropped on the way through shows up as the
    /// default instead (a task with no timezone, an unchanged prompt, an
    /// un-queued run). The two dependency commands are deliberately refusals —
    /// that refusal is the contract, and it must not be swallowed here.
    #[test]
    fn every_task_command_forwards_its_arguments_to_the_service() {
        let _home = crate::auth_store::test_support::HomeGuard::new("cmd_tasks_wave");
        assert!(list_tasks().expect("empty list").is_empty());

        let created = create_task(task_input("daily", "first prompt")).expect("create");
        assert_eq!(created.name, "daily");
        assert_eq!(created.prompt, "first prompt");
        assert_eq!(created.timezone, "Asia/Shanghai", "the zone was forwarded");
        assert_eq!(created.conversation_mode, "chat");
        assert_eq!(created.session_policy, "existing");
        assert_eq!(created.trigger_kind, "manual");
        assert!(created.enabled, "a new task is enabled");
        assert_eq!(created.prompt_version, 1);

        let listed = list_tasks().expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, created.id);

        let updated =
            update_task(created.id.clone(), task_input("daily", "second prompt")).expect("update");
        assert_eq!(
            updated.id, created.id,
            "the edited row is the requested one"
        );
        assert_eq!(updated.prompt, "second prompt");
        assert_eq!(updated.prompt_version, 2, "the edit is a new revision");

        let disabled = set_task_enabled(created.id.clone(), false).expect("disable");
        assert!(!disabled.enabled, "the flag reached the row");

        let queued = run_task_now(created.id.clone()).expect("run now");
        assert!(
            queued.queued,
            "the explicit run request is waiting for a tick"
        );

        assert!(
            list_task_runs(created.id.clone(), None)
                .expect("runs")
                .is_empty(),
            "a manual trigger has not produced a run yet"
        );

        assert!(
            list_task_deps(created.id.clone()).expect("deps").is_empty(),
            "task dependencies are gone, so there are never any edges"
        );
        let added = set_task_dep(created.id.clone(), "other".into(), Some("success".into()));
        let added_error = added.expect_err("adding a dependency is refused");
        assert!(added_error.to_string().contains("removed"), "{added_error}");
        let removed_error = remove_task_dep(created.id.clone(), "other".into())
            .expect_err("removing a dependency is refused");
        assert!(
            removed_error.to_string().contains("no edges"),
            "{removed_error}"
        );

        let revisions = list_task_revisions(created.id.clone()).expect("revisions");
        let first = revisions
            .iter()
            .find(|revision| revision.version == 1)
            .expect("the original prompt is kept as a revision");
        assert_eq!(first.prompt, "first prompt");
        let applied =
            apply_task_revision(created.id.clone(), first.id.clone()).expect("apply revision");
        assert_eq!(
            applied.prompt, "first prompt",
            "the requested revision, not the newest one, is applied"
        );

        delete_task(created.id.clone()).expect("delete");
        assert!(
            list_tasks().expect("list after delete").is_empty(),
            "a deleted task is no longer listed"
        );

        // A ghost id is an error, not a silent success: the panel reports it.
        assert!(delete_task("tsk_absent".into()).is_err());
    }
}
