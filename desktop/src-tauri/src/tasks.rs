//! Desktop host for `future-tasks`.
//!
//! The deterministic kernel and store live in the `future-tasks` crate; this
//! module is the desktop's tick loop plus the `Executor` wiring that reaches
//! the sidecar agent, creates threads, and notifies the webview. Headless
//! desktop reuses the same executor with notifications downgraded to ledger
//! writes (no webview).

use std::path::PathBuf;
use std::time::Duration;

use future_tasks::{
    compose_run_prompt, kernel, new_run_id, ConversationMode, RunKind, RunOrigin, RunStatus,
    SessionPolicy, Store, Task, TaskRun, UpstreamSource,
};

/// Tick cadence. The store is re-read every tick, so CLI/remote edits land
/// without any cache invalidation.
const TICK_INTERVAL: Duration = Duration::from_secs(30);
/// Pre-compact bound for `session_policy=existing` runs. A compaction that
/// overruns fails the run rather than silently skipping the step.
const PRE_COMPACT_TIMEOUT: Duration = Duration::from_secs(120);
/// Poll cadence while waiting for the agent to go idle / compaction to settle.
const AGENT_POLL_INTERVAL: Duration = Duration::from_millis(200);

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
    tauri::async_runtime::spawn(run_loop(notify));
}

/// Headless variant: same loop, notifications downgraded to nothing (the run
/// ledger in `tasks.db` is the observable signal).
pub fn start_headless() {
    let notify: Notifier = std::sync::Arc::new(|_| {});
    tokio::spawn(run_loop(notify));
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
    /// A dependency join came true (a chain wakeup).
    Chain,
    /// Someone asked for it: the UI button, the phone, or `future task run`.
    Explicit,
    /// The task's own schedule reached its next slot.
    Schedule,
}

/// Which tasks run this tick, and why. At most one reason per task, so a task
/// that is due *and* has a satisfied join runs once, not twice.
///
/// Dependency joins are evaluated here rather than pushed when the upstream
/// finishes. A push would be a second write path that misses the cases where
/// the marks stay behind — a manual run of the downstream leaves them intact —
/// and this way the tick is the single place that decides what runs.
fn candidates(store: &Store, now: i64) -> Result<Vec<(Task, ClaimReason)>, String> {
    let due = store.list_due_tasks(now).map_err(|e| e.to_string())?;
    let mut out: Vec<(Task, ClaimReason)> = due
        .into_iter()
        .map(|task| {
            // An explicit request outranks the schedule: the user asked for a
            // run *now*, and the run carries whatever the join satisfied.
            let reason = if task.pending_request_at.is_some() {
                ClaimReason::Explicit
            } else {
                ClaimReason::Schedule
            };
            (task, reason)
        })
        .collect();

    // Chain wakeups: every enabled task with at least one dependency edge whose
    // join is satisfied, and which is not already a candidate above.
    let all_deps = store.list_all_deps().map_err(|e| e.to_string())?;
    let downstream: std::collections::HashSet<&str> =
        all_deps.iter().map(|d| d.task_id.as_str()).collect();
    for id in downstream {
        if out.iter().any(|(t, _)| t.id == id) {
            continue;
        }
        let task = match store.get_task(id).map_err(|e| e.to_string())? {
            Some(task) => task,
            None => continue,
        };
        if !task.enabled || task.deleted_at.is_some() {
            continue;
        }
        if join_is_satisfied(store, &task)? {
            out.push((task, ClaimReason::Chain));
        }
    }
    Ok(out)
}

/// Whether every edge the task's join policy requires has a satisfied mark.
fn join_is_satisfied(store: &Store, task: &Task) -> Result<bool, String> {
    let deps = store.list_deps(&task.id).map_err(|e| e.to_string())?;
    let states = store.list_dep_states(&task.id).map_err(|e| e.to_string())?;
    Ok(kernel::join_claim(task, &deps, &states).is_some())
}

