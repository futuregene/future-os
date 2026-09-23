use super::*;

// ── Crash-recovery run reanimation ───────────────────────────────────────

/// Called after the agent sidecar is reachable: for every run that was
/// cancelled by startup convergence, check the agent's actual session state.
/// If the agent is still streaming, reanimate the run (back to "running") and
/// spawn a background event collector so the frontend's reattach poll picks up
/// the live preview. If it already finished, mirror its durable journal state.
pub async fn reconcile_interrupted_runs() {
    let Ok(runs) = crate::store::list_interrupted_runs() else {
        return;
    };
    if runs.is_empty() {
        return;
    }
    for run in runs {
        let session_id = run.session_id;
        let run_id = run.run_id;
        let thread_id = run.thread_id;
        match check_and_reanimate_run(&session_id, &run_id, &thread_id).await {
            Ok(()) => {}
            Err(error) => {
                eprintln!("FutureOS run reanimation failed for {run_id}: {error}");
            }
        }
    }
}

/// Reconcile one run that the synchronous startup phase cancelled as
/// interrupted, against the Agent's authoritative view of its session.
///
/// The Agent's `get_state` reports `activeRun` (a live run) and `interruptedRun`
/// (a run that began but never committed — recovered as interrupted-by-restart).
/// The GUI passes its local run id as `requested_run_id` and the Agent adopts it
/// as the canonical id, so canonical == local here. Three cases:
///
/// 1. The Agent is still running THIS exact run (`activeRun.runId == run_id`):
///    reanimate it and spawn a collector so the live preview resumes. We match on
///    the run id, not just "the session is streaming", so a stale local run is
///    never reattached against a different run's stream.
/// 2. The Agent confirms THIS run as interrupted (`interruptedRun.runId ==
///    run_id`): it began but never committed. The sync phase already cancelled it
///    with `error_type='interrupted'`; keep that accurate terminal state rather
///    than falsely settling it as completed.
/// 3. Neither: use `requestedRun` to mirror the exact durable terminal state.
///    If the Agent has no marker for this id, conservatively leave interrupted.
pub(super) async fn check_and_reanimate_run(
    session_id: &str,
    run_id: &str,
    thread_id: &str,
) -> Result<(), String> {
    let mut client = connect_agent().await.map_err(|e| format!("connect: {e}"))?;
    let state = client
        .execute_command(get_run_state_command(
            session_id.to_string(),
            run_id.to_string(),
        ))
        .await
        .map_err(|e| format!("get_state: {e}"))?
        .into_inner();
    if !state.success {
        // The Agent could not resolve this session (its JSONL is gone, or the
        // Agent cannot hydrate it). Treat the run as orphaned: leave it in the
        // interrupted state the synchronous phase set rather than asserting it
        // completed, so it stays visible as interrupted instead of vanishing
        // into a false "completed".
        eprintln!(
            "FutureOS startup reconcile: get_state failed for run {run_id} ({}); leaving interrupted",
            state.error
        );
        return Ok(());
    }
    let state_value = future_rpc::decode::response_data(&state);
    let is_streaming = state_value
        .get("isStreaming")
        .and_then(|s| s.as_bool())
        .unwrap_or(false);
    let active_run_id = state_value
        .get("activeRun")
        .and_then(|run| run.get("runId"))
        .and_then(|id| id.as_str())
        .map(str::to_string);
    let interrupted_run_id = state_value
        .get("interruptedRun")
        .and_then(|run| run.get("runId"))
        .and_then(|id| id.as_str())
        .map(str::to_string);
    let requested_terminal = state_value
        .get("requestedRun")
        .filter(|value| value.is_object());

    if is_streaming && active_run_id.as_deref() == Some(run_id) {
        // canonical == local: the Agent adopted this run's requested_run_id.
        // CAS: only reanimate while still in the interrupted state. If the user
        // already aborted (or another path settled it) between listing and now,
        // the guard matches zero rows and we must NOT reattach against a run
        // whose terminal state would race the projection.
        let reanimated =
            crate::store::reanimate_run(run_id).map_err(|e| format!("reanimate: {e}"))?;
        if !reanimated {
            eprintln!(
                "FutureOS startup reconcile: run {run_id} no longer interrupted; skipping reattach"
            );
            return Ok(());
        }
        // The session observer takes over from the local cursor: it projects
        // the remaining events (this run has no pipeline owner), mirrors them
        // to NATS, and settles the row at agent_end.
        observer::ensure_observer_for_thread(session_id, thread_id)?;
    } else if interrupted_run_id.as_deref() == Some(run_id) {
        // The Agent confirms this run began but never committed (interrupted by
        // the restart). The synchronous phase already cancelled it with
        // error_type='interrupted'; leave that accurate state in place.
        eprintln!("FutureOS startup reconcile: run {run_id} confirmed interrupted by restart; leaving cancelled");
    } else if let Some(terminal) = requested_terminal {
        let state = terminal
            .get("status")
            .and_then(|value| value.as_str())
            .ok_or_else(|| "requestedRun omitted terminal state".to_string())?;
        let error = terminal.get("error").and_then(|value| value.as_str());
        crate::store::settle_interrupted_run_from_agent(run_id, state, error)
            .map_err(|e| format!("settle: {e}"))?;
    } else {
        eprintln!(
            "FutureOS startup reconcile: no durable terminal for run {run_id}; leaving interrupted"
        );
    }
    Ok(())
}

