//! Shared task form, revision and run-request services.
//!
//! Reuses the `future-tasks` store and kernel; execution stays in the shared
//! task tick loop. The Desktop adapter only translates Tauri IPC.

use future_tasks::{
    ConversationMode, RunOrigin, SessionPolicy, SessionRetention, Store, Task, TriggerKind,
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

fn open() -> Result<Store, crate::Error> {
    Store::open(&future_home()).map_err(|e| crate::Error::Message(e.to_string()))
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
    /// The IANA zone the schedule's wall-clock times are read in. Always a
    /// concrete name: a task with none stored reports the zone it resolves to
    /// (the host's, else Beijing), so the form can show what will actually run.
    pub timezone: String,
    pub session_policy: String,
    /// `keep` (default) or `delete`; see [`SessionRetention`]. Only meaningful
    /// with `session_policy = new`, and the clients only offer it there.
    pub session_retention: String,
    pub conversation_mode: String,
    pub trigger_kind: String,
    pub trigger: serde_json::Value,
    /// Always 0. Task dependencies were removed; the field stays because a phone
    /// of an older build reads `depCount` to label a task's trigger, and an
    /// absent field would leave it showing a trigger kind that no longer exists.
    /// A constant is honest about that: no task waits on another any more.
    pub dep_count: usize,
    pub next_due_at: Option<i64>,
    /// An explicit request is waiting for the tick (the task was busy).
    pub queued: bool,
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
    /// Why the run ended (`answer`, `empty_answer`, `cancelled`, …), so a run
    /// that produced nothing is not just an empty string with `completed` on
    /// it. Absent for rows written before the ledger recorded it.
    pub finish_reason: Option<String>,
    /// The one-line trace of where the output went, kept for runs that produced
    /// nothing usable.
    pub finish_note: Option<String>,
    /// The conversation this run used was deleted after it settled, so the
    /// panel shows the run without an "open conversation" link and says why.
    pub session_deleted: bool,
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
        finish_reason: run.finish_reason,
        finish_note: run.finish_note,
        session_deleted: run.session_deleted,
    }
}