/// Which tasks run this tick, as `(task_id, reason)` pairs. Test-only view of
/// [`candidates`], for the command-level tests that assert a delete stops a
/// chain rather than leaving it armed.
#[cfg(test)]
pub(crate) fn candidates_for_test(
    store: &Store,
    now: i64,
) -> Result<Vec<(String, String)>, String> {
    Ok(candidates(store, now)?
        .into_iter()
        .map(|(task, reason)| {
            let label = match reason {
                ClaimReason::Chain => "chain",
                ClaimReason::Explicit => "explicit",
                ClaimReason::Schedule => "schedule",
            };
            (task.id, label.to_string())
        })
        .collect())
}

/// One tick: claim due tasks, execute them, and let their completion mark
/// downstream dependency edges. Sequential within one host (the single-writer
/// discipline that keeps dep claims race-free); each task spawns its own run
/// so a slow task does not block the next tick.
async fn tick(notify: &Notifier) -> Result<(), String> {
    let store = Store::open(&future_home()).map_err(|e| e.to_string())?;
    for (task, reason) in candidates(&store, now_ms())? {
        if store.has_running_run(&task.id).map_err(|e| e.to_string())? {
            continue; // overlap: skip this tick, the next one retries
        }
        let (run, upstream_sources) = claim(&store, &task, reason)?;
        let notify = notify.clone();
        tokio::spawn(async move {
            // A fresh store per run: `rusqlite::Connection` is not `Sync`, so
            // the tick's store cannot cross the spawn boundary.
            let store = match Store::open(&future_home()) {
                Ok(s) => s,
                Err(error) => {
                    eprintln!("FutureOS tasks: could not open store for run: {error}");
                    return;
                }
            };
            if let Err(error) = execute(&notify, store, task, run, upstream_sources).await {
                eprintln!("FutureOS task execution failed: {error}");
            }
        });
    }
    Ok(())
}

/// Claim one due task: insert the run row (running), advance `next_due_at`,
/// clear `pending_request_at`, and consume satisfied dependency marks.
/// Returns the run row plus the upstream sources for the envelope.
fn claim(
    store: &Store,
    task: &Task,
    reason: ClaimReason,
) -> Result<(TaskRun, Vec<UpstreamSource>), String> {
    let now = now_ms();
    // The reason picks the run's label: a chain wakeup is its own kind, an
    // explicit request keeps whatever origin asked for it, a schedule slot is
    // `main`. A schedule claim resolves its `due_at` to the slot it fired for.
    let (kind, origin, due_at) = match reason {
        ClaimReason::Chain => (RunKind::Chain, RunOrigin::Chain, None),
        ClaimReason::Explicit => (
            RunKind::Manual,
            task.pending_origin.unwrap_or(RunOrigin::Cli),
            None,
        ),
        ClaimReason::Schedule => (RunKind::Main, RunOrigin::Schedule, task.next_due_at),
    };

    // Cycle check (defense in depth; add/edit also checks). A cycle makes the
    // chain claim fail rather than loop forever.
    if kind == RunKind::Chain {
        let all_deps = store.list_all_deps().map_err(|e| e.to_string())?;
        if kernel::would_cycle(&task.id, &all_deps) {
            let run = TaskRun {
                id: new_run_id(),
                task_id: task.id.clone(),
                kind,
                origin,
                actor: task.pending_actor.clone(),
                due_at,
                status: RunStatus::Failed,
                thread_id: None,
                session_id: None,
                run_id: None,
                prompt_version: Some(task.prompt_version),
                result_summary: None,
                feedback: None,
                feedback_note: None,
                started_at: Some(now),
                finished_at: Some(now),
                error_message: Some("dependency cycle detected".to_string()),
            };
            store.insert_run(&run).map_err(|e| e.to_string())?;
            clear_pending(store, task)?;
            return Err("dependency cycle".to_string());
        }
    }

    // Whatever the reason, a claim consumes the dependency marks that are
    // satisfied right now, and the run carries those upstream results. The
    // alternative — leaving them for a chain run — runs the task twice for one
    // signal, which is what "run now" on a task with satisfied deps used to do.
    let mut consume: Vec<(String, String)> = Vec::new();
    let mut upstream_sources = Vec::new();
    {
        let deps = store.list_deps(&task.id).map_err(|e| e.to_string())?;
        if !deps.is_empty() {
            let states = store.list_dep_states(&task.id).map_err(|e| e.to_string())?;
            if let Some(c) = kernel::join_claim(task, &deps, &states) {
                for (upstream_id, run_id) in &c {
                    if let (Some(upstream), Some(run)) = (
                        store.get_task(upstream_id).map_err(|e| e.to_string())?,
                        store.get_run(run_id).map_err(|e| e.to_string())?,
                    ) {
                        upstream_sources.push(UpstreamSource {
                            task_id: upstream.id.clone(),
                            task_name: upstream.name.clone(),
                            run_id: run.id.clone(),
                            status: run.status,
                            finished_at: run.finished_at,
                            result_summary: run.result_summary.clone(),
                        });
                    }
                }
                consume = c;
            }
        }
    }

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

    // Chain consumption happens only after the run row exists, so a crash
    // between insert and clear does not lose the upstream signal.
    if !consume.is_empty() {
        store
            .clear_dep_states(&task.id, &consume)
            .map_err(|e| e.to_string())?;
    }

    Ok((run, upstream_sources))
}