/// The Agent returned RunGone (`failed_precondition` / `not_found`) for
/// `canonical_run_id` on attach: it no longer recognizes the run. Settle the
/// local `local_run_id` row from the Agent's authoritative journal instead of
/// leaving it stranded as `running` or guessing `failed`.
///
/// The Agent is reachable (RunGone is a response, not a connect failure), so
/// `get_state` answers with `queuedRuns` / `activeRun` / `interruptedRun` /
/// `requestedRun`:
/// - queued (`queuedRuns` contains the id): the Agent accepted the prompt but
///   it still sits behind an older run — alive, not yet attachable (no
///   execution epoch). The session observer projects it from the journal once
///   it starts; the row stays non-terminal;
/// - still active (attach raced a `start_run`): leave running, let the live path
///   converge — do not mark failed;
/// - a durable terminal marker (`requestedRun`): mirror its exact state;
/// - confirmed interrupted-by-restart: settle cancelled/interrupted;
/// - no marker at all: the run is truly gone, settle failed so the UI frees up.
///
/// Every write goes through the compare-and-set writers, so a concurrent user
/// abort (cancelled) always wins.
pub(super) async fn reconcile_run_gone(
    local_run_id: &str,
    canonical_run_id: &str,
    session_id: &str,
    thread_id: &str,
    reason: &str,
) -> Result<(), String> {
    let mut client = connect_agent()
        .await
        .map_err(|e| format!("reconcile connect: {e}"))?;
    let state = client
        .execute_command(get_run_state_command(
            session_id.to_string(),
            canonical_run_id.to_string(),
        ))
        .await
        .map_err(|e| format!("reconcile get_state: {e}"))?
        .into_inner();
    let state_value = if state.success {
        future_rpc::decode::response_data(&state)
    } else {
        serde_json::Value::Null
    };

    // A RunGone on attach is not proof the run is gone: the Agent may have
    // accepted the prompt but QUEUED it behind an older run (a queued run
    // has no execution epoch to attach to yet). Confirm queued BEFORE any
    // terminal/orphan settling — the run is alive and the session observer
    // will project it from the journal once it starts.
    if queued_run_ids(&state_value).any(|run_id| run_id == canonical_run_id) {
        eprintln!(
            "FutureOS run {local_run_id} is queued on the Agent; observer projects it on start ({reason})"
        );
        observer::ensure_observer_for_thread(session_id, thread_id)?;
        return Ok(());
    }

    let active = state_value
        .get("activeRun")
        .and_then(|run| run.get("runId"))
        .and_then(|id| id.as_str());
    if active == Some(canonical_run_id) {
        eprintln!(
            "FutureOS run {local_run_id} still active on Agent after RunGone ({reason}); leaving running"
        );
        return Ok(());
    }

    if let Some(terminal) = state_value
        .get("requestedRun")
        .filter(|value| value.is_object())
    {
        let agent_state = terminal
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let error = terminal.get("error").and_then(|value| value.as_str());
        settle_from_agent_terminal(local_run_id, agent_state, error)
            .map_err(|error| format!("reconcile terminal status: {error}"))?;
        return Ok(());
    }

    let interrupted = state_value
        .get("interruptedRun")
        .and_then(|run| run.get("runId"))
        .and_then(|id| id.as_str());
    if interrupted == Some(canonical_run_id) {
        crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
            run_id: local_run_id.to_string(),
            status: "cancelled".to_string(),
            error_message: Some("Interrupted because Future Agent restarted.".to_string()),
            error_type: Some("interrupted".to_string()),
        })
        .map_err(|error| format!("reconcile interrupted status: {error}"))?;
        return Ok(());
    }

    // No marker at all — the run is genuinely gone. Settle failed (CAS) so the
    // composer can't strand on a permanent "running".
    crate::store::fail_run_if_active(
        local_run_id,
        &format!("Future Agent run no longer active: {reason}"),
        "stream_interrupted",
    )
    .map_err(|error| format!("reconcile missing run: {error}"))?;
    Ok(())
}

