//! Desktop host for `future-tasks`.
//!
//! The deterministic kernel and store live in the `future-tasks` crate; this
//! module is the desktop's tick loop plus the `Executor` wiring that reaches
//! the Agent service, creates threads, and notifies the webview. Headless
//! desktop reuses the same executor with notifications downgraded to ledger
//! writes (no webview).

use std::path::PathBuf;
use std::time::Duration;

use future_tasks::{
    compose_run_prompt, kernel, new_run_id, ConversationMode, RunKind, RunOrigin, RunStatus,
    SessionPolicy, Store, Task, TaskRun,
};

/// Tick cadence. The store is re-read every tick, so CLI/remote edits land
/// without any cache invalidation.
const TICK_INTERVAL: Duration = Duration::from_secs(30);

/// FutureOS home root (shared with the CLI and the remote bridge, so every
/// writer resolves the same `tasks.db`).
fn future_home() -> PathBuf {
    crate::future_home_root()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Put the prompt a revision carries in force, wherever the request came from.
///
/// The webview command and the phone's remote bridge both route through here,
/// so "apply this version" means the same thing on both (the replaced version
/// stays in the history and the new one is recorded as the active version). A
/// second, thinner implementation on the remote side had drifted into recording
/// a different row than the panel did.
pub fn accept_revision(store: &Store, task_id: &str, revision_id: &str) -> Result<Task, String> {
    let mut task = store
        .get_task(task_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "task not found".to_string())?;
    let history = store.list_revisions(task_id).map_err(|e| e.to_string())?;
    let revision = history
        .iter()
        .find(|r| r.id == revision_id)
        .ok_or_else(|| "revision not found".to_string())?
        .clone();
    let reason = Some(format!(
        "applied revision {} (v{})",
        revision_id, revision.version
    ));
    let rows = kernel::prompt_change_revisions(
        &task,
        &history,
        &revision.prompt,
        future_tasks::REVISION_SOURCE_ROLLBACK,
        reason.as_deref(),
        now_ms(),
    );
    let Some(applied) = rows.last() else {
        // Already in force: report the task unchanged rather than bumping a
        // version for a no-op.
        return Ok(task);
    };
    for row in &rows {
        store.insert_revision(row).map_err(|e| e.to_string())?;
    }
    task.prompt = applied.prompt.clone();
    task.prompt_version = applied.version;
    task.updated_at = now_ms();
    store.update_task(&task).map_err(|e| e.to_string())?;
    Ok(task)
}

/// How a finished run announces itself. The GUI refreshes the sidebar; the
/// headless server has no webview and the run ledger is the only signal.
pub type Notifier = std::sync::Arc<dyn Fn(Option<&str>) + Send + Sync>;

/// The tick loop's wake signal.
///
/// A request that arrives between ticks used to wait for the next one — up to
/// `TICK_INTERVAL` of nothing happening after the user pressed "run now". The
/// request is still *stored* first (the tick remains the only thing that
/// claims runs), but the loop is woken so it looks immediately. `Notify` keeps
/// one permit, so a wake that lands while a tick is already running is not
/// lost.
fn wake_signal() -> &'static tokio::sync::Notify {
    static WAKE: std::sync::OnceLock<tokio::sync::Notify> = std::sync::OnceLock::new();
    WAKE.get_or_init(tokio::sync::Notify::new)
}

/// Ask the tick loop to tick now. Called after an explicit run is stored
/// (the desktop's button, the phone's, and a chained wakeup).
pub fn wake() {
    wake_signal().notify_one();
}

/// Start the tasks tick loop. Called once from `lib.rs` setup (GUI) and once
/// from `headless/mod.rs` (server). The loop is deliberately independent of
/// the maintenance `scheduler/` module: tasks are user-defined work, not app
/// upkeep, and they need the store/executor boundary that maintenance does
/// not have.
#[cfg(feature = "gui")]
pub fn start<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    let _ = app;
    let notify: Notifier = std::sync::Arc::new(|thread_id| {
        if let Some(thread_id) = thread_id {
            crate::emit_remote_activity(thread_id);
        }
        crate::emit_threads_updated();
    });
    crate::runtime::spawn(run_loop(notify));
}

/// Headless variant: same loop, notifications downgraded to nothing (the run
/// ledger in `tasks.db` is the observable signal).
static STOPPING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub(crate) fn stop_admission() {
    STOPPING.store(true, std::sync::atomic::Ordering::Release);
    wake();
}
#[cfg(test)]
pub(crate) fn reset_for_test() {
    STOPPING.store(false, std::sync::atomic::Ordering::Release);
}

pub fn start_headless() {
    let notify: Notifier = std::sync::Arc::new(|_| {});
    crate::runtime::spawn(run_loop(notify));
}

async fn run_loop(notify: Notifier) {
    // Startup reconciliation: runs left `running` by a crash are marked
    // failed("interrupted") before the first tick, matching the run-row
    // reconciliation the GUI already does for agent runs.
    if let Ok(store) = Store::open(&future_home()) {
        if let Err(error) = store.mark_interrupted_runs(now_ms()) {
            eprintln!("FutureOS tasks: interrupted-run reconciliation failed: {error}");
        }
    }
    let mut interval = tokio::time::interval(TICK_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        if STOPPING.load(std::sync::atomic::Ordering::Acquire) {
            break;
        }
        // Wait for the cadence *or* for someone to ask for a run right now.
        // `Interval::tick` is cancel-safe, so losing this race does not skip a
        // scheduled tick.
        tokio::select! {
            _ = interval.tick() => {}
            _ = wake_signal().notified() => {}
        }
        if let Err(error) = tick(&notify).await {
            eprintln!("FutureOS tasks tick failed: {error}");
        }
    }
}

/// Why a task is being claimed this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClaimReason {
    /// Someone asked for it: the UI button, the phone, or `future task run`.
    Explicit,
    /// The task's own schedule reached its next slot.
    Schedule,
}