fn clear_pending(store: &Store, task: &Task) -> Result<(), String> {
    let mut t = task.clone();
    t.pending_request_at = None;
    t.pending_origin = None;
    t.pending_actor = None;
    t.updated_at = now_ms();
    store.update_task(&t).map_err(|e| e.to_string())
}

/// Execute one claimed run: session → pre-compact → envelope → prompt →
/// finalize. On completion, mark downstream dependency edges satisfied.
async fn execute(
    notify: &Notifier,
    store: Store,
    task: Task,
    mut run: TaskRun,
    upstream_sources: Vec<UpstreamSource>,
) -> Result<(), String> {
    // `Store` owns a `rusqlite::Connection` (Send but not Sync), so it is
    // moved through the async call and handed back rather than borrowed.
    let (store, outcome) = run_inner(store, &task, &mut run, upstream_sources).await;
    let (status, error) = match outcome {
        Ok(summary) => {
            run.result_summary = Some(kernel::truncate(&summary, kernel::RESULT_SUMMARY_CHARS));
            (RunStatus::Completed, None)
        }
        Err(error) => (RunStatus::Failed, Some(error)),
    };
    run.status = status;
    run.finished_at = Some(now_ms());
    run.error_message = error;
    store.update_run(&run).map_err(|e| e.to_string())?;

    // Mark downstream dependency edges (same store transaction boundary as
    // the run row update — a crash here does not lose the upstream signal).
    let downstream = store.list_all_deps().map_err(|e| e.to_string())?;
    for dep in downstream.iter().filter(|d| d.upstream_task_id == task.id) {
        if kernel::run_satisfies(dep.on, status) {
            store
                .mark_dep_satisfied(&dep.task_id, &task.id, &run.id, now_ms())
                .map_err(|e| e.to_string())?;
        }
    }

    // Notify the host (webview refresh; headless downgrades to a no-op).
    notify(run.thread_id.as_deref());
    Ok(())
}

