//! Tauri commands for FutureOS tasks (list/create/update/delete/run/runs).
//!
//! These are thin wrappers over the `future-tasks` store so the webview can
//! manage tasks; execution stays in `crate::tasks`'s tick loop.

use future_tasks::{
    DepJoin, DepOn, Reflection, RunOrigin, SessionPolicy, Store, Task, TriggerKind,
};
use serde::{Deserialize, Serialize};

/// FutureOS home root, matching the tick loop / CLI resolution.
fn future_home() -> std::path::PathBuf {
    if let Some(override_dir) = future_rpc::home::future_home_override() {
        return override_dir;
    }
    future_app_settings::app_dir()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::env::temp_dir().join(".future"))
}

fn open() -> Result<Store, crate::AppError> {
    Store::open(&future_home()).map_err(|e| crate::AppError::Message(e.to_string()))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// A task as the webview sees it (camelCase, trigger flattened for the form).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskView {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub prompt: String,
    pub prompt_version: i64,
    pub cwd: String,
    pub model_id: Option<String>,
    pub thinking_level: Option<String>,
    pub session_policy: String,
    pub trigger_kind: String,
    pub trigger: serde_json::Value,
    pub dep_join: String,
    pub next_due_at: Option<i64>,
    pub last_run_at: Option<i64>,
    pub reflection: String,
    pub latest_run: Option<RunView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunView {
    pub id: String,
    pub kind: String,
    pub origin: String,
    pub status: String,
    pub due_at: Option<i64>,
    pub thread_id: Option<String>,
    pub session_id: Option<String>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub prompt_version: Option<i64>,
    pub result_summary: Option<String>,
    pub error_message: Option<String>,
}

fn run_view(run: future_tasks::TaskRun) -> RunView {
    RunView {
        id: run.id,
        kind: format!("{:?}", run.kind).to_lowercase(),
        origin: format!("{:?}", run.origin).to_lowercase(),
        status: format!("{:?}", run.status).to_lowercase(),
        due_at: run.due_at,
        thread_id: run.thread_id,
        session_id: run.session_id,
        started_at: run.started_at,
        finished_at: run.finished_at,
        prompt_version: run.prompt_version,
        result_summary: run.result_summary,
        error_message: run.error_message,
    }
}

fn task_view(store: &Store, task: Task) -> TaskView {
    let latest_run = store
        .latest_run_for_task(&task.id)
        .ok()
        .flatten()
        .map(run_view);
    TaskView {
        id: task.id,
        name: task.name,
        enabled: task.enabled,
        prompt: task.prompt,
        prompt_version: task.prompt_version,
        cwd: task.cwd,
        model_id: task.model_id,
        thinking_level: task.thinking_level,
        session_policy: format!("{:?}", task.session_policy).to_lowercase(),
        trigger_kind: format!("{:?}", task.trigger_kind).to_lowercase(),
        trigger: task.trigger_json,
        dep_join: format!("{:?}", task.dep_join).to_lowercase(),
        next_due_at: task.next_due_at,
        last_run_at: task.last_run_at,
        reflection: format!("{:?}", task.reflection).to_lowercase(),
        latest_run,
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskInput {
    pub name: String,
    pub prompt: String,
    pub cwd: String,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub thinking_level: Option<String>,
    #[serde(default)]
    pub session_policy: Option<String>,
    #[serde(default)]
    pub reflection: Option<String>,
    #[serde(default)]
    pub trigger_kind: Option<String>,
    #[serde(default)]
    pub trigger: Option<serde_json::Value>,
    #[serde(default)]
    pub dep_join: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

fn parse_session_policy(raw: Option<&str>) -> SessionPolicy {
    match raw {
        Some("existing") => SessionPolicy::Existing,
        _ => SessionPolicy::New,
    }
}

fn parse_reflection(raw: Option<&str>) -> Reflection {
    match raw {
        Some("off") => Reflection::Off,
        Some("auto") => Reflection::Auto,
        _ => Reflection::Ask,
    }
}

fn parse_trigger_kind(raw: Option<&str>) -> TriggerKind {
    match raw {
        Some("schedule") => TriggerKind::Schedule,
        _ => TriggerKind::Manual,
    }
}

fn parse_dep_join(raw: Option<&str>) -> DepJoin {
    match raw {
        Some("any") => DepJoin::Any,
        _ => DepJoin::All,
    }
}

#[tauri::command]
pub fn list_tasks() -> Result<Vec<TaskView>, crate::AppError> {
    let store = open()?;
    let tasks = store
        .list_tasks(false)
        .map_err(|e| crate::AppError::Message(e.to_string()))?;
    Ok(tasks.into_iter().map(|t| task_view(&store, t)).collect())
}

#[tauri::command]
pub fn create_task(input: TaskInput) -> Result<TaskView, crate::AppError> {
    let store = open()?;
    let now = now_ms();
    let trigger_kind = parse_trigger_kind(input.trigger_kind.as_deref());
    let trigger = input.trigger.unwrap_or(serde_json::json!({}));
    let mut task = Task {
        id: future_tasks::new_task_id(),
        name: input.name,
        enabled: input.enabled.unwrap_or(true),
        prompt: input.prompt,
        prompt_version: 1,
        cwd: input.cwd,
        model_id: input.model_id,
        thinking_level: input.thinking_level,
        session_policy: parse_session_policy(input.session_policy.as_deref()),
        thread_id: None,
        trigger_kind,
        trigger_json: trigger,
        dep_join: parse_dep_join(input.dep_join.as_deref()),
        next_due_at: None,
        last_run_at: None,
        pending_request_at: None,
        pending_origin: None,
        pending_actor: None,
        reflection: parse_reflection(input.reflection.as_deref()),
        created_at: now,
        updated_at: now,
        deleted_at: None,
    };
    if trigger_kind == TriggerKind::Schedule {
        task.next_due_at = future_tasks::next_due(&task, now - 1);
    }
    store
        .insert_task(&task)
        .map_err(|e| crate::AppError::Message(e.to_string()))?;
    Ok(task_view(&store, task))
}

#[tauri::command]
pub fn update_task(id: String, input: TaskInput) -> Result<TaskView, crate::AppError> {
    let store = open()?;
    let mut task = store
        .get_task(&id)
        .map_err(|e| crate::AppError::Message(e.to_string()))?
        .ok_or_else(|| crate::AppError::Message("task not found".to_string()))?;
    let trigger_kind = parse_trigger_kind(input.trigger_kind.as_deref());
    let trigger = input.trigger.unwrap_or_else(|| task.trigger_json.clone());
    let prompt_changed = input.prompt != task.prompt;
    task.name = input.name;
    task.prompt = input.prompt;
    task.cwd = input.cwd;
    task.model_id = input.model_id;
    task.thinking_level = input.thinking_level;
    task.session_policy = parse_session_policy(input.session_policy.as_deref());
    task.trigger_kind = trigger_kind;
    task.trigger_json = trigger;
    task.dep_join = parse_dep_join(input.dep_join.as_deref());
    task.reflection = parse_reflection(input.reflection.as_deref());
    if let Some(enabled) = input.enabled {
        task.enabled = enabled;
    }
    task.updated_at = now_ms();
    if trigger_kind == TriggerKind::Schedule {
        task.next_due_at = future_tasks::next_due(&task, now_ms() - 1);
    } else {
        task.next_due_at = None;
    }
    if prompt_changed {
        task.prompt_version += 1;
        let revision = future_tasks::PromptRevision {
            id: future_tasks::new_revision_id(),
            task_id: task.id.clone(),
            version: task.prompt_version,
            prompt: task.prompt.clone(),
            source: "user".to_string(),
            status: "active".to_string(),
            reason: None,
            confidence: None,
            source_run_id: None,
            created_at: now_ms(),
        };
        store
            .insert_revision(&revision)
            .map_err(|e| crate::AppError::Message(e.to_string()))?;
    }
    store
        .update_task(&task)
        .map_err(|e| crate::AppError::Message(e.to_string()))?;
    Ok(task_view(&store, task))
}

#[tauri::command]
pub fn delete_task(id: String) -> Result<(), crate::AppError> {
    let store = open()?;
    let mut task = store
        .get_task(&id)
        .map_err(|e| crate::AppError::Message(e.to_string()))?
        .ok_or_else(|| crate::AppError::Message("task not found".to_string()))?;
    task.deleted_at = Some(now_ms());
    task.enabled = false;
    task.updated_at = now_ms();
    store
        .update_task(&task)
        .map_err(|e| crate::AppError::Message(e.to_string()))
}

#[tauri::command]
pub fn set_task_enabled(id: String, enabled: bool) -> Result<TaskView, crate::AppError> {
    let store = open()?;
    let mut task = store
        .get_task(&id)
        .map_err(|e| crate::AppError::Message(e.to_string()))?
        .ok_or_else(|| crate::AppError::Message("task not found".to_string()))?;
    task.enabled = enabled;
    // Re-enabling a schedule must land on a real next slot. A task created while
    // disabled, or one whose schedule was edited while paused, can carry no due
    // time at all — enabling it without this would leave it permanently idle.
    if enabled && task.trigger_kind == TriggerKind::Schedule {
        task.next_due_at = future_tasks::next_due(&task, now_ms());
    }
    task.updated_at = now_ms();
    store
        .update_task(&task)
        .map_err(|e| crate::AppError::Message(e.to_string()))?;
    Ok(task_view(&store, task))
}

/// Queue an explicit run. The tick loop consumes `pending_request_at`.
#[tauri::command]
pub fn run_task_now(id: String) -> Result<TaskView, crate::AppError> {
    let store = open()?;
    let mut task = store
        .get_task(&id)
        .map_err(|e| crate::AppError::Message(e.to_string()))?
        .ok_or_else(|| crate::AppError::Message("task not found".to_string()))?;
    task.pending_request_at = Some(now_ms());
    task.pending_origin = Some(RunOrigin::Ui);
    task.pending_actor = Some("user".to_string());
    task.updated_at = now_ms();
    store
        .update_task(&task)
        .map_err(|e| crate::AppError::Message(e.to_string()))?;
    Ok(task_view(&store, task))
}

#[tauri::command]
pub fn list_task_runs(id: String, limit: Option<i64>) -> Result<Vec<RunView>, crate::AppError> {
    let store = open()?;
    let runs = store
        .list_runs_for_task(&id, limit.unwrap_or(20))
        .map_err(|e| crate::AppError::Message(e.to_string()))?;
    Ok(runs.into_iter().map(run_view).collect())
}

/// Dependency edges for a task (upstream id/name + condition), for the UI.
#[tauri::command]
pub fn list_task_deps(id: String) -> Result<Vec<DepView>, crate::AppError> {
    let store = open()?;
    let deps = store
        .list_deps(&id)
        .map_err(|e| crate::AppError::Message(e.to_string()))?;
    let mut out = Vec::new();
    for dep in deps {
        let upstream = store
            .get_task(&dep.upstream_task_id)
            .ok()
            .flatten()
            .map(|t| t.name)
            .unwrap_or_else(|| dep.upstream_task_id.clone());
        let satisfied = store
            .get_dep_state(&id, &dep.upstream_task_id)
            .ok()
            .flatten()
            .and_then(|s| s.satisfied_run_id);
        out.push(DepView {
            upstream_task_id: dep.upstream_task_id,
            upstream_name: upstream,
            on: format!("{:?}", dep.on).to_lowercase(),
            satisfied: satisfied.is_some(),
        });
    }
    Ok(out)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DepView {
    pub upstream_task_id: String,
    pub upstream_name: String,
    pub on: String,
    pub satisfied: bool,
}

#[tauri::command]
pub fn set_task_dep(
    id: String,
    upstream_task_id: String,
    on: Option<String>,
) -> Result<(), crate::AppError> {
    let store = open()?;
    let dep_on = match on.as_deref() {
        Some("failure") => DepOn::Failure,
        Some("completed") => DepOn::Completed,
        _ => DepOn::Success,
    };
    let dep = future_tasks::TaskDep {
        task_id: id.clone(),
        upstream_task_id,
        on: dep_on,
    };
    // Reject a cycle before persisting, using the full current graph.
    let mut all = store
        .list_all_deps()
        .map_err(|e| crate::AppError::Message(e.to_string()))?;
    all.retain(|d| !(d.task_id == dep.task_id && d.upstream_task_id == dep.upstream_task_id));
    all.push(dep.clone());
    if future_tasks::would_cycle(&dep.task_id, &all) {
        return Err(crate::AppError::Message(
            "dependency cycle detected".to_string(),
        ));
    }
    store
        .add_dep(&dep)
        .map_err(|e| crate::AppError::Message(e.to_string()))
}

#[tauri::command]
pub fn remove_task_dep(id: String, upstream_task_id: String) -> Result<(), crate::AppError> {
    let store = open()?;
    store
        .remove_dep(&id, &upstream_task_id)
        .map_err(|e| crate::AppError::Message(e.to_string()))
}

/// Prompt revisions for the version history / diff UI.
#[tauri::command]
pub fn list_task_revisions(id: String) -> Result<Vec<RevisionView>, crate::AppError> {
    let store = open()?;
    let revisions = store
        .list_revisions(&id)
        .map_err(|e| crate::AppError::Message(e.to_string()))?;
    Ok(revisions
        .into_iter()
        .map(|r| RevisionView {
            id: r.id,
            version: r.version,
            prompt: r.prompt,
            source: r.source,
            status: r.status,
            reason: r.reason,
            confidence: r.confidence,
            created_at: r.created_at,
        })
        .collect())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionView {
    pub id: String,
    pub version: i64,
    pub prompt: String,
    pub source: String,
    pub status: String,
    pub reason: Option<String>,
    pub confidence: Option<f64>,
    pub created_at: i64,
}

/// Apply a stored revision as the task's active prompt.
#[tauri::command]
pub fn apply_task_revision(id: String, revision_id: String) -> Result<TaskView, crate::AppError> {
    let store = open()?;
    let mut task = store
        .get_task(&id)
        .map_err(|e| crate::AppError::Message(e.to_string()))?
        .ok_or_else(|| crate::AppError::Message("task not found".to_string()))?;
    let revision = store
        .list_revisions(&id)
        .map_err(|e| crate::AppError::Message(e.to_string()))?
        .into_iter()
        .find(|r| r.id == revision_id)
        .ok_or_else(|| crate::AppError::Message("revision not found".to_string()))?;
    task.prompt = revision.prompt;
    task.prompt_version += 1;
    task.updated_at = now_ms();
    let applied = future_tasks::PromptRevision {
        id: future_tasks::new_revision_id(),
        task_id: task.id.clone(),
        version: task.prompt_version,
        prompt: task.prompt.clone(),
        source: "rollback".to_string(),
        status: "active".to_string(),
        reason: Some(format!("applied revision {}", revision.id)),
        confidence: None,
        source_run_id: None,
        created_at: now_ms(),
    };
    store
        .insert_revision(&applied)
        .map_err(|e| crate::AppError::Message(e.to_string()))?;
    store
        .update_task(&task)
        .map_err(|e| crate::AppError::Message(e.to_string()))?;
    Ok(task_view(&store, task))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]
    use super::*;
    use crate::auth_store::test_support::HomeGuard;

    /// A throwaway FutureOS home. The commands resolve the tasks store from it,
    /// so each test owns a private database.
    fn init(label: &str) -> HomeGuard {
        HomeGuard::new(label)
    }

    fn input(name: &str) -> TaskInput {
        TaskInput {
            name: name.into(),
            prompt: "do the thing".into(),
            cwd: "/tmp/repo".into(),
            model_id: Some("future/gpt-5".into()),
            thinking_level: Some("high".into()),
            session_policy: Some("existing".into()),
            reflection: Some("auto".into()),
            trigger_kind: Some("schedule".into()),
            trigger: Some(serde_json::json!({"mode": "daily", "time": "09:00"})),
            dep_join: Some("any".into()),
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

    #[test]
    fn a_task_round_trips_through_the_command_surface() {
        let _home = init("cmd_tasks_round_trip");

        let created = create_task(input("daily")).expect("create");
        assert_eq!(created.name, "daily");
        assert_eq!(created.session_policy, "existing");
        assert_eq!(created.reflection, "auto");
        assert_eq!(created.dep_join, "any");
        assert_eq!(created.trigger_kind, "schedule");
        assert_eq!(created.prompt_version, 1);
        assert!(
            created.next_due_at.is_some(),
            "a schedule must know its slot"
        );

        let listed = list_tasks().expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, created.id);

        let mut edited = input("renamed");
        edited.prompt = "a different prompt".into();
        edited.trigger_kind = Some("manual".into());
        edited.trigger = Some(serde_json::json!({}));
        let updated = update_task(created.id.clone(), edited).expect("update");
        assert_eq!(updated.name, "renamed");
        assert_eq!(updated.trigger_kind, "manual");
        assert_eq!(updated.prompt_version, 2, "editing the prompt bumps it");
        assert!(updated.next_due_at.is_none(), "manual work has no schedule");

        // The prompt edit is recorded as a revision the UI can list and apply.
        let revisions = list_task_revisions(created.id.clone()).expect("revisions");
        assert_eq!(revisions.len(), 1);
        assert_eq!(revisions[0].version, 2);
        assert_eq!(revisions[0].source, "user");

        delete_task(created.id.clone()).expect("delete");
        assert!(
            list_tasks().expect("list").is_empty(),
            "deleted tasks are hidden"
        );
    }

    #[test]
    fn an_unchanged_prompt_does_not_add_a_revision() {
        let _home = init("cmd_tasks_revision_skip");
        let created = create_task(input("stable")).expect("create");
        let again = update_task(created.id.clone(), input("stable")).expect("update");
        assert_eq!(again.prompt_version, 1);
        assert!(list_task_revisions(created.id)
            .expect("revisions")
            .is_empty());
    }

    #[test]
    fn apply_revision_promotes_a_stored_prompt() {
        let _home = init("cmd_tasks_apply_revision");
        let created = create_task(input("apply")).expect("create");
        let mut edited = input("apply");
        edited.prompt = "second".into();
        update_task(created.id.clone(), edited).expect("update");
        let revisions = list_task_revisions(created.id.clone()).expect("revisions");
        let first = revisions.first().expect("a revision").id.clone();

        let applied = apply_task_revision(created.id.clone(), first).expect("apply");
        assert_eq!(applied.prompt_version, 3, "applying adds a revision");
        let after = list_task_revisions(created.id).expect("revisions");
        assert_eq!(after.last().unwrap().source, "rollback");
        assert_eq!(
            after
                .last()
                .unwrap()
                .reason
                .as_deref()
                .map(|r| r.starts_with("applied revision")),
            Some(true)
        );
    }

    #[test]
    fn unknown_ids_are_reported_rather_than_ignored() {
        let _home = init("cmd_tasks_unknown");
        for error in [
            update_task("tsk_missing".into(), input("x")).unwrap_err(),
            delete_task("tsk_missing".into()).unwrap_err(),
            set_task_enabled("tsk_missing".into(), true).unwrap_err(),
            run_task_now("tsk_missing".into()).unwrap_err(),
            apply_task_revision("tsk_missing".into(), "rev_1".into()).unwrap_err(),
        ] {
            assert!(error.to_string().contains("task not found"), "{error}");
        }
        assert!(apply_task_revision("tsk_missing".into(), "rev_1".into()).is_err());
        // A revision id that does not exist on an existing task is refused too.
        let created = create_task(input("exists")).expect("create");
        let error = apply_task_revision(created.id.clone(), "rev_missing".into()).unwrap_err();
        assert!(error.to_string().contains("revision not found"), "{error}");
    }

    #[test]
    fn enabling_a_paused_schedule_lands_on_a_real_slot() {
        let _home = init("cmd_tasks_enable");
        let mut paused = input("paused");
        paused.enabled = Some(false);
        let created = create_task(paused).expect("create");
        assert!(!created.enabled);

        let enabled = set_task_enabled(created.id.clone(), true).expect("enable");
        assert!(enabled.enabled);
        assert!(
            enabled.next_due_at.is_some(),
            "enabling must schedule the next run, not leave the task idle"
        );

        let disabled = set_task_enabled(created.id, false).expect("disable");
        assert!(!disabled.enabled);
    }

    #[test]
    fn a_manual_task_has_no_schedule_to_recompute() {
        let _home = init("cmd_tasks_manual_enable");
        let mut manual = input("manual");
        manual.trigger_kind = Some("manual".into());
        manual.trigger = Some(serde_json::json!({}));
        manual.enabled = Some(false);
        let created = create_task(manual).expect("create");
        let enabled = set_task_enabled(created.id, true).expect("enable");
        assert!(enabled.enabled);
        assert!(enabled.next_due_at.is_none());
    }

    #[test]
    fn a_run_request_is_queued_for_the_tick_loop() {
        let _home = init("cmd_tasks_run_now");
        let created = create_task(input("queued")).expect("create");
        assert!(created.latest_run.is_none());

        let queued = run_task_now(created.id.clone()).expect("run now");
        assert!(queued.latest_run.is_none(), "the run has not started yet");

        // The queue is what the tick loop reads, so assert on it directly.
        let store = open().expect("store");
        let task = store.get_task(&created.id).unwrap().unwrap();
        assert!(task.pending_request_at.is_some());
        assert_eq!(task.pending_actor.as_deref(), Some("user"));
    }

    #[test]
    fn dependencies_are_set_listed_and_removed() {
        let _home = init("cmd_tasks_deps");
        let upstream = create_task(input("upstream")).expect("create upstream");
        let downstream = create_task(input("downstream")).expect("create downstream");
        assert!(list_task_deps(downstream.id.clone()).unwrap().is_empty());

        set_task_dep(
            downstream.id.clone(),
            upstream.id.clone(),
            Some("failure".into()),
        )
        .expect("set");
        let deps = list_task_deps(downstream.id.clone()).expect("list");
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].upstream_task_id, upstream.id);
        assert_eq!(deps[0].upstream_name, "upstream");
        assert_eq!(deps[0].on, "failure");
        assert!(!deps[0].satisfied);

        // Progress shows up once the edge is marked satisfied.
        let store = open().expect("store");
        store
            .mark_dep_satisfied(&downstream.id, &upstream.id, "trn_1", 1)
            .expect("mark");
        assert!(list_task_deps(downstream.id.clone()).unwrap()[0].satisfied);

        remove_task_dep(downstream.id.clone(), upstream.id.clone()).expect("remove");
        assert!(list_task_deps(downstream.id).unwrap().is_empty());
    }

    #[test]
    fn a_dependency_cycle_is_refused() {
        let _home = init("cmd_tasks_cycle");
        let a = create_task(input("a")).expect("create a");
        let b = create_task(input("b")).expect("create b");
        set_task_dep(b.id.clone(), a.id.clone(), None).expect("b after a");

        let error = set_task_dep(a.id.clone(), b.id.clone(), None).unwrap_err();
        assert!(error.to_string().contains("cycle"), "{error}");
        // The refused edge is not persisted.
        assert!(list_task_deps(a.id).unwrap().is_empty());
    }

    #[test]
    fn run_views_report_the_ledger() {
        let _home = init("cmd_tasks_runs");
        let created = create_task(input("ledger")).expect("create");
        let store = open().expect("store");
        store
            .insert_run(&future_tasks::TaskRun {
                id: future_tasks::new_run_id(),
                task_id: created.id.clone(),
                kind: future_tasks::RunKind::Chain,
                origin: future_tasks::RunOrigin::Chain,
                actor: Some("task:tsk_up".into()),
                due_at: None,
                status: future_tasks::RunStatus::Failed,
                thread_id: Some("thr_1".into()),
                session_id: Some("sess_1".into()),
                run_id: None,
                prompt_version: Some(2),
                result_summary: Some("partial".into()),
                feedback: None,
                feedback_note: None,
                started_at: Some(1),
                finished_at: Some(2),
                error_message: Some("agent unreachable".into()),
            })
            .expect("insert run");

        let runs = list_task_runs(created.id.clone(), None).expect("runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].kind, "chain");
        assert_eq!(runs[0].status, "failed");
        assert_eq!(runs[0].error_message.as_deref(), Some("agent unreachable"));
        assert_eq!(runs[0].prompt_version, Some(2));

        // The list row carries the latest run so the panel can badge it.
        let listed = list_tasks().expect("list");
        assert_eq!(listed[0].latest_run.as_ref().unwrap().id, runs[0].id);

        // The limit is honoured.
        assert!(list_task_runs(created.id, Some(0))
            .expect("runs")
            .is_empty());
    }
}