/// Which tasks run this tick, and why.
fn candidates(store: &Store, now: i64) -> Result<Vec<(Task, ClaimReason)>, String> {
    let due = store.list_due_tasks(now).map_err(|e| e.to_string())?;
    let out: Vec<(Task, ClaimReason)> = due
        .into_iter()
        .map(|task| {
            // An explicit request outranks the schedule: the user asked for a
            // run *now*.
            let reason = if task.pending_request_at.is_some() {
                ClaimReason::Explicit
            } else {
                ClaimReason::Schedule
            };
            (task, reason)
        })
        .collect();

    Ok(out)
}

/// One tick: claim due tasks and execute them. Sequential within one host; each
/// task spawns its own run so a slow task does not block the next tick.
async fn tick(notify: &Notifier) -> Result<(), String> {
    let store = Store::open(&future_home()).map_err(|e| e.to_string())?;
    for (task, reason) in candidates(&store, now_ms())? {
        if STOPPING.load(std::sync::atomic::Ordering::Acquire) {
            break;
        }
        if store.has_running_run(&task.id).map_err(|e| e.to_string())? {
            continue; // overlap: skip this tick, the next one retries
        }
        let run = claim(&store, &task, reason)?;
        let notify = notify.clone();
        // Own finalization before the first poll, including rejected/aborted spawns.
        let interrupted = InterruptedTask {
            home: future_home(),
            id: run.id.clone(),
        };
        crate::runtime::spawn(async move {
            let _interrupted = interrupted;
            // A fresh store per run: `rusqlite::Connection` is not `Sync`, so
            // the tick's store cannot cross the spawn boundary.
            let store = match Store::open(&future_home()) {
                Ok(s) => s,
                Err(error) => {
                    eprintln!("FutureOS tasks: could not open store for run: {error}");
                    return;
                }
            };
            if let Err(error) = execute(&notify, store, task, run).await {
                eprintln!("FutureOS task execution failed: {error}");
            }
        });
    }
    Ok(())
}

/// Claim one due task: insert the run row (running), advance `next_due_at` and
/// clear `pending_request_at`.
fn claim(store: &Store, task: &Task, reason: ClaimReason) -> Result<TaskRun, String> {
    let now = now_ms();
    // The reason picks the run's label: an explicit request keeps whatever
    // origin asked for it, a schedule slot is `main`. A schedule claim resolves
    // its `due_at` to the slot it fired for.
    let (kind, origin, due_at) = match reason {
        ClaimReason::Explicit => (
            RunKind::Manual,
            task.pending_origin.unwrap_or(RunOrigin::Cli),
            None,
        ),
        ClaimReason::Schedule => (RunKind::Main, RunOrigin::Schedule, task.next_due_at),
    };

    let run = TaskRun {
        id: new_run_id(),
        task_id: task.id.clone(),
        kind,
        origin,
        actor: task.pending_actor.clone(),
        due_at,
        status: RunStatus::Running,
        thread_id: None,
        session_id: None,
        run_id: None,
        prompt_version: Some(task.prompt_version),
        result_summary: None,
        feedback: None,
        feedback_note: None,
        started_at: Some(now),
        finished_at: None,
        error_message: None,
        session_deleted: false,
    };
    store.insert_run(&run).map_err(|e| e.to_string())?;

    // Advance schedule / clear pending.
    let mut t = task.clone();
    if kind == RunKind::Main {
        t.next_due_at = kernel::next_due(task, now);
    }
    t.pending_request_at = None;
    t.pending_origin = None;
    t.pending_actor = None;
    t.updated_at = now;
    store.update_task(&t).map_err(|e| e.to_string())?;

    Ok(run)
}