/// The inner run: session, pre-compact, prompt. Returns the final assistant
/// text on success.
async fn run_inner(
    store: Store,
    task: &Task,
    run: &mut TaskRun,
    upstream_sources: Vec<UpstreamSource>,
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

    // 2. Pre-compact (existing sessions only).
    if task.session_policy == SessionPolicy::Existing {
        if let Err(error) = pre_compact(&session_id).await {
            return (store, Err(error));
        }
    }

    // 3. Envelope + upstream evidence + instruction + completion contract.
    let prompt = compose_run_prompt(task, run.kind, run.due_at, None, &upstream_sources);

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
    let handle = tokio::spawn(crate::agent_bridge::run_prepared_prompt_with_acceptance(
        prepared, tx,
    ));
    let _ = rx.await;
    let prompt_result = handle
        .await
        .map_err(|e| format!("prompt task panicked: {e}"))
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

/// Pre-compact an existing session before the run. Compaction is asynchronous
/// (accepted ack + worker events); we wait for it to settle before prompting.
/// Failure fails the run (the user asked for compact-then-run, not run).
async fn pre_compact(session_id: &str) -> Result<(), String> {
    wait_for_agent_idle(session_id)
        .await
        .map_err(|e| e.to_string())?;
    let ack = crate::agent_bridge::compact_agent_session(session_id.to_string())
        .await
        .map_err(|e| e.to_string())?;
    let operation_id = ack
        .get("operationId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "compact returned no operationId".to_string())?
        .to_string();
    wait_for_compaction(session_id, &operation_id).await
}

async fn wait_for_agent_idle(session_id: &str) -> Result<(), String> {
    let deadline = std::time::Instant::now() + PRE_COMPACT_TIMEOUT;
    loop {
        if std::time::Instant::now() > deadline {
            return Err("agent did not go idle before pre-compact".to_string());
        }
        if crate::agent_bridge::wait_for_agent_idle(session_id).await {
            return Ok(());
        }
        tokio::time::sleep(AGENT_POLL_INTERVAL).await;
    }
}

async fn wait_for_compaction(session_id: &str, _operation_id: &str) -> Result<(), String> {
    // The compaction worker emits `compaction_unchanged` / `compaction_failed`
    // on the session broadcaster; the simplest robust wait is to poll the
    // session state until it is idle again (the worker holds the busy flag).
    let deadline = std::time::Instant::now() + PRE_COMPACT_TIMEOUT;
    loop {
        if std::time::Instant::now() > deadline {
            return Err("pre-compact timed out".to_string());
        }
        if crate::agent_bridge::wait_for_agent_idle(session_id).await {
            return Ok(());
        }
        tokio::time::sleep(AGENT_POLL_INTERVAL).await;
    }
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
    use future_tasks::{new_task_id, DepJoin, DepOn, Reflection, Task, TaskDep, TriggerKind};
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
            dep_join: DepJoin::All,
            next_due_at: None,
            pending_request_at: None,
            pending_origin: None,
            pending_actor: None,
            reflection: Reflection::Ask,
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

    /// A terminal run row for `task_id`, the shape a dependency edge records.
    fn finished_run(task_id: &str, run_id: &str, status: RunStatus) -> TaskRun {
        TaskRun {
            id: run_id.to_string(),
            task_id: task_id.to_string(),
            kind: RunKind::Main,
            origin: RunOrigin::Schedule,
            actor: None,
            due_at: Some(1),
            status,
            thread_id: None,
            session_id: None,
            run_id: None,
            prompt_version: Some(1),
            result_summary: Some("upstream landed".into()),
            feedback: None,
            feedback_note: None,
            started_at: Some(1),
            finished_at: Some(2),
            error_message: None,
        }
    }

    /// A schedule claim advances the schedule forward and leaves no pending
    /// request behind, so the next tick does not run it again.
    #[test]
    fn claim_for_a_schedule_advances_next_due_and_clears_pending() {
        let (_dir, store) = store();
        let t = scheduler();
        store.insert_task(&t).unwrap();

        let (run, upstream) = claim(&store, &t, ClaimReason::Schedule).unwrap();
        assert!(upstream.is_empty());
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

        let (run, _) = claim(&store, &t, ClaimReason::Explicit).unwrap();
        assert_eq!(run.kind, RunKind::Manual);
        assert_eq!(run.origin, RunOrigin::Cli);
        assert!(run.due_at.is_none());

        let after = store.get_task(&t.id).unwrap().unwrap();
        assert!(after.pending_request_at.is_none());
        assert_eq!(after.next_due_at, t.next_due_at, "run-now is not a slot");
    }

    /// A manual claim consumes the dependency marks it found satisfied, and the
    /// run carries those upstream results. Leaving them for a later chain run
    /// would run the task twice for one signal — press "run now" on a task with
    /// satisfied deps and you would get the run you asked for *and* a chain run
    /// behind it.
    #[test]
    fn a_manual_claim_consumes_satisfied_marks_and_carries_the_upstream_results() {
        let (_dir, store) = store();
        let upstream = task("upstream");
        let mut downstream = task("downstream");
        downstream.pending_request_at = Some(1_000);
        downstream.pending_origin = Some(RunOrigin::Ui);
        store.insert_task(&upstream).unwrap();
        store.insert_task(&downstream).unwrap();
        store
            .add_dep(&TaskDep {
                task_id: downstream.id.clone(),
                upstream_task_id: upstream.id.clone(),
                on: DepOn::Success,
            })
            .unwrap();
        let finished = finished_run(&upstream.id, "trn_done", RunStatus::Completed);
        store.insert_run(&finished).unwrap();
        store
            .mark_dep_satisfied(&downstream.id, &upstream.id, "trn_done", 1)
            .unwrap();

        let (run, sources) = claim(&store, &downstream, ClaimReason::Explicit).unwrap();
        assert_eq!(
            run.kind,
            RunKind::Manual,
            "the user's request drove this run"
        );
        assert_eq!(run.origin, RunOrigin::Ui);
        assert_eq!(
            sources.len(),
            1,
            "the consumed upstream travels with the run"
        );
        assert_eq!(sources[0].run_id, "trn_done");
        let state = store
            .get_dep_state(&downstream.id, &upstream.id)
            .unwrap()
            .unwrap();
        assert!(
            state.satisfied_run_id.is_none(),
            "the signal was used by this run, so it does not fire again"
        );
    }

    /// A dependency wakeup reaches the tick: a downstream task whose join is
    /// satisfied is a candidate even though nothing asked for it and it has no
    /// schedule of its own. This is the link that was missing — the edges and
    /// their marks were written, but nothing ever looked at them.
    #[test]
    fn a_satisfied_join_makes_the_downstream_a_candidate() {
        let (_dir, store) = store();
        let upstream = task("upstream");
        let downstream = task("downstream");
        store.insert_task(&upstream).unwrap();
        store.insert_task(&downstream).unwrap();
        store
            .add_dep(&TaskDep {
                task_id: downstream.id.clone(),
                upstream_task_id: upstream.id.clone(),
                on: DepOn::Success,
            })
            .unwrap();

        // Nothing satisfied yet: the downstream is untouched.
        let before = candidates(&store, 1_000).unwrap();
        assert!(
            !before.iter().any(|(t, _)| t.id == downstream.id),
            "an unsatisfied join must not fire"
        );

        store
            .mark_dep_satisfied(&downstream.id, &upstream.id, "trn_done", 1)
            .unwrap();
        let after = candidates(&store, 1_000).unwrap();
        let found = after
            .iter()
            .find(|(t, _)| t.id == downstream.id)
            .expect("a satisfied join is a chain candidate");
        assert_eq!(found.1, ClaimReason::Chain);
    }

    /// An `all` join waits for every edge. One satisfied edge is not enough,
    /// and the candidate appears only once the last one lands.
    #[test]
    fn an_all_join_waits_for_every_edge() {
        let (_dir, store) = store();
        let a = task("a");
        let b = task("b");
        let downstream = task("downstream");
        for t in [&a, &b, &downstream] {
            store.insert_task(t).unwrap();
        }
        for upstream in [&a, &b] {
            store
                .add_dep(&TaskDep {
                    task_id: downstream.id.clone(),
                    upstream_task_id: upstream.id.clone(),
                    on: DepOn::Success,
                })
                .unwrap();
        }
        store
            .mark_dep_satisfied(&downstream.id, &a.id, "trn_a", 1)
            .unwrap();
        assert!(
            !candidates(&store, 1_000)
                .unwrap()
                .iter()
                .any(|(t, _)| t.id == downstream.id),
            "one of two edges is not an `all` join"
        );

        store
            .mark_dep_satisfied(&downstream.id, &b.id, "trn_b", 2)
            .unwrap();
        let found = candidates(&store, 1_000)
            .unwrap()
            .into_iter()
            .find(|(t, _)| t.id == downstream.id)
            .expect("both edges satisfied");
        assert_eq!(found.1, ClaimReason::Chain);
    }

    /// An `any` join fires on the first edge to land.
    #[test]
    fn an_any_join_fires_on_the_first_edge() {
        let (_dir, store) = store();
        let a = task("a");
        let b = task("b");
        let mut downstream = task("downstream");
        downstream.dep_join = DepJoin::Any;
        for t in [&a, &b, &downstream] {
            store.insert_task(t).unwrap();
        }
        for upstream in [&a, &b] {
            store
                .add_dep(&TaskDep {
                    task_id: downstream.id.clone(),
                    upstream_task_id: upstream.id.clone(),
                    on: DepOn::Success,
                })
                .unwrap();
        }

        store
            .mark_dep_satisfied(&downstream.id, &a.id, "trn_a", 1)
            .unwrap();
        assert!(
            candidates(&store, 1_000)
                .unwrap()
                .iter()
                .any(|(t, _)| t.id == downstream.id),
            "one satisfied edge is enough for `any`"
        );
    }

    /// A disabled or deleted downstream is never woken: disabling a task is how
    /// you stop a chain without tearing it down. The live sibling is the
    /// control — without it this would pass just as well if nothing woke.
    #[test]
    fn a_disabled_or_deleted_downstream_is_not_woken() {
        let (_dir, store) = store();
        let upstream = task("upstream");
        let mut disabled = task("disabled");
        disabled.enabled = false;
        let mut deleted = task("deleted");
        deleted.deleted_at = Some(5);
        let live = task("live");
        for t in [&upstream, &disabled, &deleted, &live] {
            store.insert_task(t).unwrap();
        }
        for downstream in [&disabled, &deleted, &live] {
            store
                .add_dep(&TaskDep {
                    task_id: downstream.id.clone(),
                    upstream_task_id: upstream.id.clone(),
                    on: DepOn::Success,
                })
                .unwrap();
            store
                .mark_dep_satisfied(&downstream.id, &upstream.id, "trn_done", 1)
                .unwrap();
        }

        let found = candidates(&store, 1_000).unwrap();
        assert!(
            found.iter().any(|(t, _)| t.id == live.id),
            "the live downstream is woken, so the others are skips and not a dead path"
        );
        assert!(
            !found
                .iter()
                .any(|(t, _)| t.id == disabled.id || t.id == deleted.id),
            "only enabled, live tasks are woken"
        );
    }

    /// A task that is both schedule-due and chain-ready is one candidate with
    /// one reason, so it runs once. Two reasons would run it twice for the same
    /// moment in time. Its chain-only sibling proves the chain path is live.
    #[test]
    fn a_task_ready_for_two_reasons_is_one_candidate() {
        let (_dir, store) = store();
        let upstream = task("upstream");
        let mut both = scheduler();
        both.name = "both".into();
        let chain_only = task("chain_only");
        for t in [&upstream, &both, &chain_only] {
            store.insert_task(t).unwrap();
        }
        for downstream in [&both, &chain_only] {
            store
                .add_dep(&TaskDep {
                    task_id: downstream.id.clone(),
                    upstream_task_id: upstream.id.clone(),
                    on: DepOn::Success,
                })
                .unwrap();
            store
                .mark_dep_satisfied(&downstream.id, &upstream.id, "trn_done", 1)
                .unwrap();
        }

        let found: Vec<_> = candidates(&store, 10_000)
            .unwrap()
            .into_iter()
            .filter(|(t, _)| t.id == both.id || t.id == chain_only.id)
            .collect();
        assert!(
            found
                .iter()
                .any(|(t, r)| t.id == chain_only.id && *r == ClaimReason::Chain),
            "the chain path is live"
        );
        assert_eq!(
            found.iter().filter(|(t, _)| t.id == both.id).count(),
            1,
            "one run per task per tick"
        );
        assert_eq!(
            found.iter().find(|(t, _)| t.id == both.id).map(|(_, r)| *r),
            Some(ClaimReason::Schedule),
            "its own slot fired first"
        );
    }

    /// A chain claim consumes them and carries the upstream summaries into the
    /// envelope sources.
    #[test]
    fn a_chain_claim_consumes_marks_and_collects_upstream_sources() {
        let (_dir, store) = store();
        let upstream = task("upstream");
        store.insert_task(&upstream).unwrap();
        let upstream_run = store
            .list_runs_for_task(&upstream.id, 1)
            .unwrap()
            .into_iter()
            .next();
        assert!(upstream_run.is_none());

        // A finished upstream run is what a chain claim summarises.
        let finished = finished_run(
            &upstream.id,
            &future_tasks::new_run_id(),
            RunStatus::Completed,
        );
        store.insert_run(&finished).unwrap();

        let mut downstream = task("downstream");
        downstream.pending_request_at = Some(1_000);
        downstream.pending_origin = Some(RunOrigin::Chain);
        downstream.pending_actor = Some(format!("task:{}", upstream.id));
        store.insert_task(&downstream).unwrap();
        store
            .add_dep(&TaskDep {
                task_id: downstream.id.clone(),
                upstream_task_id: upstream.id.clone(),
                on: DepOn::Success,
            })
            .unwrap();
        store
            .mark_dep_satisfied(&downstream.id, &upstream.id, &finished.id, 2)
            .unwrap();

        let (run, sources) = claim(&store, &downstream, ClaimReason::Chain).unwrap();
        assert_eq!(run.kind, RunKind::Chain);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].task_name, "upstream");
        assert_eq!(
            sources[0].result_summary.as_deref(),
            Some("upstream landed")
        );

        let state = store
            .get_dep_state(&downstream.id, &upstream.id)
            .unwrap()
            .unwrap();
        assert!(
            state.satisfied_run_id.is_none(),
            "a chain claim must consume the edge it acted on"
        );
    }

    /// A dependency cycle fails the chain claim instead of running forever.
    #[test]
    fn a_cyclic_chain_claim_fails_instead_of_looping() {
        let (_dir, store) = store();
        let mut a = task("a");
        a.pending_request_at = Some(1);
        a.pending_origin = Some(RunOrigin::Chain);
        let mut b = task("b");
        b.pending_request_at = Some(1);
        b.pending_origin = Some(RunOrigin::Chain);
        store.insert_task(&a).unwrap();
        store.insert_task(&b).unwrap();
        store
            .add_dep(&TaskDep {
                task_id: b.id.clone(),
                upstream_task_id: a.id.clone(),
                on: DepOn::Success,
            })
            .unwrap();
        store
            .add_dep(&TaskDep {
                task_id: a.id.clone(),
                upstream_task_id: b.id.clone(),
                on: DepOn::Success,
            })
            .unwrap();

        let error = claim(&store, &a, ClaimReason::Chain).unwrap_err();
        assert!(error.contains("cycle"), "{error}");
        let run = store
            .list_runs_for_task(&a.id, 1)
            .unwrap()
            .into_iter()
            .next()
            .expect("the failed claim is recorded");
        assert_eq!(run.status, RunStatus::Failed);
        assert!(run.error_message.unwrap().contains("cycle"));
    }

    /// A finished run marks only the edges whose condition it satisfies.
    #[test]
    fn finishing_a_run_marks_only_matching_downstream_edges() {
        use future_tasks::kernel::run_satisfies;

        assert!(run_satisfies(DepOn::Completed, RunStatus::Completed));
        assert!(run_satisfies(DepOn::Completed, RunStatus::Failed));
        assert!(run_satisfies(DepOn::Success, RunStatus::Completed));
        assert!(!run_satisfies(DepOn::Success, RunStatus::Failed));
        assert!(run_satisfies(DepOn::Failure, RunStatus::Failed));
        assert!(!run_satisfies(DepOn::Failure, RunStatus::Completed));
        assert!(!run_satisfies(DepOn::Completed, RunStatus::Skipped));
    }

    // ─── the executor (mock agent) ────────────────────────────────────────

    /// Script the commands a run sends before its stream: session creation,
    /// the permission/sandbox push, and the compaction probe.
    fn script_agent_setup(mock: &crate::agent_bridge::test_support::MockAgentGuard, session: &str) {
        mock.push_data("new_session", serde_json::json!({ "sessionId": session }));
        mock.push_data("set_permission_level", serde_json::json!({}));
        mock.push_data(
            "set_sandbox_policy",
            serde_json::json!({ "sandboxAvailable": true }),
        );
        // `get_state` must be typed: the idle probe decodes the typed payload.
        // Several are queued because each phase (pre-compact idle, post-compact
        // settle) probes again.
        for _ in 0..4 {
            mock.push_typed_data(
                "get_state",
                crate::agent_bridge::test_support::get_state_payload(session, false),
            );
        }
        mock.push_data(
            "compact",
            serde_json::json!({ "accepted": true, "operationId": "cmp_1" }),
        );
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
        let (run, upstream) = claim(&store, &t, ClaimReason::Explicit).unwrap();
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

        execute(&notify, store, t.clone(), run.clone(), upstream)
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
        let (run, upstream) = claim(&store, &t, ClaimReason::Explicit).unwrap();
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
        execute(&notify, store, t.clone(), run.clone(), upstream)
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
            assert!(
                cwd.contains("workspaces/chat/") && !cwd.trim().is_empty(),
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
        let (run, upstream) = claim(&store, &t, ClaimReason::Explicit).unwrap();

        let notify: Notifier = std::sync::Arc::new(|_| {});
        execute(&notify, store, t.clone(), run.clone(), upstream)
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
        let (run, upstream) = claim(&store, &t, ClaimReason::Explicit).unwrap();
        // The agent refuses to create the session.
        mock.push(
            "new_session",
            crate::agent_bridge::test_support::Reply::Reject("no capacity".into()),
        );

        let notify: Notifier = std::sync::Arc::new(|_| {});
        execute(&notify, store, t, run.clone(), upstream)
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

    /// A reused conversation is compacted before the prompt, and a compaction
    /// that fails fails the run rather than prompting with stale context.
    #[tokio::test]
    async fn a_reused_conversation_is_compacted_and_a_failure_fails_the_run() {
        let _home = crate::agent_bridge::test_support::TestHome::new("tasks-execute-compact");
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
        let mut t = store.get_task(&t.id).unwrap().unwrap();
        let (run, upstream) = claim(&store, &t, ClaimReason::Explicit).unwrap();
        t.updated_at += 1;

        // The compact request is refused. The rejection is queued as the FIRST
        // compact reply (the queues are FIFO), so the failure belongs to the
        // pre-compact step rather than to the prompt.
        mock.push(
            "compact",
            crate::agent_bridge::test_support::Reply::Reject("compaction exploded".into()),
        );
        script_agent_setup(&mock, "sess_reused");

        let notify: Notifier = std::sync::Arc::new(|_| {});
        execute(&notify, store, t.clone(), run.clone(), upstream)
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
                .contains("compaction exploded"),
            "{finished:?}"
        );
        assert!(
            mock.requests_of("prompt").is_empty(),
            "a failed compaction must not be followed by a prompt"
        );
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
            })
            .unwrap();
        assert!(store.has_running_run(&t.id).unwrap());
    }
}
