//! Continue, on launch, the conversations a run was interrupted in.
//!
//! The Agent may be embedded in this process (the default), so quitting the
//! Desktop ends the run it was streaming. Nothing on either side records that as
//! work waiting for the user: the Agent's journal keeps the run's start and no
//! terminal, `get_run_state` reports it as `interruptedRun`, and the local row
//! stays non-terminal. A user who closes the app mid-task reopens it to a
//! conversation that reads "interrupted" and has to type 继续 by hand — once per
//! conversation.
//!
//! [`resume_interrupted_runs_at_launch`] closes that gap once per launch. For
//! every conversation whose **newest** run the Agent itself confirms as
//! interrupted — a run that began and never committed, which is exactly what the
//! embedded Agent's death leaves behind — it settles that row and sends one
//! continuation prompt through `super::headless`, the same pipeline a prompt
//! from a paired phone uses. The Agent then continues with the whole
//! conversation as its context, which is what the manual 继续 flow does too
//! (`continue_run` on the remote side, `buildContinuePrompt` in the GUI).
//!
//! What it deliberately does **not** do:
//!
//! * A run the Agent still owns (live or queued) is left to the watchdog, which
//!   reattaches a collector instead of starting a second run. That is the
//!   external-Agent case: the Agent outlived the Desktop and its run is still
//!   going.
//! * A run the Agent has a terminal marker for is left to the watchdog as well:
//!   it did finish (or fail, or was cancelled) while the Desktop was away.
//! * A conversation parked on an approval is skipped. Resuming would re-issue a
//!   tool call the user never decided on, under whatever approval tier is
//!   current; the approval reconciliation owns that row.
//! * Runs older than [`RESUME_MAX_AGE`] are skipped. The app can be closed for
//!   weeks, and continuing a task nobody has looked at in a month is not the
//!   "pick up where I stopped" this exists for.
//!
//! One pass per launch, so a resumed run that is itself interrupted by the next
//! quit is resumed again — that is the intended shape, and it cannot loop inside
//! one launch because a resume creates a *new* run.

use std::time::Duration;

use super::*;

/// How long after its start a run is still worth continuing automatically.
///
/// A week covers a weekend and a holiday without resurrecting a task the user
/// abandoned long ago; the number is a product judgement, not a technical bound.
pub const RESUME_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// How long a resume waits for the Agent to acknowledge its prompt before the
/// pass moves on. The pipeline settles its own row either way, so a slow
/// acknowledgement costs a log line, not a stuck run.
const ACK_TIMEOUT: Duration = Duration::from_secs(30);

/// What the Agent's own ledger says about one interrupted candidate.
#[derive(Debug, PartialEq)]
pub enum ResumeDecision {
    /// The run began and never committed: the previous process died with it.
    /// Continue this conversation.
    Resume,
    /// The Agent still owns the run (executing it, or holding it queued). The
    /// watchdog reattaches; starting a second run would race it.
    Live,
    /// The Agent's journal has a terminal marker for this run: it ended on its
    /// own while the Desktop was away.
    Terminal,
    /// No evidence either way (an unreadable session, an older Agent). Leaving
    /// the row alone is the only honest answer.
    Unknown,
}

/// Which candidates to consider, decided from the Agent's `get_state` payload.
///
/// Marker precedence mirrors the watchdog's (`reconciliation.rs`): the live and
/// queued markers are in-memory truth, the durable terminal marker outranks the
/// interrupted one, and "interrupted" is only claimed when neither of the others
/// is present.
pub fn plan_resume(state: &serde_json::Value, run_id: &str) -> ResumeDecision {
    let active_run_id = state
        .get("activeRun")
        .and_then(|run| run.get("runId"))
        .and_then(|id| id.as_str());
    if active_run_id == Some(run_id) {
        return ResumeDecision::Live;
    }
    let queued = state
        .get("queuedRuns")
        .and_then(|runs| runs.as_array())
        .is_some_and(|runs| {
            runs.iter()
                .any(|run| run.get("runId").and_then(|id| id.as_str()) == Some(run_id))
        });
    if queued {
        return ResumeDecision::Live;
    }
    if state
        .get("requestedRun")
        .is_some_and(|terminal| terminal.is_object())
    {
        return ResumeDecision::Terminal;
    }
    let interrupted_run_id = state
        .get("interruptedRun")
        .and_then(|run| run.get("runId"))
        .and_then(|id| id.as_str());
    if interrupted_run_id == Some(run_id) {
        return ResumeDecision::Resume;
    }
    ResumeDecision::Unknown
}