/// Execute one claimed run: session → envelope → prompt → finalize.
struct InterruptedTask {
    home: std::path::PathBuf,
    id: String,
}
impl Drop for InterruptedTask {
    fn drop(&mut self) {
        // Only this executor's claimed run is settled; other processes' task
        // rows and externally submitted Agent conversations are untouched.
        let result = (|| {
            let store = Store::open(&self.home).map_err(|e| e.to_string())?;
            if let Some(mut run) = store.get_run(&self.id).map_err(|e| e.to_string())? {
                if run.status == RunStatus::Running {
                    run.status = RunStatus::Failed;
                    run.finished_at = Some(now_ms());
                    run.error_message = Some("Desktop stopped before task completion".into());
                    store.update_run(&run).map_err(|e| e.to_string())?;
                }
            }
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            eprintln!("FutureOS task interruption persistence failed: {error}");
            if crate::desktop_shutdown::is_stopping() {
                crate::runtime::record_cleanup_error(error);
            }
        }
    }
}
struct AbortTaskOnDrop(crate::runtime::TaskHandle);
impl Drop for AbortTaskOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn execute(
    notify: &Notifier,
    store: Store,
    task: Task,
    mut run: TaskRun,
) -> Result<(), String> {
    let _interrupted = InterruptedTask {
        home: future_home(),
        id: run.id.clone(),
    };
    // `Store` owns a `rusqlite::Connection` (Send but not Sync), so it is
    // moved through the async call and handed back rather than borrowed.
    let (store, outcome) = run_inner(store, &task, &mut run).await;
    let (status, error, output) = match outcome {
        Ok(summary) => {
            run.result_summary = Some(kernel::truncate(&summary, kernel::RESULT_SUMMARY_CHARS));
            (RunStatus::Completed, None, Some(summary))
        }
        Err(error) => (RunStatus::Failed, Some(error), None),
    };
    run.status = status;
    run.finished_at = Some(now_ms());
    run.error_message = error;
    store.update_run(&run).map_err(|e| e.to_string())?;
    // Save the answer before anything can take it away: a task configured to
    // delete its conversations keeps its *results*, only the conversation goes.
    // The summary above is what lists and downstream runs read; this is the
    // whole answer, so the run stays readable afterwards.
    if let Some(text) = output {
        let saved = kernel::truncate(&text, kernel::RUN_OUTPUT_CHARS);
        store
            .set_run_output(&run.id, &saved)
            .map_err(|e| e.to_string())?;
    }

    // Then the conversation, when the task does not keep one per run. After the
    // ledger write, never before: whatever happens to the delete, the run's
    // status and answer are already durable.
    if kernel::deletes_run_conversation(&task) {
        if let Some(thread_id) = run.thread_id.clone() {
            match delete_run_conversation(&thread_id).await {
                Ok(()) => {
                    store
                        .mark_run_conversation_deleted(&run.id)
                        .map_err(|e| e.to_string())?;
                    run.session_deleted = true;
                    run.thread_id = None;
                    run.session_id = None;
                }
                // Leave the conversation and its ids in place: it is still
                // there and still openable, which is a better outcome than a
                // ledger that claims a delete that did not happen. The next run
                // is unaffected (each run has its own conversation).
                Err(error) => eprintln!(
                    "FutureOS tasks: could not delete the conversation of run {}: {error}",
                    run.id
                ),
            }
        }
    }

    // Notify the host (webview refresh; headless downgrades to a no-op).
    notify(run.thread_id.as_deref());

    Ok(())
}

/// Delete the conversation a finished run used (`session_retention = delete`).
///
/// Goes through the same path as a user deleting a conversation, so both mean
/// exactly the same thing. Chat conversations take their temporary workspace
/// with them (`delete_files`), like any other delete; a workspace conversation
/// is never touched on disk — that directory is the user's, not the run's.
async fn delete_run_conversation(thread_id: &str) -> Result<(), String> {
    crate::conversations::delete_conversation(thread_id, true)
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// The inner run: resolve the session and submit the prompt. Returns the final assistant
/// text on success.
async fn run_inner(
    store: Store,
    task: &Task,
    run: &mut TaskRun,
) -> (Store, Result<String, String>) {
    // 1. Session: new conversation, or the task's bound one (lazy create).
    //    Every store access below happens between awaits, never across one:
    //    the store owns a `rusqlite::Connection`, which is `Send` but not
    //    `Sync`, so a `&Store` held across an await would make this future
    //    non-`Send` and break `tokio::spawn`.
    let (thread_id, session_id) = match task.session_policy {
        SessionPolicy::New => match create_new_session(task).await {
            Ok(ids) => ids,
            Err(error) => return (store, Err(error)),
        },
        SessionPolicy::Existing => {
            let bound = bound_thread(&store, task);
            match bound {
                Err(error) => return (store, Err(error)),
                Ok(Some(ids)) => ids,
                Ok(None) => {
                    let ids = match create_new_session(task).await {
                        Ok(ids) => ids,
                        Err(error) => return (store, Err(error)),
                    };
                    if let Err(error) = bind_task_thread(&store, task, &ids.0) {
                        return (store, Err(error));
                    }
                    ids
                }
            }
        }
    };
    run.thread_id = Some(thread_id.clone());
    run.session_id = Some(session_id.clone());
    if let Err(error) = store.update_run(run) {
        return (store, Err(error.to_string()));
    }

    // 2. Envelope + instruction + completion contract.
    //    A reused conversation is *not* compacted first: the point of
    //    `session_policy=existing` is continuity, and a compaction both costs a
    //    pass over the whole history and drops the context the next run was
    //    meant to inherit.
    let prompt = compose_run_prompt(
        task,
        run.kind,
        run.due_at,
        None,
        run.started_at.unwrap_or_else(now_ms),
    );

    let thread = match crate::store::get_thread(&thread_id) {
        Ok(Some(thread)) => thread,
        Ok(None) => return (store, Err("task thread could not be loaded".to_string())),
        Err(error) => return (store, Err(error.to_string())),
    };
    let prepared = match crate::agent_bridge::prepare_prompt_persisted_with_trigger(
        &thread,
        prompt,
        task.model_id.clone(),
        task.thinking_level.clone(),
        Vec::new(),
        None,
    ) {
        Ok(prepared) => prepared,
        Err(error) => return (store, Err(error.to_string())),
    };
    let (tx, rx) = tokio::sync::oneshot::channel();
    let (finished, result) = tokio::sync::oneshot::channel();
    let handle = crate::runtime::spawn(async move {
        let result = crate::agent_bridge::run_prepared_prompt_with_acceptance(prepared, tx).await;
        let _ = finished.send(result);
    });
    // Both futures are registered. Cancelling the executor aborts the collector,
    // and the process cleanup still waits for its actual drop before reset.
    let _abort_on_drop = AbortTaskOnDrop(handle);
    let _ = rx.await;
    let prompt_result = result
        .await
        .map_err(|e| format!("prompt task stopped: {e}"))
        .and_then(|inner| inner.map_err(|e| e.to_string()));
    if let Err(error) = prompt_result {
        return (store, Err(error));
    }

    // The final assistant text is the run's result summary. Read it back from
    // the agent session (the durable source of truth).
    let text = match read_final_assistant_text(&session_id).await {
        Ok(text) => text,
        Err(error) => return (store, Err(error)),
    };
    (store, Ok(text))
}

async fn create_new_session(task: &Task) -> Result<(String, String), String> {
    let due = now_ms();
    let title = format!(
        "{} · {}",
        task.name,
        chrono::DateTime::from_timestamp_millis(due)
            .map(|d| d
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string())
            .unwrap_or_default()
    );
    // A task's `conversation_mode` decides where the conversation is filed, and
    // — since a chat task needs no working directory of its own — where the
    // agent works. A chat conversation runs in the temporary workspace every
    // chat conversation gets; a workspace conversation runs in `task.cwd`.
    let chat = task.conversation_mode == ConversationMode::Chat;
    let cwd = task.cwd.trim();
    // A workspace conversation *is* its directory (the sidebar files it under
    // that path), so an empty one is refused here rather than silently filed
    // under whatever the process happens to be sitting in.
    if !chat && cwd.is_empty() {
        return Err("a workspace conversation needs a working directory".to_string());
    }
    let thread = crate::store::create_thread(crate::store::CreateThreadInput {
        mode: if chat { "chat" } else { "workspace" }.to_string(),
        title: Some(title),
        workspace_id: None,
        workspace_path: if chat { None } else { Some(cwd.to_string()) },
        workspace_name: None,
        agent_session_id: None,
    })
    .map_err(|e| e.to_string())?;
    let session_id = crate::agent_bridge::provision_agent_session_with_policy(
        &thread.id,
        task.model_id.clone(),
        task.thinking_level.clone(),
        "all",
        Some("off"),
    )
    .await
    .map_err(|e| e.to_string())?;
    if chat && !cwd.is_empty() {
        // A chat task may still carry a directory (tasks created before chat
        // stopped asking for one, or an explicit path the user typed), and it
        // keeps working there. Without one, the conversation's own temporary
        // workspace stands — which is what a chat conversation means.
        set_session_cwd(&session_id, cwd).await?;
    }
    Ok((thread.id, session_id))
}

/// Point an agent session at a working directory (the chat-mode repair above).
async fn set_session_cwd(session_id: &str, cwd: &str) -> Result<(), String> {
    let mut client = crate::agent_bridge::connect_agent()
        .await
        .map_err(|e| e.to_string())?;
    let command = future_rpc::proto::RpcCommand {
        id: future_tasks::new_run_id(),
        r#type: "set_cwd".to_string(),
        session_id: session_id.to_string(),
        cwd: cwd.to_string(),
        ..Default::default()
    };
    let response = client
        .execute_command(command)
        .await
        .map_err(|e| format!("set_cwd: {e}"))?
        .into_inner();
    if !response.success {
        return Err(format!(
            "Could not set the task workspace: {}",
            response.error
        ));
    }
    Ok(())
}

/// The already-bound conversation for an `existing` task, when there is one.
/// Reads the desktop store (not the tasks store), so no handle is needed.
fn bound_thread(_store: &Store, task: &Task) -> Result<Option<(String, String)>, String> {
    let Some(thread_id) = &task.thread_id else {
        return Ok(None);
    };
    let thread = crate::store::get_thread(thread_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "bound task thread is missing".to_string())?;
    let session_id = thread
        .agent_session_id
        .clone()
        .ok_or_else(|| "bound task thread has no agent session".to_string())?;
    Ok(Some((thread_id.clone(), session_id)))
}

/// Bind the lazily created conversation to the task (idempotent).
fn bind_task_thread(store: &Store, task: &Task, thread_id: &str) -> Result<(), String> {
    let mut t = task.clone();
    t.thread_id = Some(thread_id.to_string());
    t.updated_at = now_ms();
    store.update_task(&t).map_err(|e| e.to_string())
}

/// Read the final assistant text of the last run in a session (the durable
/// journal is the source of truth).
async fn read_final_assistant_text(session_id: &str) -> Result<String, String> {
    let mut client = crate::agent_bridge::connect_agent()
        .await
        .map_err(|e| e.to_string())?;
    let response = client
        .execute_command(future_rpc::proto::RpcCommand {
            id: future_tasks::new_run_id(),
            r#type: "get_last_assistant_text".to_string(),
            session_id: session_id.to_string(),
            ..Default::default()
        })
        .await
        .map_err(|e| format!("get_last_assistant_text: {e}"))?
        .into_inner();
    if !response.success {
        return Err(format!(
            "get_last_assistant_text failed: {}",
            response.error
        ));
    }
    let value = future_rpc::decode::response_data(&response);
    Ok(value
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use future_tasks::{new_task_id, Task, TriggerKind};
    use tempfile::TempDir;

    /// A store rooted at a throwaway directory (the real `future_home()` is
    /// never touched by these tests).
    fn store() -> (TempDir, Store) {
        let dir = TempDir::new().unwrap();
        let store = Store::open(dir.path()).unwrap();
        (dir, store)
    }

    fn task(name: &str) -> Task {
        Task {
            id: new_task_id(),
            name: name.into(),
            enabled: true,
            prompt: "do the thing".into(),
            prompt_version: 1,
            cwd: "/tmp".into(),
            model_id: None,
            thinking_level: None,
            session_policy: SessionPolicy::New,
            conversation_mode: future_tasks::ConversationMode::Workspace,
            thread_id: None,
            trigger_kind: TriggerKind::Manual,
            trigger_json: serde_json::json!({}),
            next_due_at: None,
            pending_request_at: None,
            pending_origin: None,
            pending_actor: None,
            session_retention: future_tasks::SessionRetention::Keep,
            created_at: 1,
            updated_at: 1,
            deleted_at: None,
        }
    }

    fn scheduler() -> Task {
        let mut t = task("scheduled");
        t.trigger_kind = TriggerKind::Schedule;
        t.trigger_json = serde_json::json!({"mode":"daily","time":"09:00"});
        t.next_due_at = Some(1);
        t
    }

    /// A schedule claim advances the schedule forward and leaves no pending
    /// request behind, so the next tick does not run it again.
    #[test]
    fn claim_for_a_schedule_advances_next_due_and_clears_pending() {
        let (_dir, store) = store();
        let t = scheduler();
        store.insert_task(&t).unwrap();

        let run = claim(&store, &t, ClaimReason::Schedule).unwrap();
        assert_eq!(run.kind, RunKind::Main);
        assert_eq!(run.origin, RunOrigin::Schedule);
        assert_eq!(run.status, RunStatus::Running);

        let after = store.get_task(&t.id).unwrap().unwrap();
        assert!(after.next_due_at.unwrap() > t.next_due_at.unwrap());
        assert!(after.pending_request_at.is_none());
    }

    /// An explicit (manual/UI/CLI) claim consumes the pending request. It must
    /// NOT touch `next_due_at`: pressing "run now" is not a scheduled slot.
    #[test]
    fn claim_for_a_pending_request_clears_pending_and_keeps_the_schedule() {
        let (_dir, store) = store();
        let mut t = scheduler();
        t.pending_request_at = Some(1_000);
        t.pending_origin = Some(RunOrigin::Cli);
        t.pending_actor = Some("cli".into());
        store.insert_task(&t).unwrap();

        let run = claim(&store, &t, ClaimReason::Explicit).unwrap();
        assert_eq!(run.kind, RunKind::Manual);
        assert_eq!(run.origin, RunOrigin::Cli);
        assert!(run.due_at.is_none());

        let after = store.get_task(&t.id).unwrap().unwrap();
        assert!(after.pending_request_at.is_none());
        assert_eq!(after.next_due_at, t.next_due_at, "run-now is not a slot");
    }

    // ─── the executor (mock agent) ────────────────────────────────────────

    /// Script the commands a run sends before its stream: session creation,
    /// the permission/sandbox push, and the idle probe.
    fn script_agent_setup(mock: &crate::agent_bridge::test_support::MockAgentGuard, session: &str) {
        mock.push_data("new_session", serde_json::json!({ "sessionId": session }));
        mock.push_data("set_permission_level", serde_json::json!({}));
        mock.push_data(
            "set_sandbox_policy",
            serde_json::json!({ "sandboxAvailable": true }),
        );
        // `get_state` must be typed: the idle probe decodes the typed payload.
        // Repeated idle probes may be needed when reusing a conversation.
        for _ in 0..4 {
            mock.push_typed_data(
                "get_state",
                crate::agent_bridge::test_support::get_state_payload(session, false),
            );
        }
        mock.push_data(
            "get_last_assistant_text",
            serde_json::json!({ "text": "the finished answer" }),
        );
    }

    /// A whole run: the task opens a conversation, drives one prompt turn, and
    /// records the agent's answer on the run row.
    #[tokio::test]
    async fn a_run_opens_a_conversation_prompts_and_records_the_answer() {
        let _home = crate::agent_bridge::test_support::TestHome::new("tasks-execute");
        let mock = crate::agent_bridge::test_support::mock_agent();
        let dir = TempDir::new().unwrap();
        let store = Store::open(dir.path()).unwrap();

        let mut t = task("runs");
        t.prompt = "summarize yesterday".into();
        store.insert_task(&t).unwrap();
        let run = claim(&store, &t, ClaimReason::Explicit).unwrap();
        script_agent_setup(&mock, "sess_task_1");

        let notified = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = notified.clone();
        let notify: Notifier = std::sync::Arc::new(move |thread_id| {
            seen.lock().unwrap().push(thread_id.map(str::to_string));
        });

        // The prompt stream is what `run_prepared_prompt_*` collects; a clean
        // `agent_end` is what makes the run complete.
        // `@attach` binds to the canonical run id the prompt pipeline picks,
        // which the desktop generates at prompt time.
        mock.push_stream(crate::agent_bridge::test_support::StreamScript::Events(
            vec![crate::agent_bridge::test_support::stream_event(
                "@attach",
                0,
                "agent_end",
                r#"{"reason":"complete"}"#,
            )],
            None,
        ));

        execute(&notify, store, t.clone(), run.clone())
            .await
            .expect("execute");

        // Re-open the store the same way the tick loop does.
        let store = Store::open(dir.path()).unwrap();
        let finished = store.get_run(&run.id).unwrap().unwrap();
        assert_eq!(finished.status, RunStatus::Completed, "{finished:?}");
        assert!(finished.thread_id.is_some(), "the run owns a conversation");
        assert_eq!(finished.session_id.as_deref(), Some("sess_task_1"));
        assert_eq!(
            finished.result_summary.as_deref(),
            Some("the finished answer")
        );
        // The answer is saved on the run itself, whole, not only as the
        // truncated summary the lists carry.
        assert_eq!(
            store.run_output(&run.id).unwrap().as_deref(),
            Some("the finished answer"),
            "the run keeps its own answer"
        );
        assert!(!finished.session_deleted);
        assert!(finished.error_message.is_none());

        // The prompt carried the envelope, and the task's own text follows it.
        let prompts = mock.requests_of("prompt");
        let prompt = prompts.first().expect("one prompt");
        assert!(
            prompt
                .message
                .contains(future_tasks::TASK_ENVELOPE_SCHEMA_VERSION),
            "{}",
            prompt.message
        );
        assert!(
            prompt.message.contains("Instruction:\n"),
            "{}",
            prompt.message
        );
        assert!(
            prompt.message.contains("Completion contract:"),
            "{}",
            prompt.message
        );
        assert!(
            prompt.message.contains("summarize yesterday"),
            "{}",
            prompt.message
        );

        // The conversation is a real desktop thread the sidebar can show.
        let thread_id = finished.thread_id.clone().unwrap();
        let thread = crate::store::get_thread(&thread_id).unwrap().unwrap();
        assert_eq!(thread.agent_session_id.as_deref(), Some("sess_task_1"));
        assert!(thread.title.starts_with("runs · "), "{}", thread.title);

        // The host was told, so the UI can refresh.
        assert_eq!(
            notified.lock().unwrap().as_slice(),
            &[Some(thread_id)],
            "the notifier receives the conversation id"
        );
    }

    /// `session_retention = delete`: the run settles, its answer is saved on the
    /// ledger row, and the conversation goes — so a task that runs often does
    /// not leave one conversation per run behind without losing what each run
    /// did.
    #[tokio::test]
    async fn a_run_whose_task_deletes_its_conversation_keeps_the_answer() {
        let _home = crate::agent_bridge::test_support::TestHome::new("tasks-delete-conversation");
        let mock = crate::agent_bridge::test_support::mock_agent();
        let dir = TempDir::new().unwrap();
        let store = Store::open(dir.path()).unwrap();

        let mut t = task("ephemeral");
        t.prompt = "summarize yesterday".into();
        t.session_retention = future_tasks::SessionRetention::Delete;
        store.insert_task(&t).unwrap();
        let run = claim(&store, &t, ClaimReason::Explicit).unwrap();
        script_agent_setup(&mock, "sess_ephemeral");
        // The agent is asked to drop the session once the thread is gone.
        mock.push_data(
            "delete_sessions",
            serde_json::json!({"results": [{
                "sessionId": "sess_ephemeral", "deleted": true,
                "error": "", "errorCode": "", "errorData": null
            }]}),
        );
        mock.push_stream(crate::agent_bridge::test_support::StreamScript::Events(
            vec![crate::agent_bridge::test_support::stream_event(
                "@attach",
                0,
                "agent_end",
                r#"{"reason":"complete"}"#,
            )],
            None,
        ));

        let notified = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = notified.clone();
        let notify: Notifier = std::sync::Arc::new(move |thread_id| {
            seen.lock().unwrap().push(thread_id.map(str::to_string));
        });

        execute(&notify, store, t.clone(), run.clone())
            .await
            .expect("execute");

        let store = Store::open(dir.path()).unwrap();
        let finished = store.get_run(&run.id).unwrap().unwrap();
        assert_eq!(finished.status, RunStatus::Completed, "{finished:?}");
        // The run reads as it always did: status, summary, and the whole answer.
        assert_eq!(
            finished.result_summary.as_deref(),
            Some("the finished answer")
        );
        assert_eq!(
            store.run_output(&run.id).unwrap().as_deref(),
            Some("the finished answer"),
            "the answer is what survives the conversation"
        );
        // …and it no longer claims a conversation it does not have.
        assert!(finished.session_deleted, "{finished:?}");
        assert!(finished.thread_id.is_none());
        assert!(finished.session_id.is_none());
        assert!(
            crate::store::list_threads().unwrap().is_empty(),
            "the conversation is gone from the sidebar"
        );
        assert_eq!(
            mock.requests_of("delete_sessions").len(),
            1,
            "the Agent session behind it was dropped too"
        );
        // The host is told, with nothing to open: the panel shows the run and
        // says where the output went.
        assert_eq!(notified.lock().unwrap().as_slice(), &[None]);
    }

    /// A run that never reached an answer (the agent refused the session) has
    /// no conversation to delete, and the ledger says exactly that: no
    /// conversation, nothing deleted.
    #[tokio::test]
    async fn a_run_that_never_prompted_has_nothing_to_delete() {
        let _home = crate::agent_bridge::test_support::TestHome::new("tasks-delete-nothing");
        let mock = crate::agent_bridge::test_support::mock_agent();
        let dir = TempDir::new().unwrap();
        let store = Store::open(dir.path()).unwrap();

        let mut t = task("fails-before-prompting");
        t.session_retention = future_tasks::SessionRetention::Delete;
        store.insert_task(&t).unwrap();
        let run = claim(&store, &t, ClaimReason::Explicit).unwrap();
        mock.push(
            "new_session",
            crate::agent_bridge::test_support::Reply::Reject("no capacity".into()),
        );

        let notify: Notifier = std::sync::Arc::new(|_| {});
        execute(&notify, store, t.clone(), run.clone())
            .await
            .expect("execute records the failure instead of returning it");

        let store = Store::open(dir.path()).unwrap();
        let finished = store.get_run(&run.id).unwrap().unwrap();
        assert_eq!(finished.status, RunStatus::Failed, "{finished:?}");
        assert!(
            finished.thread_id.is_none() && finished.session_id.is_none(),
            "{finished:?}"
        );
        assert!(
            !finished.session_deleted,
            "there was no conversation to delete: {finished:?}"
        );
        assert_eq!(store.run_output(&run.id).unwrap(), None);
    }

    /// A task that reuses one conversation keeps it whatever the retention
    /// column says: the combination is contradictory, and deleting the
    /// conversation the next run continues would silently break the setting.
    #[tokio::test]
    async fn a_reused_conversation_is_never_deleted_after_a_run() {
        let _home = crate::agent_bridge::test_support::TestHome::new("tasks-delete-reused");
        let mock = crate::agent_bridge::test_support::mock_agent();
        let dir = TempDir::new().unwrap();
        let store = Store::open(dir.path()).unwrap();

        let mut t = task("reuses");
        t.session_policy = SessionPolicy::Existing;
        t.session_retention = future_tasks::SessionRetention::Delete;
        store.insert_task(&t).unwrap();
        let workspace = crate::agent_bridge::test_support::seed_workspace(_home.path(), "ws");
        let thread =
            crate::agent_bridge::test_support::seed_thread(&workspace.id, Some("sess_reused"));
        bind_task_thread(&store, &t, &thread.id).unwrap();
        let t = store.get_task(&t.id).unwrap().unwrap();

        let run = claim(&store, &t, ClaimReason::Explicit).unwrap();
        script_agent_setup(&mock, "sess_reused");
        mock.push_stream(crate::agent_bridge::test_support::StreamScript::Events(
            vec![crate::agent_bridge::test_support::stream_event(
                "@attach",
                0,
                "agent_end",
                r#"{"reason":"complete"}"#,
            )],
            None,
        ));

        let notify: Notifier = std::sync::Arc::new(|_| {});
        execute(&notify, store, t.clone(), run.clone())
            .await
            .expect("execute");

        let store = Store::open(dir.path()).unwrap();
        let finished = store.get_run(&run.id).unwrap().unwrap();
        assert_eq!(finished.status, RunStatus::Completed, "{finished:?}");
        assert!(!finished.session_deleted, "{finished:?}");
        assert_eq!(finished.thread_id.as_deref(), Some(thread.id.as_str()));
        assert!(
            crate::store::get_thread(&thread.id).unwrap().is_some(),
            "the conversation the next run continues is still there"
        );
    }

    /// A chat task needs no directory of its own: the conversation runs in the
    /// temporary workspace every chat conversation gets, so no `set_cwd` is
    /// sent (before this, the empty path was pushed at the agent and the run
    /// failed on a working directory nobody had asked for).
    #[tokio::test]
    async fn a_chat_task_without_a_directory_keeps_the_conversations_workspace() {
        let _home = crate::agent_bridge::test_support::TestHome::new("tasks-chat-no-cwd");
        let mock = crate::agent_bridge::test_support::mock_agent();
        let dir = TempDir::new().unwrap();
        let store = Store::open(dir.path()).unwrap();

        let mut t = task("chatty");
        t.conversation_mode = ConversationMode::Chat;
        t.cwd = String::new();
        store.insert_task(&t).unwrap();
        let run = claim(&store, &t, ClaimReason::Explicit).unwrap();
        script_agent_setup(&mock, "sess_chat_1");
        mock.push_stream(crate::agent_bridge::test_support::StreamScript::Events(
            vec![crate::agent_bridge::test_support::stream_event(
                "@attach",
                0,
                "agent_end",
                r#"{"reason":"complete"}"#,
            )],
            None,
        ));

        let notify: Notifier = std::sync::Arc::new(|_| {});
        execute(&notify, store, t.clone(), run.clone())
            .await
            .expect("execute");

        let store = Store::open(dir.path()).unwrap();
        let finished = store.get_run(&run.id).unwrap().unwrap();
        assert_eq!(finished.status, RunStatus::Completed, "{finished:?}");
        // Every working directory this run named is the conversation's own
        // temporary workspace — never an empty path (the bug: the empty
        // `task.cwd` used to be pushed at the agent) and never the task's
        // directory.
        let touched: Vec<String> = mock
            .requests_of("set_cwd")
            .iter()
            .map(|c| c.cwd.clone())
            .collect();
        for cwd in &touched {
            // Built from the platform's own separator: the stored path is a real
            // `Path`, so on Windows the components are split by `\`.
            let chat_root = format!(
                "workspaces{}chat{}",
                std::path::MAIN_SEPARATOR,
                std::path::MAIN_SEPARATOR
            );
            assert!(
                cwd.contains(&chat_root) && !cwd.trim().is_empty(),
                "not the conversation's workspace: {cwd}"
            );
        }
        // The conversation is filed under Chat, and its envelope says so — with
        // no empty `cwd=` in the settings line.
        let thread = crate::store::get_thread(finished.thread_id.as_deref().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(thread.mode, "chat");
        let prompt = mock
            .requests_of("prompt")
            .first()
            .expect("one prompt")
            .message
            .clone();
        assert!(prompt.contains("conversation=chat"), "{prompt}");
        assert!(!prompt.contains("cwd="), "{prompt}");
    }

    /// A workspace conversation *is* its directory: an empty one is refused in
    /// words rather than filed under whatever the process sits in.
    #[tokio::test]
    async fn a_workspace_task_without_a_directory_fails_in_words() {
        let _home = crate::agent_bridge::test_support::TestHome::new("tasks-workspace-no-cwd");
        let _mock = crate::agent_bridge::test_support::mock_agent();
        let dir = TempDir::new().unwrap();
        let store = Store::open(dir.path()).unwrap();

        let mut t = task("homeless");
        t.cwd = "   ".into();
        store.insert_task(&t).unwrap();
        let run = claim(&store, &t, ClaimReason::Explicit).unwrap();

        let notify: Notifier = std::sync::Arc::new(|_| {});
        execute(&notify, store, t.clone(), run.clone())
            .await
            .expect("execute");

        let store = Store::open(dir.path()).unwrap();
        let finished = store.get_run(&run.id).unwrap().unwrap();
        assert_eq!(finished.status, RunStatus::Failed, "{finished:?}");
        assert!(
            finished
                .error_message
                .as_deref()
                .unwrap_or_default()
                .contains("needs a working directory"),
            "{finished:?}"
        );
    }

    /// A run that fails to reach the agent is recorded as failed with the
    /// reason, and still marks only the edges its status satisfies.
    #[tokio::test]
    async fn a_run_without_an_agent_fails_with_the_reason() {
        let _home = crate::agent_bridge::test_support::TestHome::new("tasks-execute-fail");
        let mock = crate::agent_bridge::test_support::mock_agent();
        let dir = TempDir::new().unwrap();
        let store = Store::open(dir.path()).unwrap();

        let t = task("fails");
        store.insert_task(&t).unwrap();
        let run = claim(&store, &t, ClaimReason::Explicit).unwrap();
        // The agent refuses to create the session.
        mock.push(
            "new_session",
            crate::agent_bridge::test_support::Reply::Reject("no capacity".into()),
        );

        let notify: Notifier = std::sync::Arc::new(|_| {});
        execute(&notify, store, t, run.clone())
            .await
            .expect("execute records the failure instead of returning it");

        let store = Store::open(dir.path()).unwrap();
        let finished = store.get_run(&run.id).unwrap().unwrap();
        assert_eq!(finished.status, RunStatus::Failed);
        assert!(
            finished
                .error_message
                .as_deref()
                .unwrap_or_default()
                .contains("no capacity"),
            "{finished:?}"
        );
        assert!(finished.thread_id.is_none(), "no conversation was created");
    }

    /// A reused conversation is prompted in place, retaining earlier context.
    #[tokio::test]
    async fn a_reused_conversation_is_prompted_in_place_without_compacting() {
        let _home = crate::agent_bridge::test_support::TestHome::new("tasks-execute-reuse");
        let mock = crate::agent_bridge::test_support::mock_agent();
        let dir = TempDir::new().unwrap();
        let store = Store::open(dir.path()).unwrap();

        // An existing task bound to a conversation. The binding is in place
        // before the claim, exactly as a task that ran once already would be.
        let mut t = task("reuses");
        t.session_policy = SessionPolicy::Existing;
        store.insert_task(&t).unwrap();
        let workspace = crate::agent_bridge::test_support::seed_workspace(_home.path(), "ws");
        let thread =
            crate::agent_bridge::test_support::seed_thread(&workspace.id, Some("sess_reused"));
        bind_task_thread(&store, &t, &thread.id).unwrap();
        let t = store.get_task(&t.id).unwrap().unwrap();
        let run = claim(&store, &t, ClaimReason::Explicit).unwrap();
        script_agent_setup(&mock, "sess_reused");
        mock.push_stream(crate::agent_bridge::test_support::StreamScript::Events(
            vec![crate::agent_bridge::test_support::stream_event(
                "@attach",
                0,
                "agent_end",
                r#"{"reason":"complete"}"#,
            )],
            None,
        ));

        let notify: Notifier = std::sync::Arc::new(|_| {});
        execute(&notify, store, t.clone(), run.clone())
            .await
            .expect("execute");

        let store = Store::open(dir.path()).unwrap();
        let finished = store.get_run(&run.id).unwrap().unwrap();
        assert_eq!(finished.status, RunStatus::Completed, "{finished:?}");
        // The run continues the bound conversation…
        assert_eq!(finished.thread_id.as_deref(), Some(thread.id.as_str()));
        assert_eq!(finished.session_id.as_deref(), Some("sess_reused"));
        // …and does not spend a compaction pass on it: continuity is the point
        // of reusing a conversation, and a compaction drops the context the run
        // came back for.
        assert!(
            mock.requests_of("compact").is_empty(),
            "a reused conversation must not be compacted: {:?}",
            mock.requests_of("compact")
        );
        assert_eq!(mock.requests_of("prompt").len(), 1);
    }

    /// Overlap: a task with a live run is not claimed again.
    #[test]
    fn a_running_run_blocks_another_claim() {
        let (_dir, store) = store();
        let t = task("busy");
        store.insert_task(&t).unwrap();
        assert!(!store.has_running_run(&t.id).unwrap());
        store
            .insert_run(&future_tasks::TaskRun {
                id: future_tasks::new_run_id(),
                task_id: t.id.clone(),
                kind: RunKind::Main,
                origin: RunOrigin::Schedule,
                actor: None,
                due_at: Some(1),
                status: RunStatus::Running,
                thread_id: None,
                session_id: None,
                run_id: None,
                prompt_version: Some(1),
                result_summary: None,
                feedback: None,
                feedback_note: None,
                started_at: Some(1),
                finished_at: None,
                error_message: None,
                session_deleted: false,
            })
            .unwrap();
        assert!(store.has_running_run(&t.id).unwrap());
    }

    // ─── the real-agent harnesses ─────────────────────────────────────────

    /// The whole path against a **real** agent and a real model: a queued task is
    /// claimed by a tick and run.
    ///
    /// Ignored by default: it spends credit and needs an instance nobody else is
    /// using. Point `HOME` at a throwaway directory and start an agent for it
    /// (the test refuses to run against a home outside the temp directory):
    ///
    /// ```text
    /// HOME=<throwaway> FUTURE_AGENT_GRPC_ADDR=127.0.0.1:<port> \
    ///   cargo test --lib tasks::tests::a_queued_run_against_a_real_agent \
    ///   -- --ignored --nocapture --test-threads=1
    /// ```
    ///
    /// Everything the mocks cannot prove lives here: that the envelope reaches a
    /// real model in the shape the kernel composes, and that what comes back is
    /// recorded as the run's summary.
    #[tokio::test]
    #[ignore = "needs a real agent + model; see the doc comment"]
    async fn a_queued_run_against_a_real_agent_ends_with_a_usable_answer() {
        let home = crate::future_home_root();
        let spelled = home.display().to_string();
        assert!(
            home.starts_with(std::env::temp_dir())
                || spelled.starts_with("/tmp/")
                || spelled.starts_with("/private/tmp/"),
            "refusing to spend credit against a home that is not a throwaway: {spelled}"
        );
        // The host does this at startup; a bare `Store` on a fresh home has no
        // schema, and "no such table: workspaces" is what a run reports then.
        crate::store::initialize_app_store().expect("app store");
        let store = Store::open(&home).unwrap();
        let stamp = now_ms();
        let mut t = task(&format!("real-{stamp}"));
        t.prompt = "Write the single word: landed. Nothing else.".into();
        t.cwd = std::env::temp_dir().display().to_string();
        t.model_id = Some("future/deepseek-flash".into());
        t.thinking_level = Some("minimal".into());
        t.pending_request_at = Some(stamp);
        t.pending_origin = Some(RunOrigin::Cli);
        t.pending_actor = Some("smoke".into());
        store.insert_task(&t).unwrap();

        let notify: Notifier = std::sync::Arc::new(|_| {});
        tick(&notify).await.expect("tick claims the queued task");

        // The run itself.
        let settled = wait_for_run(&home, &t.id, RunKind::Manual).await;
        println!("RUN status={:?}", settled.status);
        println!("RUN summary={:?}", settled.result_summary);
        println!("RUN error={:?}", settled.error_message);
        assert_eq!(settled.status, RunStatus::Completed, "{settled:?}");
        let summary = settled.result_summary.clone().unwrap_or_default();
        assert!(
            summary.to_lowercase().contains("landed"),
            "a real model answered the run: {summary:?}"
        );
    }

    /// A one-shot host for hands-on testing: tick until nothing is pending or
    /// running, so a run queued from the CLI (`future task run --wait`) actually
    /// gets executed. Ignored by default like the smoke test above.
    ///
    /// `futureos-headless` is the real host, but it insists on a platform login
    /// and starts the remote bridge; this is the same `tick`, without either. It
    /// reconciles runs left running by a host that exited mid-flight, as
    /// `run_loop` does.
    #[tokio::test]
    #[ignore = "a manual harness: ticks a throwaway instance until it is idle"]
    async fn serve_pending_runs_until_idle() {
        let home = crate::future_home_root();
        crate::store::initialize_app_store().expect("app store");
        let notify: Notifier = std::sync::Arc::new(|_| {});
        let _ = Store::open(&home).unwrap().mark_interrupted_runs(now_ms());
        let deadline = std::time::Instant::now() + Duration::from_secs(900);
        loop {
            tick(&notify).await.expect("tick");
            let busy = {
                let store = Store::open(&home).unwrap();
                store.list_tasks(false).unwrap().into_iter().any(|task| {
                    let runs = store.list_runs_for_task(&task.id, 5).unwrap_or_default();
                    task.pending_request_at.is_some()
                        || task.next_due_at.is_some_and(|due| due <= now_ms())
                        || runs.iter().any(|run| run.status == RunStatus::Running)
                })
            };
            if !busy {
                let store = Store::open(&home).unwrap();
                for task in store.list_tasks(false).unwrap() {
                    for run in store.list_runs_for_task(&task.id, 3).unwrap() {
                        println!(
                            "{} · {:?} · {:?} · {:?}",
                            task.name, run.kind, run.status, run.result_summary
                        );
                    }
                }
                println!("idle");
                return;
            }
            if std::time::Instant::now() > deadline {
                panic!("still busy after 15 minutes");
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Wait for one of a task's runs to reach a terminal state.
    async fn wait_for_run(home: &std::path::Path, task_id: &str, kind: RunKind) -> TaskRun {
        let deadline = std::time::Instant::now() + Duration::from_secs(600);
        loop {
            {
                let store = Store::open(home).unwrap();
                if let Some(settled) = store
                    .list_runs_for_task(task_id, 10)
                    .unwrap()
                    .into_iter()
                    .find(|run| run.kind == kind && run.status != RunStatus::Running)
                {
                    return settled;
                }
            }
            if std::time::Instant::now() > deadline {
                panic!("the {kind:?} run never settled");
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
}