fn task_view(store: &Store, task: Task) -> TaskView {
    let latest_run = store
        .latest_run_for_task(&task.id)
        .ok()
        .flatten()
        .map(run_view);
    // Resolved before the fields below are moved out of `task`.
    let timezone = future_tasks::task_timezone(&task).name().to_string();
    TaskView {
        id: task.id,
        name: task.name,
        enabled: task.enabled,
        prompt: task.prompt,
        prompt_version: task.prompt_version,
        cwd: task.cwd,
        model_id: task.model_id,
        thinking_level: task.thinking_level,
        timezone,
        session_policy: format!("{:?}", task.session_policy).to_lowercase(),
        session_retention: format!("{:?}", task.session_retention).to_lowercase(),
        conversation_mode: format!("{:?}", task.conversation_mode).to_lowercase(),
        trigger_kind: format!("{:?}", task.trigger_kind).to_lowercase(),
        trigger: task.trigger_json,
        dep_count: 0,
        next_due_at: task.next_due_at,
        queued: task.pending_request_at.is_some(),
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
    /// IANA zone (`Asia/Shanghai`). Absent or empty means "not chosen": the
    /// host's own zone is resolved and stored, Beijing when it cannot be read.
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub session_policy: Option<String>,
    #[serde(default)]
    pub conversation_mode: Option<String>,
    #[serde(default)]
    pub session_retention: Option<String>,
    #[serde(default)]
    pub trigger_kind: Option<String>,
    #[serde(default)]
    pub trigger: Option<serde_json::Value>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

fn parse_conversation_mode(raw: Option<&str>) -> ConversationMode {
    match raw {
        Some("chat") => ConversationMode::Chat,
        _ => ConversationMode::Workspace,
    }
}

fn parse_session_policy(raw: Option<&str>) -> SessionPolicy {
    match raw {
        Some("existing") => SessionPolicy::Existing,
        _ => SessionPolicy::New,
    }
}

/// `delete` is only meaningful for a task that opens a conversation per run, so
/// the combination is normalized here rather than stored: what the user asked
/// for survives in the column, and `kernel::deletes_run_conversation` decides
/// what actually happens. A client that offers the choice only for `new` never
/// produces the other combination anyway.
fn parse_session_retention(raw: Option<&str>, policy: SessionPolicy) -> SessionRetention {
    match (raw, policy) {
        (Some("delete"), SessionPolicy::New) => SessionRetention::Delete,
        _ => SessionRetention::Keep,
    }
}

/// The zone to store for a **create**: an explicit, valid name wins; an empty
/// value means "not chosen" and resolves to the host's zone. A non-empty name
/// this build cannot parse is refused rather than silently replaced — the user
/// typed a zone, and getting Beijing back without being told would make the
/// schedule fire on a clock they never picked.
fn parse_timezone_field(raw: Option<&str>) -> Result<Option<String>, crate::Error> {
    match raw.map(str::trim).filter(|name| !name.is_empty()) {
        None => Ok(Some(future_tasks::resolve_timezone(None))),
        Some(name) => match future_tasks::parse_timezone(name) {
            Some(tz) => Ok(Some(tz.name().to_string())),
            None => Err(crate::Error::Message(format!(
                "unknown timezone {name:?}: expected an IANA name like Asia/Shanghai"
            ))),
        },
    }
}

fn parse_trigger_kind(raw: Option<&str>) -> TriggerKind {
    match raw {
        Some("schedule") => TriggerKind::Schedule,
        _ => TriggerKind::Manual,
    }
}

pub fn list_tasks() -> Result<Vec<TaskView>, crate::Error> {
    let store = open()?;
    let tasks = store
        .list_tasks(false)
        .map_err(|e| crate::Error::Message(e.to_string()))?;
    Ok(tasks.into_iter().map(|t| task_view(&store, t)).collect())
}

pub fn create_task(input: TaskInput) -> Result<TaskView, crate::Error> {
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
        timezone: parse_timezone_field(input.timezone.as_deref())?,
        session_policy: parse_session_policy(input.session_policy.as_deref()),
        session_retention: parse_session_retention(
            input.session_retention.as_deref(),
            parse_session_policy(input.session_policy.as_deref()),
        ),
        conversation_mode: parse_conversation_mode(input.conversation_mode.as_deref()),
        thread_id: None,
        trigger_kind,
        trigger_json: trigger,
        next_due_at: None,
        pending_request_at: None,
        pending_origin: None,
        pending_actor: None,
        created_at: now,
        updated_at: now,
        deleted_at: None,
    };
    if trigger_kind == TriggerKind::Schedule {
        task.next_due_at = future_tasks::next_due(&task, now - 1);
    }
    store
        .insert_task(&task)
        .map_err(|e| crate::Error::Message(e.to_string()))?;
    Ok(task_view(&store, task))
}

pub fn update_task(id: String, input: TaskInput) -> Result<TaskView, crate::Error> {
    let store = open()?;
    let mut task = store
        .get_task(&id)
        .map_err(|e| crate::Error::Message(e.to_string()))?
        .ok_or_else(|| crate::Error::Message("task not found".to_string()))?;
    let trigger_kind = parse_trigger_kind(input.trigger_kind.as_deref());
    let trigger = input.trigger.unwrap_or_else(|| task.trigger_json.clone());
    // The version bookkeeping runs *before* `task.prompt` is overwritten: the
    // helper compares the incoming prompt against the task's current one, so
    // assigning first would look like an unchanged prompt and record nothing.
    let prompt_revisions = if input.prompt != task.prompt {
        let history = store
            .list_revisions(&task.id)
            .map_err(|e| crate::Error::Message(e.to_string()))?;
        // No reason prose, matching what this path wrote before: the field is
        // rendered as-is, so an English sentence would show up untranslated.
        future_tasks::prompt_change_revisions(
            &task,
            &history,
            &input.prompt,
            "user",
            None,
            now_ms(),
        )
    } else {
        Vec::new()
    };
    task.name = input.name;
    task.prompt = input.prompt;
    task.cwd = input.cwd;
    task.model_id = input.model_id;
    task.thinking_level = input.thinking_level;
    // An update that says nothing about the zone keeps the stored one; an empty
    // value asks for "the host's zone" explicitly. (A whole-record write from
    // another client may omit the field entirely.)
    if let Some(raw) = input.timezone.as_deref() {
        task.timezone = parse_timezone_field(Some(raw))?;
    }
    task.session_policy = parse_session_policy(input.session_policy.as_deref());
    task.session_retention =
        parse_session_retention(input.session_retention.as_deref(), task.session_policy);
    task.conversation_mode = parse_conversation_mode(input.conversation_mode.as_deref());
    task.trigger_kind = trigger_kind;
    task.trigger_json = trigger;
    if let Some(enabled) = input.enabled {
        task.enabled = enabled;
    }
    task.updated_at = now_ms();
    if trigger_kind == TriggerKind::Schedule {
        task.next_due_at = future_tasks::next_due(&task, now_ms() - 1);
    } else {
        task.next_due_at = None;
    }
    if !prompt_revisions.is_empty() {
        for row in &prompt_revisions {
            store
                .insert_revision(row)
                .map_err(|e| crate::Error::Message(e.to_string()))?;
        }
        if let Some(last) = prompt_revisions.last() {
            task.prompt_version = last.version;
        }
    }
    store
        .update_task(&task)
        .map_err(|e| crate::Error::Message(e.to_string()))?;
    Ok(task_view(&store, task))
}

pub fn delete_task(id: String) -> Result<(), crate::Error> {
    let store = open()?;
    let mut task = store
        .get_task(&id)
        .map_err(|e| crate::Error::Message(e.to_string()))?
        .ok_or_else(|| crate::Error::Message("task not found".to_string()))?;
    task.deleted_at = Some(now_ms());
    task.enabled = false;
    task.updated_at = now_ms();
    store
        .update_task(&task)
        .map_err(|e| crate::Error::Message(e.to_string()))?;
    // Nothing else to clean up: the `task_deps` rows that used to point at this
    // task are no longer read by anything (see `store::SCHEMA_SQL`), so deleting
    // the task leaves them where they are.
    Ok(())
}

pub fn set_task_enabled(id: String, enabled: bool) -> Result<TaskView, crate::Error> {
    let store = open()?;
    let mut task = store
        .get_task(&id)
        .map_err(|e| crate::Error::Message(e.to_string()))?
        .ok_or_else(|| crate::Error::Message("task not found".to_string()))?;
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
        .map_err(|e| crate::Error::Message(e.to_string()))?;
    Ok(task_view(&store, task))
}

/// Queue an explicit run. The tick loop consumes `pending_request_at`; the
/// wake makes it look now instead of at the next cadence tick, so pressing the
/// button starts the run rather than scheduling it up to 30s out.
pub fn run_task_now(id: String) -> Result<TaskView, crate::Error> {
    let store = open()?;
    let mut task = store
        .get_task(&id)
        .map_err(|e| crate::Error::Message(e.to_string()))?
        .ok_or_else(|| crate::Error::Message("task not found".to_string()))?;
    task.pending_request_at = Some(now_ms());
    task.pending_origin = Some(RunOrigin::Ui);
    task.pending_actor = Some("user".to_string());
    task.updated_at = now_ms();
    store
        .update_task(&task)
        .map_err(|e| crate::Error::Message(e.to_string()))?;
    crate::tasks::wake();
    Ok(task_view(&store, task))
}

pub fn list_task_runs(id: String, limit: Option<i64>) -> Result<Vec<RunView>, crate::Error> {
    let store = open()?;
    let runs = store
        .list_runs_for_task(&id, limit.unwrap_or(20))
        .map_err(|e| crate::Error::Message(e.to_string()))?;
    Ok(runs.into_iter().map(run_view).collect())
}

/// Dependency edges for a task.
///
/// **Transitional answer to a removed feature.** Task dependencies are gone, so
/// there are never any edges: this returns an empty list rather than refusing,
/// because a client of an older build reads it for the list it renders. The
/// command (and the phone's `list_task_deps` on the remote bridge) stays until no
/// shipped client asks for it.
pub fn list_task_deps(_id: String) -> Result<Vec<DepView>, crate::Error> {
    Ok(Vec::new())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DepView {
    pub upstream_task_id: String,
    pub upstream_name: String,
    pub on: String,
    pub satisfied: bool,
}

/// Add or change a dependency edge.
///
/// **Transitional answer to a removed feature**: refused explicitly rather than
/// accepted and dropped. A caller that got `Ok` would believe it wrote an edge
/// that nothing will ever act on — the same silence this removal is meant to end.
pub fn set_task_dep(
    _id: String,
    _upstream_task_id: String,
    _on: Option<String>,
) -> Result<(), crate::Error> {
    Err(crate::Error::Message(
        "task dependencies have been removed; a task now runs on its own schedule or on demand"
            .to_string(),
    ))
}

/// Remove a dependency edge — likewise refused (see [`set_task_dep`]).
pub fn remove_task_dep(_id: String, _upstream_task_id: String) -> Result<(), crate::Error> {
    Err(crate::Error::Message(
        "task dependencies have been removed; there are no edges to remove".to_string(),
    ))
}

/// Prompt revisions for the version history / diff UI.
pub fn list_task_revisions(id: String) -> Result<Vec<RevisionView>, crate::Error> {
    let store = open()?;
    let revisions = store
        .list_revisions(&id)
        .map_err(|e| crate::Error::Message(e.to_string()))?;
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
            // The run a suggestion came from: the detail view groups a
            // suggestion under that run's result instead of parking every
            // suggestion at the bottom of the page.
            source_run_id: r.source_run_id,
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
    /// The run this revision came from (a suggestion names the run it read).
    pub source_run_id: Option<String>,
    pub created_at: i64,
}

/// Apply a stored revision as the task's active prompt.
pub fn apply_task_revision(id: String, revision_id: String) -> Result<TaskView, crate::Error> {
    let store = open()?;
    // Shared with the phone's remote bridge, so accepting a suggestion means
    // the same thing on both surfaces (crate::tasks::accept_revision).
    let task =
        crate::tasks::accept_revision(&store, &id, &revision_id).map_err(crate::Error::Message)?;
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
            timezone: Some("Asia/Shanghai".into()),
            session_policy: Some("existing".into()),
            conversation_mode: Some("chat".into()),
            session_retention: None,
            trigger_kind: Some("schedule".into()),
            trigger: Some(serde_json::json!({"mode": "daily", "time": "09:00"})),
            enabled: None,
        }
    }

    /// The three `parse_*` helpers are the form's normalization layer: an
    /// unknown or absent value must land on the default, and only the exact
    /// wire string selects the non-default variant. `delete` retention is
    /// meaningful only for a run that opens a new conversation.
    #[test]
    fn unknown_task_form_values_normalize_to_the_default() {
        assert_eq!(
            parse_conversation_mode(Some("chat")),
            ConversationMode::Chat
        );
        for raw in [None, Some("bogus"), Some("")] {
            assert_eq!(parse_conversation_mode(raw), ConversationMode::Workspace);
        }
        assert_eq!(
            parse_session_policy(Some("existing")),
            SessionPolicy::Existing
        );
        for raw in [None, Some("bogus"), Some("")] {
            assert_eq!(parse_session_policy(raw), SessionPolicy::New);
        }
        assert_eq!(parse_trigger_kind(Some("schedule")), TriggerKind::Schedule);
        for raw in [None, Some("bogus")] {
            assert_eq!(parse_trigger_kind(raw), TriggerKind::Manual);
        }
        assert_eq!(
            parse_session_retention(Some("delete"), SessionPolicy::New),
            SessionRetention::Delete
        );
        assert_eq!(
            parse_session_retention(Some("delete"), SessionPolicy::Existing),
            SessionRetention::Keep,
            "a reused conversation is never deleted by a retention choice"
        );
        assert_eq!(
            parse_session_retention(None, SessionPolicy::New),
            SessionRetention::Keep
        );
    }

    /// `FUTURE_HOME` relocates the whole root; the command surface must resolve
    /// the tasks store from the same override the tick loop uses, or the GUI
    /// would show one task list while the loop runs another.
    #[test]
    fn the_task_store_home_follows_a_future_home_override() {
        let _home = init("cmd_tasks_home_override");
        let dir = std::env::temp_dir().join("futureos-task-home-override");
        std::env::set_var("FUTURE_HOME", &dir);
        assert_eq!(future_home(), dir);
        std::env::remove_var("FUTURE_HOME");
    }

    /// `enabled` is optional on the update payload; when present it must reach
    /// the stored row (both directions), and when absent the existing value has
    /// to survive the edit.
    #[test]
    fn updating_a_task_carries_an_explicit_enabled_flag() {
        let _home = init("cmd_tasks_disable");
        let created = create_task(input("daily")).expect("create");
        assert!(created.enabled, "a new task is enabled by default");

        let mut disable = input("daily");
        disable.enabled = Some(false);
        let disabled = update_task(created.id.clone(), disable).expect("disable");
        assert!(!disabled.enabled);
        let listed = list_tasks()
            .expect("list")
            .into_iter()
            .find(|task| task.id == created.id)
            .expect("the task is still listed");
        assert!(
            !listed.enabled,
            "the update must be persisted, not just viewed"
        );

        // An edit that leaves `enabled` unset keeps the stored false.
        let edit = input("daily renamed");
        assert!(edit.enabled.is_none());
        let kept = update_task(created.id.clone(), edit).expect("edit");
        assert!(!kept.enabled, "an absent flag must not re-enable the task");
        assert_eq!(kept.name, "daily renamed");

        let mut enable = input("daily renamed");
        enable.enabled = Some(true);
        assert!(update_task(created.id, enable).expect("enable").enabled);
    }

    #[test]
    fn a_task_round_trips_through_the_command_surface() {
        let _home = init("cmd_tasks_round_trip");

        let created = create_task(input("daily")).expect("create");
        assert_eq!(created.name, "daily");
        assert_eq!(created.session_policy, "existing");
        assert_eq!(created.conversation_mode, "chat");
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

        // The edit records the new prompt *and* the one it replaced, so the
        // version the task started with stays reachable by `prompt revert`.
        let revisions = list_task_revisions(created.id.clone()).expect("revisions");
        assert_eq!(revisions.len(), 2);
        let by_version = |v: i64| {
            revisions
                .iter()
                .find(|r| r.version == v)
                .unwrap_or_else(|| panic!("no revision for v{v}: {revisions:?}"))
        };
        assert_eq!(by_version(1).source, "superseded");
        assert_eq!(by_version(2).source, "user");

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
        let after = list_task_revisions(created.id.clone()).expect("revisions");
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

        // Applying the version that is already active changes nothing: a bump
        // for a no-op would fill the history with identical copies.
        let active = after.last().unwrap().id.clone();
        let again = apply_task_revision(created.id.clone(), active).expect("apply again");
        assert_eq!(again.prompt_version, 3, "no version is spent on a no-op");
        assert_eq!(
            list_task_revisions(created.id).expect("revisions").len(),
            after.len()
        );
    }

    /// Applying a stored version puts it in force, and the version it replaced
    /// stays in the history — that record is the whole point of keeping versions.
    #[test]
    fn applying_a_stored_version_keeps_the_one_it_replaced() {
        let _home = init("cmd_tasks_apply_revision");
        let created = create_task(input("applies")).expect("create");

        let store = open().expect("store");
        store
            .insert_revision(&future_tasks::PromptRevision {
                id: "rev_older".into(),
                task_id: created.id.clone(),
                version: 9,
                prompt: "an earlier prompt".into(),
                source: future_tasks::REVISION_SOURCE_USER.into(),
                status: future_tasks::REVISION_STATUS_SUPERSEDED.into(),
                reason: None,
                confidence: None,
                source_run_id: None,
                created_at: now_ms(),
            })
            .unwrap();

        let applied = apply_task_revision(created.id.clone(), "rev_older".into()).expect("apply");
        assert_eq!(applied.prompt, "an earlier prompt");
        assert_eq!(applied.prompt_version, 2, "the next version in sequence");

        let revisions = list_task_revisions(created.id.clone()).expect("revisions");
        let live = revisions
            .iter()
            .find(|r| r.version == applied.prompt_version)
            .expect("the live version");
        assert_eq!(live.source, future_tasks::REVISION_SOURCE_ROLLBACK);
        assert_eq!(live.status, future_tasks::REVISION_STATUS_ACTIVE);
        assert!(
            revisions
                .iter()
                .any(|r| r.status == future_tasks::REVISION_STATUS_SUPERSEDED),
            "the replaced version stays in the history: {revisions:?}"
        );
        // Applying what is already in force is a no-op, not another version.
        let again = apply_task_revision(created.id.clone(), "rev_older".into()).expect("re-apply");
        assert_eq!(again.prompt_version, applied.prompt_version);
        assert_eq!(
            list_task_revisions(created.id).expect("revisions").len(),
            revisions.len()
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

    /// The zone is part of the record: what the form sends is what the schedule
    /// reads, and a task that sends none still ends up with a concrete name
    /// (the host's zone, else Beijing) rather than a NULL that the next read has
    /// to guess at again.
    #[test]
    fn the_task_zone_is_stored_and_an_unset_one_resolves() {
        let _home = init("cmd_tasks_timezone");
        let created = create_task(input("shanghai")).expect("create");
        assert_eq!(created.timezone, "Asia/Shanghai");

        let mut tokyo = input("tokyo");
        tokyo.timezone = Some("Asia/Tokyo".into());
        let created = create_task(tokyo).expect("create tokyo");
        assert_eq!(created.timezone, "Asia/Tokyo");
        assert_eq!(
            list_scheduled_tasks_includes(&created.id).timezone,
            "Asia/Tokyo",
            "the view reports what the row carries"
        );

        // An edit that does not mention the zone keeps it; an explicit zone
        // replaces it (canonically, whatever the client's spelling).
        let mut edit = input("tokyo");
        edit.timezone = None;
        assert_eq!(
            update_task(created.id.clone(), edit)
                .expect("edit")
                .timezone,
            "Asia/Tokyo"
        );
        let mut edit = input("tokyo");
        edit.timezone = Some("europe/berlin".into());
        assert_eq!(
            update_task(created.id.clone(), edit)
                .expect("edit")
                .timezone,
            "Europe/Berlin"
        );

        // An empty value asks for the host's zone.
        let mut edit = input("tokyo");
        edit.timezone = Some(String::new());
        assert_eq!(
            update_task(created.id, edit).expect("edit").timezone,
            future_tasks::resolve_timezone(None)
        );

        // A name nobody can resolve is refused, not silently replaced.
        let mut bad = input("bogus");
        bad.timezone = Some("Mars/Olympus_Mons".into());
        let error = create_task(bad).unwrap_err();
        assert!(error.to_string().contains("unknown timezone"), "{error}");
        assert!(
            list_tasks().expect("list").len() == 2,
            "nothing was created"
        );
    }

    /// The schedule is computed in each task's own zone, not the host's: a daily
    /// 09:00 reads 09:00 in the zone it was written in, whatever this machine's
    /// clock says — and the two tasks therefore come due at two different
    /// instants.
    ///
    /// Read each due date in *its own* zone rather than in a single one. A
    /// hard-coded "09:00 Berlin is 15:00 Shanghai" also encodes the offset in
    /// force on the day it runs: Europe/Berlin is UTC+2 until the last Sunday of
    /// October and UTC+1 after it, so the same correct schedule reads 15:00 in
    /// summer and 16:00 in winter, and the assertion goes red for half the year
    /// on code that never changed. (It did: the next 09:00 Berlin fell after the
    /// 2026-10-25 switch, so from that morning this test failed on every machine
    /// — including CI, which had last run it before the switch.)
    #[test]
    fn a_tasks_schedule_uses_its_own_zone() {
        let _home = init("cmd_tasks_timezone_schedule");
        let mut shanghai = input("daily shanghai");
        shanghai.timezone = Some("Asia/Shanghai".into());
        let shanghai = create_task(shanghai).expect("create");
        let mut berlin = input("daily berlin");
        berlin.timezone = Some("Europe/Berlin".into());
        let berlin = create_task(berlin).expect("create");

        let due_in = |task: &TaskView, zone: chrono_tz::Tz| {
            chrono::DateTime::from_timestamp_millis(task.next_due_at.expect("a schedule"))
                .unwrap()
                .with_timezone(&zone)
                .format("%H:%M")
                .to_string()
        };
        assert_eq!(due_in(&shanghai, chrono_tz::Tz::Asia__Shanghai), "09:00");
        assert_eq!(due_in(&berlin, chrono_tz::Tz::Europe__Berlin), "09:00");
        // The host's zone is neither task's: read through it, Berlin's 09:00 is
        // not 09:00, and the two instants are hours apart. A schedule that used
        // the host's zone for both would land them on one instant.
        assert_ne!(due_in(&berlin, chrono_tz::Tz::Asia__Shanghai), "09:00");
        assert_ne!(
            shanghai.next_due_at, berlin.next_due_at,
            "one wall clock in two zones is two different instants"
        );
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

        // The view says so too. Pressing "run now" while a run is in flight
        // makes the tick hold the request until the run ends; without this flag
        // the panel would look like the button did nothing.
        assert!(queued.queued, "the queued request is visible to the caller");
        assert!(
            list_scheduled_tasks_includes(&created.id).queued,
            "and to a list refresh"
        );

        // A task with nothing waiting is not reported as queued.
        let idle = create_task(input("idle")).expect("create idle");
        assert!(!idle.queued);
    }

    /// The list view for one task (the command returns them all).
    fn list_scheduled_tasks_includes(id: &str) -> TaskView {
        list_tasks()
            .expect("list")
            .into_iter()
            .find(|t| t.id == id)
            .expect("the task is listed")
    }

    /// Asking for the runs of a task that no longer exists answers with no runs
    /// rather than an error: the panel may hold an id it just deleted, and the
    /// ledger is empty either way.
    /// The removed feature's wire commands still answer, so a client of an older
    /// build does not see an unknown-command error: a read says "no edges", and a
    /// write says outright that dependencies are gone rather than accepting an
    /// edge nothing will act on.
    #[test]
    fn the_dependency_commands_are_transitional() {
        let _home = init("cmd_tasks_dep_removed");
        let task = create_task(input("solo")).expect("create");

        assert!(
            list_task_deps(task.id.clone()).unwrap().is_empty(),
            "there are no edges to list"
        );
        let error = set_task_dep(task.id.clone(), task.id.clone(), None).unwrap_err();
        assert!(error.to_string().contains("removed"), "{error}");
        let error = remove_task_dep(task.id.clone(), task.id.clone()).unwrap_err();
        assert!(error.to_string().contains("removed"), "{error}");

        // And the view keeps the field a phone reads, always at zero.
        assert_eq!(list_scheduled_tasks_includes(&task.id).dep_count, 0);
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
                session_deleted: false,
                finish_reason: None,
                finish_note: None,
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