/// The prompt that continues an interrupted run.
///
/// Sent to the model, so it is not localized — the same rule as the remote
/// `continue_run` prompt, whose "已执行内容摘要" shape this keeps. The extra
/// sentence says *why* the run stopped: without it the model reads an assistant
/// turn that ends mid-tool-call and cannot tell whether the tool ran.
pub(crate) fn build_resume_prompt(run_id: &str) -> String {
    let mut lines = vec![
        "继续上一个任务。".to_string(),
        String::new(),
        "上一轮运行在关闭 FutureOS 时被中断了（工具可能已经执行了一部分）。请先判断已经完成了什么，再继续未完成的部分，不要重复已经生效的修改或命令。".to_string(),
    ];
    let events = crate::store::list_run_events(run_id).unwrap_or_default();
    let terminal: Vec<_> = events
        .iter()
        .filter(|event| {
            matches!(
                event.event_type.as_str(),
                "error" | "agent_error" | "agent_end" | "tool_end" | "tool_result"
            )
        })
        .collect();
    if !terminal.is_empty() {
        lines.push(String::new());
        lines.push("已执行内容摘要:".to_string());
        for event in terminal.iter().rev().take(6).rev() {
            let payload = event.payload.as_deref().unwrap_or("");
            let truncated: String = payload.chars().take(360).collect();
            lines.push(format!("- {}: {}", event.event_type, truncated));
        }
    }
    lines.join("\n")
}

/// A conversation the pass may continue.
#[derive(Debug)]
struct Candidate {
    run_id: String,
    thread_id: String,
    session_id: String,
    /// Unix millis, from the run row.
    created_at: i64,
    /// The row is parked on an approval decision (`waiting_approval`).
    waiting_approval: bool,
}

/// The thread's newest run, when it is one this pass may consider.
///
/// It must be the **newest** run: an older interrupted row was followed by a run
/// the user (or another client) started afterwards, so the conversation has
/// moved on and continuing would answer a stale question with a second run.
fn candidate_of(
    thread_id: &str,
    session_id: &str,
    runs: &[crate::store::RunRecord],
) -> Option<Candidate> {
    let newest = runs.first()?;
    // The only non-terminal statuses; anything else is a settled row.
    if !matches!(newest.status.as_str(), "running" | "waiting_approval") {
        return None;
    }
    Some(Candidate {
        run_id: newest.id.clone(),
        thread_id: thread_id.to_string(),
        session_id: session_id.to_string(),
        created_at: newest.created_at,
        waiting_approval: newest.status == "waiting_approval",
    })
}