/// CAS-mirror the Agent journal's durable terminal marker onto a local run row.
/// Shared by attach-time RunGone reconciliation and the runtime watchdog.
/// Unknown states fall back to a generic failure — a row is never asserted
/// `completed` without positive evidence.
pub(super) fn settle_from_agent_terminal(
    local_run_id: &str,
    agent_state: &str,
    error: Option<&str>,
) -> Result<(), crate::AppError> {
    let (status, error_type, default_message) = match agent_state {
        "completed" => ("completed", None, None),
        "cancelled" => ("cancelled", Some("cancelled"), Some("Run was cancelled.")),
        "error" | "failed" => (
            "failed",
            Some("agent_error"),
            Some("Future Agent run failed."),
        ),
        _ => (
            "failed",
            Some("stream_interrupted"),
            Some("Future Agent response ended before a clean terminal."),
        ),
    };
    crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
        run_id: local_run_id.to_string(),
        status: status.to_string(),
        error_message: error
            .map(str::to_string)
            .or_else(|| default_message.map(str::to_string)),
        error_type: error_type.map(str::to_string),
    })
    .map(|_| ())
}

// ── Live run watchdog ─────────────────────────────────────────────────────

/// Seconds between watchdog passes.
pub(super) const WATCHDOG_INTERVAL_SECS: u64 = 30;
/// A run younger than this is never inspected: the row is created before the
/// Agent acknowledges the prompt (and before the replica lease is acquired),
/// so a young row can legitimately have no Agent marker and no collector yet.
/// Must comfortably exceed the slowest prompt setup (session ensure, model
/// setup, the `capture_before` git fork), or the watchdog could fail a run
/// whose prompt is still being prepared.
pub(super) const WATCHDOG_GRACE_SECS: u64 = 45;
/// A row the Agent has no marker for at all is only settled as orphaned once
/// it is this old. Below the threshold it is skipped — same startup-window
/// hazard as the grace period, just for rows that survived several passes.
pub(super) const WATCHDOG_ORPHAN_SECS: u64 = 600;

/// What the watchdog should do with a non-terminal run, given the Agent's
/// authoritative view of it. Pure (no IO) — [`reconcile_active_run_once`]
/// applies it.
#[derive(Debug, PartialEq)]
pub(super) enum ActiveRunAction {
    /// The Agent is still executing this exact run. Reattach a collector if no
    /// live collector owns the replica lease (a failed acquire means healthy).
    Attach,
    /// The Agent's journal has a durable terminal marker for this run: mirror
    /// its exact state onto the row.
    SettleTerminal {
        agent_state: String,
        error: Option<String>,
    },
    /// The Agent confirms this run began but never committed (restart).
    SettleInterrupted,
    /// The Agent has no marker at all and the row is past the orphan age.
    SettleOrphaned,
    /// Healthy, or not enough evidence yet — leave the row untouched.
    Skip,
}

