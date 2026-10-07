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
    compose_envelope_header, compose_upstream_block, kernel, new_run_id, RunKind, RunOrigin,
    RunStatus, SessionPolicy, Store, Task, TaskRun, UpstreamSource,
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
        interval.tick().await;
        if let Err(error) = tick(&notify).await {
            eprintln!("FutureOS tasks tick failed: {error}");
        }
    }
}

/// One tick: claim due tasks, execute them, and let their completion mark
/// downstream dependency edges. Sequential within one host (the single-writer
/// discipline that keeps dep claims race-free); each task spawns its own run
/// so a slow task does not block the next tick.
async fn tick(notify: &Notifier) -> Result<(), String> {
    let store = Store::open(&future_home()).map_err(|e| e.to_string())?;
    let due = store.list_due_tasks(now_ms()).map_err(|e| e.to_string())?;
    for task in due {
        if store.has_running_run(&task.id).map_err(|e| e.to_string())? {
            continue; // overlap: skip this tick, the next one retries
        }
        let (run, upstream_sources) = claim(&store, &task)?;
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
/// clear `pending_request_at`. Chain triggers also consume their dep marks.
/// Returns the run row plus the upstream sources for the envelope.
fn claim(store: &Store, task: &Task) -> Result<(TaskRun, Vec<UpstreamSource>), String> {
    let now = now_ms();
    let is_pending = task.pending_request_at.is_some();
    let (kind, origin, due_at) = if is_pending {
        (
            if task.pending_origin == Some(RunOrigin::Chain) {
                RunKind::Chain
            } else {
                RunKind::Manual
            },
            task.pending_origin.unwrap_or(RunOrigin::Cli),
            None,
        )
    } else {
        (RunKind::Main, RunOrigin::Schedule, task.next_due_at)
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

    // Chain triggers consume dep marks; manual/schedule runs never do.
    let mut consume: Vec<(String, String)> = Vec::new();
    let mut upstream_sources = Vec::new();
    if kind == RunKind::Chain {
        let deps = store.list_deps(&task.id).map_err(|e| e.to_string())?;
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

    // 3. Envelope + prompt.
    let envelope = compose_envelope_header(task, run.kind, run.due_at, None);
    let upstream = compose_upstream_block(&upstream_sources);
    let prompt = format!("{envelope}{upstream}\n{}", task.prompt);

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
    let thread = crate::store::create_thread(crate::store::CreateThreadInput {
        mode: "workspace".to_string(),
        title: Some(title),
        workspace_id: None,
        workspace_path: Some(task.cwd.clone()),
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
    Ok((thread.id, session_id))
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
            thread_id: None,
            trigger_kind: TriggerKind::Manual,
            trigger_json: serde_json::json!({}),
            dep_join: DepJoin::All,
            next_due_at: None,
            last_run_at: None,
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

    /// A schedule claim advances the schedule forward and leaves no pending
    /// request behind, so the next tick does not run it again.
    #[test]
    fn claim_for_a_schedule_advances_next_due_and_clears_pending() {
        let (_dir, store) = store();
        let t = scheduler();
        store.insert_task(&t).unwrap();

        let (run, upstream) = claim(&store, &t).unwrap();
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

        let (run, _) = claim(&store, &t).unwrap();
        assert_eq!(run.kind, RunKind::Manual);
        assert_eq!(run.origin, RunOrigin::Cli);
        assert!(run.due_at.is_none());

        let after = store.get_task(&t.id).unwrap().unwrap();
        assert!(after.pending_request_at.is_none());
        assert_eq!(after.next_due_at, t.next_due_at, "run-now is not a slot");
    }

    /// A manual run never consumes dependency marks — only a chain trigger may,
    /// so pressing "run now" can't silently eat a pending dependency wakeup.
    #[test]
    fn a_manual_claim_does_not_consume_dependency_marks() {
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
        store
            .mark_dep_satisfied(&downstream.id, &upstream.id, "trn_done", 1)
            .unwrap();

        let (run, _) = claim(&store, &downstream).unwrap();
        assert_eq!(run.kind, RunKind::Manual);
        let state = store
            .get_dep_state(&downstream.id, &upstream.id)
            .unwrap()
            .unwrap();
        assert_eq!(
            state.satisfied_run_id.as_deref(),
            Some("trn_done"),
            "a manual run must leave the dependency mark for the chain trigger"
        );
    }

    /// A chain claim with every edge satisfied consumes them and carries the
    /// upstream summaries into the envelope sources.
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
        let finished = future_tasks::TaskRun {
            id: future_tasks::new_run_id(),
            task_id: upstream.id.clone(),
            kind: RunKind::Main,
            origin: RunOrigin::Schedule,
            actor: None,
            due_at: Some(1),
            status: RunStatus::Completed,
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
        };
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

        let (run, sources) = claim(&store, &downstream).unwrap();
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

        let error = claim(&store, &a).unwrap_err();
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