/// Continue every conversation whose run the previous process interrupted.
///
/// Called once per launch, after the Agent is reachable. Never fails the launch:
/// each candidate's trouble is reported and the pass moves on, because resuming
/// is recovery, not startup.
pub async fn resume_interrupted_runs_at_launch(now_ms: i64) {
    match crate::store::get_app_settings() {
        // An unreadable settings store is not consent to spend the user's
        // credits on prompts they did not send.
        Ok(settings) if settings.auto_resume_interrupted => {}
        Ok(_) => return,
        Err(error) => {
            eprintln!("FutureOS resume: app settings unavailable: {error}");
            return;
        }
    }
    // Sessions this pass already submitted a prompt for. The candidate row is
    // itself non-terminal by definition, so "the store says the session is
    // busy" cannot be the test — a session busy for a *reason* is caught by the
    // Agent's own `activeRun`/`queuedRuns` in `resume_one`.
    let mut resumed_sessions: Vec<String> = Vec::new();
    let threads = match crate::store::list_threads() {
        Ok(threads) => threads,
        Err(error) => {
            eprintln!("FutureOS resume: thread list unavailable: {error}");
            return;
        }
    };
    let mut resumed = 0usize;
    for thread in threads {
        // Archived and deleted conversations keep whatever they were doing; a
        // thread with no Agent session was never prompted.
        if thread.status != "active" {
            continue;
        }
        let Some(session_id) = thread
            .agent_session_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        else {
            continue;
        };
        let runs = match crate::store::list_runs(&thread.id) {
            Ok(runs) => runs,
            Err(error) => {
                eprintln!(
                    "FutureOS resume: run list unavailable for {}: {error}",
                    thread.id
                );
                continue;
            }
        };
        let Some(candidate) = candidate_of(&thread.id, session_id, &runs) else {
            continue;
        };
        if candidate.waiting_approval
            || now_ms.saturating_sub(candidate.created_at).max(0) as u128
                > RESUME_MAX_AGE.as_millis()
            || resumed_sessions.iter().any(|resumed| resumed == session_id)
        {
            continue;
        }
        match resume_one(&candidate).await {
            Ok(true) => {
                resumed += 1;
                // Two prompts for one session would be refused by the Agent;
                // recording it here keeps the pass from asking twice.
                resumed_sessions.push(session_id.to_string());
            }
            Ok(false) => {}
            Err(error) => {
                eprintln!(
                    "FutureOS resume: could not continue run {} in thread {}: {error}",
                    candidate.run_id, candidate.thread_id
                );
            }
        }
    }
    if resumed > 0 {
        eprintln!("FutureOS resume: continued {resumed} interrupted conversation(s)");
    }
}

/// The trigger id a continuation is created with. `runs.trigger_message_id` is
/// what a prompt's own identity is recorded in, so a resumed run can be found
/// again by the run it continued.
fn resume_marker(run_id: &str) -> String {
    format!("resume:{run_id}")
}

/// Mark the interrupted row as what it was, without touching a row another
/// writer has already settled, and tell the rail so it stops showing that
/// conversation as running.
fn settle_interrupted_row(candidate: &Candidate) -> Result<(), String> {
    crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
        run_id: candidate.run_id.clone(),
        status: "cancelled".to_string(),
        error_message: Some(
            "Interrupted when FutureOS closed; continuing in a new run.".to_string(),
        ),
        error_type: Some("interrupted".to_string()),
    })
    .map(|_| ())
    .map_err(|error| format!("settle: {error}"))?;
    crate::emit_thread_runtime_updated(
        candidate.thread_id.clone(),
        candidate.run_id.clone(),
        "cancelled".to_string(),
        false,
    );
    Ok(())
}