/// Decide the watchdog action for one non-terminal run from the Agent's
/// `get_run_state` payload. Mirrors the marker precedence of
/// [`reconcile_run_gone`]: live active run first, then the durable terminal
/// marker, then the interrupted-by-restart marker, then age-based orphaning.
pub(super) fn plan_active_run_reconciliation(
    state: &serde_json::Value,
    canonical_run_id: &str,
    age_secs: u64,
) -> ActiveRunAction {
    let is_streaming = state
        .get("isStreaming")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let active_run_id = state
        .get("activeRun")
        .and_then(|run| run.get("runId"))
        .and_then(|id| id.as_str());
    if is_streaming && active_run_id == Some(canonical_run_id) {
        return ActiveRunAction::Attach;
    }
    if queued_run_ids(state).any(|run_id| run_id == canonical_run_id) {
        // Queued behind an older run: no execution epoch to attach to yet, but
        // the Agent owns the run — never orphan-settle it.
        return ActiveRunAction::Skip;
    }
    if let Some(terminal) = state.get("requestedRun").filter(|value| value.is_object()) {
        return ActiveRunAction::SettleTerminal {
            agent_state: terminal
                .get("status")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string(),
            error: terminal
                .get("error")
                .and_then(|value| value.as_str())
                .map(str::to_string),
        };
    }
    let interrupted_run_id = state
        .get("interruptedRun")
        .and_then(|run| run.get("runId"))
        .and_then(|id| id.as_str());
    if interrupted_run_id == Some(canonical_run_id) {
        return ActiveRunAction::SettleInterrupted;
    }
    if age_secs >= WATCHDOG_ORPHAN_SECS {
        return ActiveRunAction::SettleOrphaned;
    }
    ActiveRunAction::Skip
}

/// Ids of runs the Agent has accepted but not yet started (the session's
/// `queuedRuns` list). A run on this list is alive: it has no execution epoch
/// to attach to yet, but it must never be settled as gone or orphaned.
fn queued_run_ids(state: &serde_json::Value) -> impl Iterator<Item = &str> {
    state
        .get("queuedRuns")
        .and_then(|runs| runs.as_array())
        .into_iter()
        .flatten()
        .filter_map(|run| run.get("runId").and_then(|id| id.as_str()))
}

/// Reconcile one non-terminal run row against the Agent's authoritative state.
pub(super) async fn reconcile_active_run_once(
    run: &crate::store::ActiveRun,
    canonical_run_id: &str,
    age_secs: u64,
) -> Result<(), String> {
    let mut client = connect_agent().await.map_err(|e| format!("connect: {e}"))?;
    // Query by the canonical id (not the local row id): remote-attached runs
    // carry a synthetic local id the Agent never saw, while its durable
    // journal marker is keyed by the canonical id it adopted. For GUI runs
    // the two are identical.
    let state = client
        .execute_command(get_run_state_command(
            run.session_id.clone(),
            canonical_run_id.to_string(),
        ))
        .await
        .map_err(|e| format!("get_run_state: {e}"))?
        .into_inner();
    if !state.success {
        // The Agent cannot resolve this session — leave the row as is; startup
        // convergence settles genuinely dead rows on the next launch.
        return Ok(());
    }
    let state_value = future_rpc::decode::response_data(&state);
    match plan_active_run_reconciliation(&state_value, canonical_run_id, age_secs) {
        ActiveRunAction::Skip => Ok(()),
        ActiveRunAction::Attach => {
            // The Agent is still streaming this run but no pipeline collector
            // owns it (a crashed collector, a run started by another client,
            // or one reanimated out from under a dead lease). The session
            // observer projects it from its local cursor — idempotent, and
            // never races a pipeline collector (single-writer rule).
            observer::ensure_observer_for_thread(&run.session_id, &run.thread_id)?;
            Ok(())
        }
        ActiveRunAction::SettleTerminal { agent_state, error } => {
            settle_from_agent_terminal(&run.run_id, &agent_state, error.as_deref())
                .map_err(|e| format!("settle terminal: {e}"))
        }
        ActiveRunAction::SettleInterrupted => {
            crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
                run_id: run.run_id.clone(),
                status: "cancelled".to_string(),
                error_message: Some("Interrupted because Future Agent restarted.".to_string()),
                error_type: Some("interrupted".to_string()),
            })
            .map(|_| ())
            .map_err(|e| format!("settle interrupted: {e}"))
        }
        ActiveRunAction::SettleOrphaned => crate::store::fail_run_if_active(
            &run.run_id,
            "Future Agent run is no longer active on the agent.",
            "stream_interrupted",
        )
        .map(|_| ())
        .map_err(|e| format!("settle orphaned: {e}")),
    }
}