/// Ask the Agent whether this run was interrupted, and if so settle its row and
/// send the continuation prompt. Returns whether a prompt was submitted.
async fn resume_one(candidate: &Candidate) -> Result<bool, String> {
    let mut client = connect_agent()
        .await
        .map_err(|error| format!("connect: {error}"))?;
    verify_session_database(&mut client, &candidate.session_id)
        .await
        .map_err(|error| format!("database source: {error}"))?;
    let response = client
        .execute_command(get_run_state_command(
            candidate.session_id.clone(),
            candidate.run_id.clone(),
        ))
        .await
        .map_err(|error| format!("get_state: {error}"))?
        .into_inner();
    if !response.success {
        // The Agent cannot resolve this session (deleted, or not its own): the
        // row is left to the watchdog's orphan path.
        return Ok(false);
    }
    let state = future_rpc::decode::response_data(&response);
    if plan_resume(&state, &candidate.run_id) != ResumeDecision::Resume {
        return Ok(false);
    }
    let thread = crate::store::get_thread(&candidate.thread_id)
        .map_err(|error| format!("get_thread: {error}"))?
        .ok_or_else(|| "conversation no longer exists".to_string())?;
    // Checked after the Agent's answer and before the settle: an approval that
    // arrived while the previous run was streaming still belongs to the user.
    let pending_approval = crate::store::list_pending_approval_requests()
        .map_err(|error| format!("approvals: {error}"))?
        .iter()
        .any(|approval| approval.thread_id == candidate.thread_id);
    if pending_approval {
        return Ok(false);
    }
    // The continuation's own durable identity. A prompt triggered by this id
    // already exists when an earlier pass resumed this run (a launch that was
    // itself interrupted between the settle and the prompt), and resuming twice
    // would ask the same question in a second run.
    let marker = resume_marker(&candidate.run_id);
    let already_resumed = crate::store::find_run_by_trigger_message_id(&marker)
        .map_err(|error| format!("receipt: {error}"))?
        .is_some();
    if already_resumed {
        settle_interrupted_row(candidate)?;
        return Ok(false);
    }
    // Settle the interrupted row first: the resumed run is a different row, and
    // a row left non-terminal keeps the conversation reading as "running" while
    // the new run is what is actually running. Compare-and-set, so a concurrent
    // settle (the user's stop, the watchdog) wins over this guess.
    settle_interrupted_row(candidate)?;

    let prepared = prepare_prompt_persisted_with_trigger(
        &thread,
        build_resume_prompt(&candidate.run_id),
        None,
        None,
        Vec::new(),
        Some(marker),
    )
    .map_err(|error| format!("prepare: {error}"))?;
    let run_id = prepared.run_id.clone();
    let thread_id = prepared.thread_id.clone();
    let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
    let pipeline_thread = thread_id.clone();
    crate::runtime::spawn(async move {
        if let Err(error) = run_prepared_prompt_with_acceptance(prepared, accepted_tx).await {
            eprintln!("FutureOS resume: run {run_id} failed: {error}");
        }
        crate::emit_remote_activity(&pipeline_thread);
    });
    // The rail reads the new run from the event stream; the activity push is
    // what makes a view already showing this conversation re-read it, exactly as
    // for a prompt sent from another client.
    crate::emit_threads_updated();
    crate::emit_remote_activity(&thread_id);
    match tokio::time::timeout(ACK_TIMEOUT, accepted_rx).await {
        Ok(Ok(Ok(()))) => Ok(true),
        Ok(Ok(Err(error))) => Err(format!("agent refused the continuation: {error}")),
        Ok(Err(_)) => Err("agent never acknowledged the continuation".to_string()),
        Err(_) => Err("agent acknowledgement timed out".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{
        get_state_payload, mock_agent, seed_run, seed_thread, seed_workspace, Reply, StreamScript,
        TestHome,
    };
    use super::*;
    use crate::test_support::agent::MockAgentGuard;

    /// The prompt pipeline needs the session's own state and a stream of its
    /// own. Both are queued per session / FIFO, so two conversations in one test
    /// cannot consume each other's replies.
    fn arm_prompt_pipeline(mock: &MockAgentGuard, session_id: &str, cwd: &str) {
        mock.push_state_for_session(
            session_id,
            Reply::Data(serde_json::json!({ "sessionId": session_id, "cwd": cwd }).to_string()),
        );
        mock.push_stream(StreamScript::Events(vec![], None));
    }

    /// A run the Agent calls interrupted is continued: the old row is settled
    /// with the interrupted marker, a new run exists, and the prompt sent to the
    /// Agent is the resume prompt naming the interruption.
    #[tokio::test]
    async fn an_interrupted_run_is_continued_with_one_new_run() {
        let home = TestHome::new("resume-interrupted");
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-resume"));
        let run = seed_run(&thread.id);
        let mock = mock_agent();
        mock.push_run_state(
            &run.id,
            serde_json::json!({"interruptedRun": {"runId": run.id}}),
        );
        arm_prompt_pipeline(&mock, "sess-resume", &workspace.path);

        resume_interrupted_runs_at_launch(crate::store::now_millis()).await;

        let settled = crate::store::get_run(&run.id).expect("get").expect("some");
        assert_eq!(settled.status, "cancelled");
        assert_eq!(settled.error_type.as_deref(), Some("interrupted"));
        let prompts = mock.requests_of("prompt");
        assert_eq!(prompts.len(), 1, "exactly one prompt");
        assert!(prompts[0].message.starts_with("继续上一个任务。"));
        assert!(prompts[0].message.contains("关闭 FutureOS 时被中断"));
        let runs = crate::store::list_runs(&thread.id).expect("runs");
        let resumed = runs
            .iter()
            .find(|row| row.id != run.id)
            .expect("the continuation's row");
        assert_eq!(prompts[0].session_id, "sess-resume");
        assert_eq!(
            resumed.trigger_message_id.as_deref(),
            Some(format!("resume:{}", run.id).as_str()),
            "the continuation records which run it continues"
        );
    }

    /// A run the Agent still owns is not continued: the watchdog reattaches, and
    /// a second prompt would race the live one.
    #[tokio::test]
    async fn a_live_run_is_left_to_the_watchdog() {
        let home = TestHome::new("resume-live");
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-live"));
        let run = seed_run(&thread.id);
        let mock = mock_agent();
        mock.push_run_state(&run.id, serde_json::json!({"activeRun": {"runId": run.id}}));

        resume_interrupted_runs_at_launch(crate::store::now_millis()).await;

        assert_eq!(crate::store::list_runs(&thread.id).expect("runs").len(), 1);
        assert_eq!(
            crate::store::get_run(&run.id)
                .expect("get")
                .expect("some")
                .status,
            "running"
        );
        assert!(mock.requests_of("prompt").is_empty());
    }

    /// The Agent finished the run while the app was closed: there is nothing to
    /// continue, and this pass does not settle the row either — the watchdog
    /// mirrors the Agent's terminal marker.
    #[tokio::test]
    async fn a_run_the_agent_finished_is_not_continued() {
        let home = TestHome::new("resume-terminal");
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-terminal"));
        let run = seed_run(&thread.id);
        let mock = mock_agent();
        mock.push_run_state(
            &run.id,
            serde_json::json!({"requestedRun": {"runId": run.id, "status": "completed"}}),
        );

        resume_interrupted_runs_at_launch(crate::store::now_millis()).await;

        assert_eq!(crate::store::list_runs(&thread.id).expect("runs").len(), 1);
        assert_eq!(
            crate::store::get_run(&run.id)
                .expect("get")
                .expect("some")
                .status,
            "running"
        );
        assert!(mock.requests_of("prompt").is_empty());
    }

    /// A run older than the window is not resurrected.
    #[tokio::test]
    async fn a_run_older_than_the_window_is_not_continued() {
        let home = TestHome::new("resume-stale");
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-stale"));
        let run = seed_run(&thread.id);
        let mock = mock_agent();
        mock.push_run_state(
            &run.id,
            serde_json::json!({"interruptedRun": {"runId": run.id}}),
        );
        let later = crate::store::now_millis() + RESUME_MAX_AGE.as_millis() as i64 + 1;

        resume_interrupted_runs_at_launch(later).await;

        assert_eq!(crate::store::list_runs(&thread.id).expect("runs").len(), 1);
        assert!(mock.requests_of("prompt").is_empty());
    }

    /// The switch is the user's consent to send prompts on their behalf: off
    /// means the Agent is not even asked.
    #[tokio::test]
    async fn the_switch_off_keeps_the_agent_untouched() {
        let home = TestHome::new("resume-switch-off");
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-off"));
        let run = seed_run(&thread.id);
        crate::store::update_app_settings(future_app_settings::UpdateAppSettingsInput {
            auto_resume_interrupted: Some(false),
            ..Default::default()
        })
        .expect("settings");
        let mock = mock_agent();

        resume_interrupted_runs_at_launch(crate::store::now_millis()).await;

        assert!(mock.requests_of("get_state").is_empty());
        assert!(mock.requests_of("prompt").is_empty());
        assert_eq!(
            crate::store::get_run(&run.id)
                .expect("get")
                .expect("some")
                .status,
            "running"
        );
    }

    /// A conversation parked on an approval is not continued: the tool call the
    /// user never decided on stays theirs to decide.
    #[tokio::test]
    async fn a_pending_approval_keeps_the_conversation_out() {
        let home = TestHome::new("resume-approval");
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-approval"));
        let run = seed_run(&thread.id);
        let mock = mock_agent();
        crate::store::test_support::seed_approval("approval-1", &run.id);
        mock.push_run_state(
            &run.id,
            serde_json::json!({"interruptedRun": {"runId": run.id}}),
        );

        resume_interrupted_runs_at_launch(crate::store::now_millis()).await;

        assert!(mock.requests_of("prompt").is_empty());
        assert_eq!(crate::store::list_runs(&thread.id).expect("runs").len(), 1);
    }

    /// Only the newest run of a conversation is a candidate: an interrupted row
    /// followed by a newer run has already been superseded.
    #[tokio::test]
    async fn only_the_newest_run_of_a_conversation_is_a_candidate() {
        let home = TestHome::new("resume-newest");
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-newest"));
        let older = seed_run(&thread.id);
        crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
            run_id: older.id.clone(),
            status: "cancelled".to_string(),
            error_message: Some("user stop".to_string()),
            error_type: Some("abort_requested".to_string()),
        })
        .expect("settle");
        let newer = seed_run(&thread.id);
        let runs = crate::store::list_runs(&thread.id).expect("runs");
        let candidate = candidate_of(&thread.id, "sess-newest", &runs).expect("a candidate");
        assert_eq!(candidate.run_id, newer.id);
        assert_ne!(candidate.run_id, older.id);
    }

    /// The marker precedence is read off the Agent's payload, not guessed.
    #[test]
    fn the_agent_payload_decides_the_decision() {
        let run = "run-1";
        assert_eq!(
            plan_resume(&serde_json::json!({"activeRun": {"runId": run}}), run),
            ResumeDecision::Live
        );
        assert_eq!(
            plan_resume(&serde_json::json!({"queuedRuns": [{"runId": run}]}), run),
            ResumeDecision::Live
        );
        assert_eq!(
            plan_resume(
                &serde_json::json!({"requestedRun": {"runId": run, "status": "completed"},
                    "interruptedRun": {"runId": run}}),
                run
            ),
            ResumeDecision::Terminal,
            "a durable terminal marker outranks the interrupted one"
        );
        assert_eq!(
            plan_resume(&serde_json::json!({"interruptedRun": {"runId": run}}), run),
            ResumeDecision::Resume
        );
        assert_eq!(
            plan_resume(
                &serde_json::json!({"interruptedRun": {"runId": "other"}}),
                run
            ),
            ResumeDecision::Unknown
        );
        assert_eq!(
            plan_resume(&serde_json::json!({}), run),
            ResumeDecision::Unknown
        );
    }

    /// A settled row is never a candidate: the only shapes this pass continues
    /// are the two non-terminal ones.
    #[test]
    fn a_settled_newest_row_is_not_a_candidate() {
        let home = TestHome::new("resume-settled");
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-settled"));
        let run = seed_run(&thread.id);
        crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
            run_id: run.id.clone(),
            status: "cancelled".to_string(),
            error_message: Some("user stop".to_string()),
            error_type: Some("abort_requested".to_string()),
        })
        .expect("settle");
        let runs = crate::store::list_runs(&thread.id).expect("runs");
        assert!(candidate_of(&thread.id, "sess-settled", &runs).is_none());
    }

    /// The prompt carries the run's own tail so the model knows what already
    /// happened; a run with no recorded events still gets the instruction.
    #[test]
    fn the_prompt_names_the_interruption_and_the_last_events() {
        let plain = build_resume_prompt("run-without-events");
        assert!(plain.starts_with("继续上一个任务。"));
        assert!(plain.contains("关闭 FutureOS 时被中断"));
        assert!(!plain.contains("已执行内容摘要"));
    }

    /// The mock's own `get_state` shape never claims a run it does not report.
    #[test]
    fn an_idle_session_reports_unknown() {
        assert_eq!(
            plan_resume(&get_state_payload("sess", false), "run-1"),
            ResumeDecision::Unknown
        );
    }

    /// A session the Agent refuses to describe leaves both the row and the
    /// conversation alone.
    #[tokio::test]
    async fn an_unanswerable_agent_continues_nothing() {
        let home = TestHome::new("resume-rejected");
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-rejected"));
        let run = seed_run(&thread.id);
        let mock = mock_agent();
        mock.push(
            &format!("get_state#{}", run.id),
            Reply::Reject("session is not mine".into()),
        );

        resume_interrupted_runs_at_launch(crate::store::now_millis()).await;

        assert_eq!(crate::store::list_runs(&thread.id).expect("runs").len(), 1);
        assert_eq!(
            crate::store::get_run(&run.id)
                .expect("get")
                .expect("some")
                .status,
            "running"
        );
        assert!(mock.requests_of("prompt").is_empty());
    }

    /// Two conversations interrupted together are both continued; the second
    /// prompt is not suppressed by the first's bookkeeping.
    #[tokio::test]
    async fn two_conversations_are_each_continued() {
        let home = TestHome::new("resume-two");
        let workspace = seed_workspace(home.path(), "ws");
        let first = seed_thread(&workspace.id, Some("sess-first"));
        let second = seed_thread(&workspace.id, Some("sess-second"));
        let first_run = seed_run(&first.id);
        let second_run = seed_run(&second.id);
        let mock = mock_agent();
        mock.push_run_state(
            &first_run.id,
            serde_json::json!({"interruptedRun": {"runId": first_run.id}}),
        );
        mock.push_run_state(
            &second_run.id,
            serde_json::json!({"interruptedRun": {"runId": second_run.id}}),
        );
        arm_prompt_pipeline(&mock, "sess-first", &workspace.path);
        arm_prompt_pipeline(&mock, "sess-second", &workspace.path);

        resume_interrupted_runs_at_launch(crate::store::now_millis()).await;

        let sessions: Vec<String> = mock
            .requests_of("prompt")
            .iter()
            .map(|request| request.session_id.clone())
            .collect();
        assert_eq!(sessions.len(), 2, "{sessions:?}");
        assert!(sessions.contains(&"sess-first".to_string()));
        assert!(sessions.contains(&"sess-second".to_string()));
    }

    /// A continuation that already exists for this run is not repeated: the
    /// marker in `trigger_message_id` is what tells the pass it already asked.
    #[tokio::test]
    async fn a_second_pass_does_not_repeat_a_continuation() {
        let home = TestHome::new("resume-idempotent");
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-idempotent"));
        let run = seed_run(&thread.id);
        let mock = mock_agent();
        mock.push_run_state(
            &run.id,
            serde_json::json!({"interruptedRun": {"runId": run.id}}),
        );
        arm_prompt_pipeline(&mock, "sess-idempotent", &workspace.path);
        resume_interrupted_runs_at_launch(crate::store::now_millis()).await;
        assert_eq!(mock.requests_of("prompt").len(), 1);

        // The interrupted row is settled by now, but a store that still shows it
        // non-terminal (or a launch that died before settling) must not ask the
        // same question again.
        crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
            run_id: run.id.clone(),
            status: "running".to_string(),
            error_message: None,
            error_type: None,
        })
        .expect("reopen the row");
        mock.push_run_state(
            &run.id,
            serde_json::json!({"interruptedRun": {"runId": run.id}}),
        );
        resume_interrupted_runs_at_launch(crate::store::now_millis()).await;
        assert_eq!(
            mock.requests_of("prompt").len(),
            1,
            "the existing continuation is found by its marker"
        );
    }
}