/// Launch the runtime watchdog for active runs: a periodic pass reconciling
/// every non-terminal run row against the Agent's authoritative state. The
/// backstop for rows whose owning pipeline/collector never settled them:
///
/// - The webview was suspended (hidden/occluded window) and never applied the
///   `agent_prompt` invoke response, so the frontend's status write never ran
///   — the backend settles these rows itself now, but this pass repairs rows
///   created by older builds and any future settlement gap.
/// - A collector task died or wedged while the run was still streaming
///   Agent-side: the replica lease probe finds no owner and reattaches one,
///   resuming live projection from the last persisted cursor.
/// - The Agent lost the run (restart/rollover) without the GUI noticing: the
///   durable journal marker (or its absence) settles the row.
///
/// All writes go through the compare-and-set writers, so a healthy collector,
/// a concurrent user abort, and the normal pipeline are never clobbered. The
/// pass self-gates on Agent reachability; Agent downtime is left for startup
/// convergence on the next launch.
pub fn spawn_active_run_watchdog() {
    crate::runtime::spawn(async move {
        loop {
            tokio::time::sleep(watchdog_interval()).await;
            #[cfg(test)]
            if TEST_WATCHDOG_STOP.swap(false, std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            if connect_agent().await.is_err() {
                continue;
            }
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            active_run_watchdog_pass(now_ms).await;
        }
    });
}

/// One watchdog pass: reconcile every non-terminal run row against the
/// Agent's authoritative state, then the pending approvals. Extracted from
/// the spawn loop so tests can drive it with a synthetic clock (run rows
/// cannot be backdated through the store API).
pub(super) async fn active_run_watchdog_pass(now_ms: i64) {
    let Ok(active_runs) = crate::store::list_active_runs() else {
        return;
    };
    for run in active_runs {
        let age_secs = now_ms.saturating_sub(run.created_at).max(0) as u64 / 1000;
        if age_secs < WATCHDOG_GRACE_SECS {
            continue;
        }
        let canonical = AGENT_REPLICAS
            .canonical_for_local(&run.run_id)
            .unwrap_or_else(|| run.run_id.clone());
        if let Err(error) = reconcile_active_run_once(&run, &canonical, age_secs).await {
            eprintln!(
                "FutureOS run watchdog could not reconcile {}: {error}",
                run.run_id
            );
        }
    }
    // Approvals outlive their collectors (the Agent stays parked while
    // the GUI restarts), so reconcile them against the Agent's pending
    // set on every tick — not just at startup.
    approval::reconcile_pending_approvals().await;
}

#[cfg(test)]
pub(super) static TEST_WATCHDOG_STOP: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Watchdog tick interval; tests shrink it via env (a cfg(test)-only seam).
pub(super) fn watchdog_interval() -> std::time::Duration {
    #[cfg(test)]
    if let Some(ms) = std::env::var("FUTURE_TEST_WATCHDOG_INTERVAL_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
    {
        return std::time::Duration::from_millis(ms);
    }
    std::time::Duration::from_secs(WATCHDOG_INTERVAL_SECS)
}

// ── Remote-stream attach (cross-client streaming) ─────────────────────────

/// Called when the GUI opens a thread whose agent session is being driven by
/// another client (TUI, CLI, phone). Ensures the session observer is live (it
/// projects the run's events, mirrors them to NATS, and settles the run),
/// then returns the local run row for the agent's active run so the existing
/// reattach machinery picks up live previews immediately.
pub async fn attach_remote_stream(thread_id: &str) -> Result<String, String> {
    let thread = crate::store::get_thread(thread_id)
        .map_err(|e| format!("get_thread: {e}"))?
        .ok_or_else(|| "Thread not found".to_string())?;
    let session_id = thread
        .agent_session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Thread has no agent session".to_string())?;

    // Don't create a duplicate run if one is already active or waiting on an
    // approval decision — the observer is already projecting it.
    fn is_active(status: &str) -> bool {
        status == "running" || status == "waiting_approval"
    }
    let existing_runs = crate::store::list_runs(thread_id).unwrap_or_default();
    if let Some(active) = existing_runs.iter().find(|r| is_active(&r.status)) {
        observer::ensure_observer_for_thread(session_id, thread_id)?;
        return Ok(active.id.clone());
    }
    let mut client = connect_agent().await.map_err(|e| format!("connect: {e}"))?;
    let state = client
        .execute_command(get_state_command(session_id.to_string()))
        .await
        .map_err(|e| format!("get_state: {e}"))?
        .into_inner();
    let state_value = future_rpc::decode::response_data(&state);
    let canonical_run_id = state_value
        .get("activeRun")
        .and_then(|run| run.get("runId"))
        .and_then(|id| id.as_str())
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "Agent session has no active canonical run".to_string())?
        .to_string();

    observer::ensure_observer_for_thread(session_id, thread_id)?;
    // Get-or-create the local row NOW (id == canonical run id) so the frontend
    // gets a concrete run id back instead of waiting for the observer's first
    // event. The observer reuses this row via the same binding path.
    observer::ensure_run_binding(session_id, &canonical_run_id, thread_id)
        .ok_or_else(|| "Unable to create local run for the agent's active run".to_string())
}

/// When the agent session's cwd changes (via TUI /cwd or another client),
/// move the thread to the workspace that matches the new cwd.
pub fn reconcile_thread_workspace(session_id: &str, new_cwd: &str) -> Result<(), String> {
    let thread = crate::store::find_thread_by_agent_session(session_id)
        .map_err(|e| format!("find_thread: {e}"))?
        .ok_or_else(|| "No thread found for this session".to_string())?;

    let cwd = new_cwd.trim().trim_end_matches(['/', '\\']);
    if cwd.is_empty() {
        return Ok(());
    }

    // Determine workspace type.
    let is_chat = {
        let cwd_normalized = cwd.replace('\\', "/");
        // Normalize the home separators too: `cwd_normalized` is forward-slash
        // but a Windows home is not, and comparing the two spellings would
        // route a chat cwd into the project-workspace branch (where the
        // directory has to already exist). See `is_desktop_chat_cwd`, which
        // normalizes the same way.
        let home = crate::home_dir().unwrap_or_default().replace('\\', "/");
        let chat_dir = format!("{}/.future/workspaces/chat/", home.trim_end_matches('/'));
        cwd_normalized.starts_with(&chat_dir) || cwd_normalized == chat_dir.trim_end_matches('/')
    };

    if is_chat {
        crate::store::update_chat_workspace_path(&thread.id, cwd)
            .map_err(|e| format!("update_workspace: {e}"))?;
        return Ok(());
    }

    // Project workspace: find or create by cwd path.
    let workspace_name = std::path::Path::new(cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(cwd)
        .to_string();

    // Identity is the canonical directory, not the spelling the agent
    // reported: `/tmp/x` and `/private/tmp/x` are one workspace.
    let existing =
        crate::store::find_user_workspace_by_path(std::path::Path::new(cwd)).unwrap_or(None);

    let workspace_id = if let Some(ws) = existing {
        ws.id
    } else {
        let ws = crate::store::create_workspace(crate::store::CreateWorkspaceInput {
            name: Some(workspace_name),
            path: cwd.to_string(),
            description: None,
            create_directory: Some(false),
        })
        .map_err(|e| format!("create_workspace: {e}"))?;
        ws.id
    };

    // Update the thread's workspace assignment.
    crate::store::move_thread_to_workspace(&thread.id, &workspace_id)
        .map_err(|e| format!("move_thread: {e}"))?;

    Ok(())
}
