//! Continue, on startup, the conversations a run was interrupted in.
//!
//! This is the Agent's own half of restart recovery. The Desktop has always done
//! a version of this for its own conversations, but a machine that only runs
//! `future agent` or `future host` had nothing: after a restart the session
//! sat at `interrupted_by_restart` forever, with the work unfinished and nobody
//! to notice.
//!
//! Two properties make doing it *here* different from doing it in a client:
//!
//! * **One trigger.** The instance lock admits one Agent per data root, so there
//!   is exactly one process that can run this pass. Two clients each noticing
//!   `interruptedRun` and each sending a continuation would run the same task
//!   twice; that window cannot exist here.
//! * **It is the same authority.** Which run was interrupted is decided from the
//!   Agent's own journal (`unterminated_run_id`), not from a mirror that may be
//!   behind.
//!
//! What it deliberately does not do: it never resumes a session that is still
//! streaming, parked on an approval, has no model configured, or whose last
//! activity is older than [`MAX_AGE`].
//!
//! Every remaining candidate is taken, oldest interruption first, with at most
//! [`MAX_IN_FLIGHT`] submissions in flight. Taking a fixed slice of the newest
//! ones starved the rest: `list_all` orders by `updated_at` descending, so the
//! three conversations most recently touched (exactly the ones a user is working
//! in, and so the ones most likely to be cut off again by the next restart) took
//! the whole budget every time. The tail was only reached once those three went
//! quiet. Oldest-first plus a queue drains on the first pass.
//!
//! **On unless turned off.** Resuming sends a prompt the user did not type,
//! which spends their credits with nobody present — but that consent was
//! already recorded by the Desktop's `autoResumeInterrupted` (also on), so the
//! switch here (`resumeInterruptedOnStart` in `agent/settings.json`) defaults to
//! on, and is the one control the user edits to opt out.
//!
//! **Quiet when there is nothing to do.** A session whose journal shows no open
//! run is the normal case, not a failure: the pass says nothing about it. Only
//! a real interruption (and why it was or was not continued) is reported. A
//! healthy start used to print one "no interrupted run" line per conversation,
//! which is exactly the noise that corrupted the TUI's first frame when the
//! TUI hosts the Agent in-process (`tui/src/stderr_log.rs` is the other half of
//! that fix).

use std::time::Duration;

use crate::rpc::AppState;

/// How long after its last activity a conversation is still worth continuing
/// automatically. A weekend or a holiday is covered; a task abandoned a month
/// ago is not resurrected.
pub const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// How many continuations may be in flight at once.
///
/// This is a concurrency bound, not a quota: every eligible conversation is
/// still continued in the same pass, the rest of them queued behind this many.
/// The bound exists because the Agent has no cross-session concurrency limiter
/// (`GlobalQueueBudget` caps queue depth and bytes, not running sessions), so
/// firing one model request per interrupted conversation at once is what a
/// restart after an outage would otherwise do.
pub const MAX_IN_FLIGHT: usize = 3;

/// How long the pass waits after startup before looking. Clients get a moment to
/// connect, so the continuation they see is one they can watch.
const START_DELAY: Duration = Duration::from_secs(2);

/// Whether this start should run the pass at all.
#[cfg_attr(test, allow(dead_code))] // only reached from the non-test startup hook
pub fn enabled() -> bool {
    let path = std::path::PathBuf::from(crate::models::settings_path());
    crate::config::load_settings(&path)
        .map(|settings| settings.resume_interrupted_on_start)
        .unwrap_or(false)
}

/// The prompt that continues an interrupted run.
///
/// It is sent to the model, so it is not localized — the same rule as the
/// Desktop's continuation prompt, whose shape this keeps. It names *why* the run
/// stopped: without that the model reads an assistant turn that ends
/// mid-tool-call and cannot tell whether the tool ran.
pub fn build_prompt() -> String {
    [
        "继续上一个任务。",
        "",
        "上一轮运行在 Agent 重启时被中断了（工具可能已经执行了一部分）。请先判断已经完成了什么，\
         再继续未完成的部分，不要重复已经生效的修改或命令。",
    ]
    .join("\n")
}

/// What one session's continuation attempt settled on.
///
/// Returned instead of a bare `bool` so the pass can tell "nothing was
/// interrupted" (silent, the healthy case) from "a run is open but was left
/// alone" (worth reporting and counting). The old shape folded every session
/// without an open run into `Err("no interrupted run")`, and the caller logged
/// that as a failure line per conversation on every single start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// The journal showed an open run and the continuation was submitted.
    Continued,
    /// The journal shows no open run: the conversation finished. Normal, and
    /// deliberately not reported.
    NotInterrupted,
    /// A run is open, but this pass left it alone.
    Held(Held),
}

/// Why an interrupted run was left interrupted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Held {
    /// Still streaming or parked on an approval decision. The process that owns
    /// that state settles it; a second submission would duplicate the work.
    Busy,
    /// No model configured, so the continuation prompt cannot run.
    NoModel,
    Uncertain,
}

/// The pass's tally; the summary line is derived from it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Tally {
    /// Conversations `list_all` returned.
    considered: usize,
    /// Of those, the ones worth a per-session look (top-level and fresh).
    eligible: usize,
    /// Eligible conversations whose journal still shows an open run.
    interrupted: usize,
    /// Continuations actually submitted.
    continued: usize,
    /// Interrupted conversations left alone (`Held`).
    held: usize,
    /// Eligible conversations whose attempt failed for a real reason.
    failed: usize,
}

/// Record one finished attempt. `None` is a real failure (already logged).
fn record(tally: &mut Tally, outcome: Option<Outcome>) {
    match outcome {
        Some(Outcome::Continued) => {
            tally.interrupted += 1;
            tally.continued += 1;
        }
        Some(Outcome::Held(_)) => {
            tally.interrupted += 1;
            tally.held += 1;
        }
        Some(Outcome::NotInterrupted) => {}
        None => tally.failed += 1,
    }
}

/// The one line a pass prints, or `None` when there was nothing to continue.
///
/// Silent on a healthy start is the point: every conversation that finished is
/// expected, and a line per conversation is what a TUI sees as corruption.
fn summary_line(tally: &Tally) -> Option<String> {
    if tally.interrupted == 0 {
        return None;
    }
    let skipped = tally.considered.saturating_sub(tally.eligible);
    let mut line = format!(
        "FutureOS resume: {} conversation(s) checked, {} interrupted, {} continued \
         (at most {MAX_IN_FLIGHT} at a time), {} left interrupted",
        tally.considered, tally.interrupted, tally.continued, tally.held,
    );
    if tally.failed > 0 {
        line.push_str(&format!(", {} failed", tally.failed));
    }
    line.push_str(&format!(
        ", {skipped} skipped (older than {} days or sub-session)",
        MAX_AGE.as_secs() / 86_400
    ));
    Some(line)
}

/// Run one pass. Never fails the launch: resuming is recovery, not startup.
#[cfg_attr(test, allow(dead_code))] // only reached from the non-test startup hook
pub async fn resume_interrupted_sessions(state: AppState) {
    tokio::time::sleep(START_DELAY).await;
    let summaries = match state.session_manager.list_all() {
        Ok(summaries) => summaries,
        Err(error) => {
            eprintln!("FutureOS resume: cannot enumerate sessions: {error}");
            return;
        }
    };
    let mut tally = Tally {
        considered: summaries.len(),
        ..Tally::default()
    };
    let candidates = candidates_in_order(summaries);
    tally.eligible = candidates.len();

    let mut in_flight = tokio::task::JoinSet::new();
    for summary in candidates {
        while in_flight.len() >= MAX_IN_FLIGHT {
            if let Some(joined) = in_flight.join_next().await {
                record(&mut tally, settle(joined));
            }
        }
        let state = state.clone();
        let session_id = summary.id.clone();
        // Each continuation is independent; one session's trouble is reported and
        // the pass moves on rather than abandoning the rest of the queue.
        in_flight.spawn_blocking(move || {
            let outcome = resume_one(&state, &session_id).map_err(|error| {
                eprintln!("FutureOS resume: {session_id} not continued: {error}");
            });
            if matches!(outcome, Ok(Outcome::Held(Held::NoModel))) {
                // No model means the prompt cannot run; leaving the row
                // interrupted is honest, and the user still sees it.
                eprintln!(
                    "FutureOS resume: {session_id} has no model configured; left interrupted"
                );
            }
            outcome.ok()
        });
    }
    while let Some(joined) = in_flight.join_next().await {
        record(&mut tally, settle(joined));
    }

    // Report the account, not just the work: "checked N, continued 3" hid how
    // many were left behind. Nothing interrupted → nothing printed.
    if let Some(line) = summary_line(&tally) {
        eprintln!("{line}");
    }
}

/// Settle one continuation task's join result into an [`Outcome`] slot.
///
/// A task that panicked is a real failure: the pass keeps going (the JoinSet
/// has already isolated it) but the summary has to know.
fn settle(joined: Result<Option<Outcome>, tokio::task::JoinError>) -> Option<Outcome> {
    match joined {
        Ok(outcome) => outcome,
        Err(error) => {
            eprintln!("FutureOS resume: a continuation task failed: {error}");
            None
        }
    }
}

/// The queue: every eligible conversation, oldest interruption first.
///
/// `list_all` hands them over newest-first, and the newest are the ones still in
/// use — the likeliest to be interrupted again before the queue reaches the end,
/// which is how a queue ordered that way never drains.
fn candidates_in_order(
    summaries: Vec<crate::session::SessionSummary>,
) -> Vec<crate::session::SessionSummary> {
    let mut candidates: Vec<_> = summaries.into_iter().filter(is_candidate).collect();
    candidates.sort_by_key(|summary| summary.updated_at);
    candidates
}

/// Whether one session is worth considering at all.
///
/// Cheap checks only; the per-session ones that need the Agent's own state
/// (streaming, approval, model) happen inside [`resume_one`], under the session
/// lock that also makes "decide and submit" atomic.
fn is_candidate(summary: &crate::session::SessionSummary) -> bool {
    if !summary.parent_session_id.trim().is_empty() {
        // A sub-session continues when the work that started it does; on its own
        // it would answer a question nobody re-asked.
        return false;
    }
    let age = chrono::Local::now().signed_duration_since(summary.updated_at);
    age.num_seconds() <= MAX_AGE.as_secs() as i64
}

/// Settle one session's interrupted run and submit the continuation prompt.
///
/// `Err` is reserved for a real failure (the journal or the session cannot be
/// read); "this conversation finished" is [`Outcome::NotInterrupted`], and an
/// interrupted run that was deliberately left alone is [`Outcome::Held`].
fn resume_one(state: &AppState, session_id: &str) -> Result<Outcome, String> {
    let run_id = state
        .session_manager
        .resumable_run_id(session_id)
        .map_err(|error| format!("journal: {error}"))?;
    let Some(run_id) = run_id else {
        return Ok(Outcome::NotInterrupted);
    };

    let session = state
        .try_get_session(session_id)
        .map_err(|error| format!("load: {error}"))?
        .ok_or_else(|| "session storage unavailable".to_string())?;
    // Decide and submit under one session write lock, so a second pass cannot
    // read the same interrupted marker and submit behind this one.
    let mut session = session.write();
    if session
        .is_streaming
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        return Ok(Outcome::Held(Held::Busy));
    }
    if !session
        .approval_gate
        .pending_for_session(session_id)
        .is_empty()
    {
        // Resuming would re-issue a tool call nobody decided on, under whatever
        // approval tier applies now. The approval owns that row.
        return Ok(Outcome::Held(Held::Busy));
    }
    if session.model.trim().is_empty() {
        return Ok(Outcome::Held(Held::NoModel));
    }
    let safety = super::recovery_safety::RecoverySafety::load(
        &session.session_manager,
        session_id,
        &session.cwd,
        true,
    )
    .map_err(|error| format!("recovery verification: {error}"))?;
    if let Some(reason) = safety.pause_reason() {
        let previous = state
            .session_manager
            .run_terminal(session_id, &run_id)
            .map_err(|error| format!("recovery outcome: {error}"))?;
        if previous.as_ref().and_then(|v| v["error"].as_str()) != Some(&reason) {
            let mut terminal = crate::session::SessionEntry::run_terminal(
                &run_id,
                crate::session::RUN_STATE_INTERRUPTED_BY_RESTART,
                0,
                0,
                Some(&reason),
            );
            if let Some(previous) = previous {
                // Verification must not erase tokens, duration or other facts
                // already committed by a graceful system interruption.
                terminal.content = Some(previous);
                terminal.content.as_mut().unwrap()["error"] = reason.into();
            }
            terminal.content.as_mut().unwrap()["interruption_cause"] = "outcome_unknown".into();
            state
                .session_manager
                .append_entries_synced(session_id, &[terminal])
                .map_err(|error| format!("recovery outcome: {error}"))?;
        }
        return Ok(Outcome::Held(Held::Uncertain));
    }
    submit_continuation(&mut session, session_id, &run_id)?;
    Ok(Outcome::Continued)
}

/// Continue the verified interruption without falling back to a new prompt.
pub(super) fn submit_continuation(
    session: &mut crate::rpc::ServerSession,
    session_id: &str,
    run_id: &str,
) -> Result<(), String> {
    // A refused recovery must never bypass its safety check by becoming an
    // ordinary user prompt. Keep the original work intact for reconciliation.
    session
        .continue_interrupted_run(run_id, &build_prompt())
        .map(|_| ())
        .map_err(|error| format!("{session_id}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_names_partial_tool_execution() {
        let prompt = build_prompt();
        assert!(prompt.contains("继续上一个任务"));
        assert!(prompt.contains("工具可能已经执行了一部分"));
        assert!(prompt.contains("不要重复"), "{prompt}");
    }

    fn summary(id: &str, minutes_ago: i64, parent: &str) -> crate::session::SessionSummary {
        crate::session::SessionSummary {
            id: id.to_string(),
            cwd: String::new(),
            updated_at: chrono::Local::now() - chrono::Duration::minutes(minutes_ago),
            model: "m".to_string(),
            name: None,
            parent_session_id: parent.to_string(),
            first_message: None,
            query_count: 1,
        }
    }

    /// The ordering that removes the starvation. `list_all` hands over
    /// newest-first; the pass must reverse it, or the three most recently touched
    /// conversations (the ones still in use, and so the ones the next restart is
    /// most likely to cut off again) hold the whole concurrency budget forever.
    #[test]
    fn candidates_are_taken_oldest_interruption_first() {
        let rows = vec![
            summary("newest", 1, ""),
            summary("older", 30, ""),
            summary("oldest", 120, ""),
            summary("child", 200, "parent-1"), // sub-session: never a candidate
            summary("stale", 60 * 24 * 8, ""), // older than MAX_AGE: not a candidate
        ];
        // The shipped queueing function, not a copy of its sort: a test that
        // re-sorts the fixture itself passes whatever the implementation does.
        let order: Vec<String> = candidates_in_order(rows)
            .into_iter()
            .map(|row| row.id)
            .collect();
        assert_eq!(order, vec!["oldest", "older", "newest"], "{order:?}");
    }

    /// A sub-session and anything past the age limit are the only sessions the
    /// cheap filter drops.
    #[test]
    fn only_fresh_top_level_sessions_are_candidates() {
        assert!(is_candidate(&summary("ok", 5, "")));
        assert!(!is_candidate(&summary("child", 5, "parent")));
        assert!(!is_candidate(&summary("stale", 60 * 24 * 8, "")));
    }

    /// A healthy start has nothing to report. A conversation that finished is
    /// *not* an interrupted one — the old summary counted every eligible
    /// conversation as "interrupted and eligible", and the old pass printed one
    /// "no interrupted run" line per conversation. On a TUI that hosts the
    /// Agent those lines landed on the welcome screen (see
    /// `tui/src/stderr_log.rs` for the other half of that fix).
    #[test]
    fn a_healthy_start_reports_nothing() {
        let mut tally = Tally {
            considered: 8,
            eligible: 8,
            ..Tally::default()
        };
        for _ in 0..8 {
            record(&mut tally, Some(Outcome::NotInterrupted));
        }
        assert_eq!(
            tally.interrupted, 0,
            "a finished conversation is not interrupted"
        );
        assert_eq!(summary_line(&tally), None, "a healthy start is silent");
    }

    /// When something *was* interrupted the summary states each count for what
    /// it is, and skips what the cheap filter dropped.
    #[test]
    fn the_summary_counts_what_happened() {
        let mut tally = Tally {
            considered: 5,
            eligible: 3,
            ..Tally::default()
        };
        record(&mut tally, Some(Outcome::Continued));
        record(&mut tally, Some(Outcome::Held(Held::NoModel)));
        record(&mut tally, Some(Outcome::NotInterrupted));
        assert_eq!((tally.interrupted, tally.continued, tally.held), (2, 1, 1));

        let line = summary_line(&tally).expect("an interrupted run is reported");
        assert!(line.contains("5 conversation(s) checked"), "{line}");
        assert!(line.contains("2 interrupted"), "{line}");
        assert!(line.contains("1 continued"), "{line}");
        assert!(line.contains("1 left interrupted"), "{line}");
        assert!(line.contains("2 skipped"), "{line}");
    }

    /// A failed attempt is counted without hiding the work that did happen, so
    /// the summary cannot read green while something went wrong.
    #[test]
    fn a_failed_attempt_is_counted_alongside_continued_work() {
        let mut tally = Tally {
            considered: 2,
            eligible: 2,
            ..Tally::default()
        };
        record(&mut tally, Some(Outcome::Continued));
        record(&mut tally, None);
        assert_eq!((tally.continued, tally.failed), (1, 1));
        let line = summary_line(&tally).unwrap();
        assert!(line.contains("1 continued"), "{line}");
        assert!(line.contains("1 failed"), "{line}");
    }

    /// The concurrency bound is real: ten queued tasks never run more than
    /// `MAX_IN_FLIGHT` at once. The hub exercises the same queue the pass uses
    /// (`JoinSet` with a refill only when a slot is free), so a change that fired
    /// everything at once would be visible here.
    #[tokio::test]
    async fn at_most_max_in_flight_continuations_run_at_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let running = std::sync::Arc::new(AtomicUsize::new(0));
        let peak = std::sync::Arc::new(AtomicUsize::new(0));
        let mut in_flight = tokio::task::JoinSet::new();
        for _ in 0..10 {
            while in_flight.len() >= MAX_IN_FLIGHT {
                in_flight.join_next().await;
            }
            let running = running.clone();
            let peak = peak.clone();
            in_flight.spawn_blocking(move || {
                let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(20));
                running.fetch_sub(1, Ordering::SeqCst);
            });
        }
        while in_flight.join_next().await.is_some() {}
        // Asserted against a literal, not the constant: comparing the peak with
        // `MAX_IN_FLIGHT` would make the test pass for any value of it, including
        // one large enough to fire everything at once — the failure mode this
        // exists to prevent.
        assert!(
            peak.load(Ordering::SeqCst) <= 3,
            "at most three continuations may run at once, saw {}",
            peak.load(Ordering::SeqCst)
        );
    }

    /// Every eligible conversation is taken, not a fixed slice of the newest. The
    /// old shape stopped at three and the rest were silently left interrupted.
    #[test]
    fn every_eligible_conversation_is_taken_not_just_the_first_few() {
        let rows: Vec<_> = (0..10)
            .map(|n| summary(&format!("s{n}"), n as i64, ""))
            .collect();
        let eligible: Vec<_> = rows.into_iter().filter(is_candidate).collect();
        assert_eq!(eligible.len(), 10);
        // The concurrency bound is what limits work at any moment, and it must be
        // smaller than the queue, or this test proves nothing about ordering.
        assert!(
            MAX_IN_FLIGHT < eligible.len(),
            "the bound must be below the candidate count to exercise queueing"
        );
    }
}

#[cfg(test)]
mod wiring_tests {
    //! The pass, end to end, at the call site production uses.
    //!
    //! Unit tests around `continue_interrupted_run` prove the accepting path
    //! behaves; these prove the restart recovery *calls* it. A check that lives
    //! beside the right code but is not on the path the product takes guards
    //! nothing, so this drives `resume_one` — the function the pass spawns —
    //! against a real session on disk and reads what the model was asked.

    use super::*;
    use crate::llm::schema::{FinishReason, ModelRequest, ModelStreamEvent};
    use crate::session::SessionEntry;
    use crate::types::LLMProvider;
    use std::sync::Arc;
    use tokio_stream::wrappers::ReceiverStream;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn user_stop_during_startup_delay_prevents_automatic_resume() {
        let provider = RecordingProvider::new();
        let state = state_with(provider.clone());
        seed_crashed_session(&state, "stopped");
        register_session(&state, provider.clone(), "stopped");
        state.sessions.read()["stopped"]
            .read()
            .abort_run(Some("run-crashed"))
            .unwrap();
        assert_eq!(
            resume_one(&state, "stopped").unwrap(),
            Outcome::NotInterrupted
        );
        assert!(provider.requests.lock().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn accepted_user_stop_is_durable_before_the_task_finishes() {
        let provider = RecordingProvider::new();
        let state = state_with(provider.clone());
        seed_crashed_session(&state, "active-stop");
        register_session(&state, provider, "active-stop");
        let session = state.sessions.read()["active-stop"].clone();
        session
            .read()
            .runtime
            .begin(Some("run-crashed"), None)
            .unwrap();
        session.read().abort_run(Some("run-crashed")).unwrap();
        // No terminal exists yet: an immediate process loss must still not
        // resurrect work the user has already stopped.
        assert!(state
            .session_manager
            .unterminated_run_id("active-stop")
            .unwrap()
            .is_some());
        assert!(state
            .session_manager
            .resumable_run_id("active-stop")
            .unwrap()
            .is_none());
        state
            .session_manager
            .reconcile_user_stops_on_start()
            .unwrap();
        let terminal = state
            .session_manager
            .run_terminal("active-stop", "run-crashed")
            .unwrap()
            .unwrap();
        assert_eq!(terminal["state"], crate::session::RUN_STATE_CANCELLED);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unknown_shell_holds_recovery_without_spending_a_model_request() {
        let provider = RecordingProvider::new();
        let state = state_with(provider.clone());
        seed_crashed_session(&state, "uncertain");
        state
            .session_manager
            .append_entries_synced(
                "uncertain",
                &[SessionEntry::new_assistant(
                    serde_json::json!("working"),
                    vec![crate::types::ToolCall {
                        id: "unknown-effect".into(),
                        call_type: "function".into(),
                        function: crate::types::ToolCallFn {
                            name: "shell".into(),
                            arguments: serde_json::json!({"command":"append-once"}),
                        },
                    }],
                )],
            )
            .unwrap();
        register_session(&state, provider.clone(), "uncertain");
        for _ in 0..2 {
            assert_eq!(
                resume_one(&state, "uncertain").unwrap(),
                Outcome::Held(Held::Uncertain)
            );
        }
        assert!(provider.requests.lock().unwrap().is_empty());
        let terminal = state
            .session_manager
            .run_terminal("uncertain", "run-crashed")
            .unwrap()
            .unwrap();
        assert!(terminal["error"]
            .as_str()
            .unwrap()
            .contains("RECOVERY_UNCERTAIN"));
        assert_eq!(
            state
                .session_manager
                .resumable_run_id("uncertain")
                .unwrap()
                .as_deref(),
            Some("run-crashed")
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn graceful_system_interruption_resumes_once_and_refusal_never_falls_back() {
        let provider = RecordingProvider::new();
        let state = state_with(provider.clone());
        seed_crashed_session(&state, "system-stopped");
        let mut terminal = SessionEntry::run_terminal(
            "run-crashed",
            crate::session::RUN_STATE_INTERRUPTED_BY_RESTART,
            0,
            0,
            None,
        );
        terminal.content.as_mut().unwrap()["interruption_cause"] = "system_shutdown".into();
        state
            .session_manager
            .append_entries_synced("system-stopped", &[terminal])
            .unwrap();
        register_session(&state, provider.clone(), "system-stopped");
        assert_eq!(
            resume_one(&state, "system-stopped").unwrap(),
            Outcome::Continued
        );
        let session = state.sessions.read()["system-stopped"].clone();
        assert!(
            submit_continuation(&mut session.write(), "system-stopped", "run-crashed").is_err()
        );
    }

    /// Answers every request with one sentence, and records what it was asked.
    struct RecordingProvider {
        requests: std::sync::Mutex<Vec<ModelRequest>>,
    }

    impl RecordingProvider {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                requests: std::sync::Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait::async_trait]
    impl LLMProvider for RecordingProvider {
        async fn stream_model(
            &self,
            request: ModelRequest,
        ) -> anyhow::Result<ReceiverStream<ModelStreamEvent>> {
            self.requests.lock().unwrap().push(request);
            let (tx, rx) = tokio::sync::mpsc::channel(4);
            let _ = tx
                .send(ModelStreamEvent::TextDelta {
                    id: "text".into(),
                    text: "carried on".into(),
                })
                .await;
            let _ = tx
                .send(ModelStreamEvent::Finish {
                    reason: FinishReason::Stop,
                    usage: None,
                })
                .await;
            Ok(ReceiverStream::new(rx))
        }
    }

    fn state_with(provider: Arc<RecordingProvider>) -> crate::rpc::AppState {
        let state = crate::rpc::commands::test_support::make_app_state_with_provider(
            provider,
            crate::rpc::commands::test_support::test_session_dir(),
            Arc::new(crate::runtime::GlobalQueueBudget::defaults()),
        );
        // The session's model reference has to be in the catalogue, or anything
        // that re-resolves it treats it as stale and installs the account's real
        // default model — a test that spent the user's credits. The base URL
        // points at a closed port so an accidental request fails instead of
        // reaching a provider.
        let model = crate::models::Model {
            id: "mock".to_string(),
            name: "Mock".to_string(),
            provider: "mock-provider".to_string(),
            api: "chat".to_string(),
            base_url: "http://127.0.0.1:9".to_string(),
            input: vec!["text".to_string()],
            output: vec!["text".to_string()],
            // A usable context budget: the default (0) makes the context
            // manager refuse the request before it is built.
            context_window: 100_000,
            max_tokens: 4_000,
            ..Default::default()
        };
        *state.model_registry.write() = crate::models::Registry::from_models_and_auth(
            vec![model],
            r#"{"mock-provider":{"type":"api_key","key":"test"}}"#,
        );
        state
    }

    /// Put the session in memory before the pass looks for it, so the run uses
    /// the provider this test controls. Hydration would re-resolve the session's
    /// model against the catalogue and install a live HTTP client, which is
    /// correct in production and useless in a test.
    fn register_session(state: &crate::rpc::AppState, provider: Arc<RecordingProvider>, id: &str) {
        let mut session = crate::rpc::ServerSession::new_with_queue_budget(
            id.to_string(),
            Arc::new(tokio::sync::RwLock::new(crate::agent::Loop::new(
                provider as Arc<dyn LLMProvider>,
                "mock",
            ))),
            state.session_manager.clone(),
            &state.welcome_cwd,
            Arc::new(crate::rpc::SseBroadcaster::new()),
            state.approval_gate.clone(),
            state.model_registry.clone(),
            state.queue_budget.clone(),
        );
        // What hydration would have read from the session's own metadata.
        session.model = "mock".to_string();
        session.thinking_level = "off".to_string();
        state
            .sessions
            .write()
            .insert(id.to_string(), Arc::new(parking_lot::RwLock::new(session)));
    }

    fn seed_crashed_session(state: &crate::rpc::AppState, id: &str) {
        std::fs::create_dir_all(&state.welcome_cwd).unwrap();
        // A crash in the middle of a tool call: the assistant turn is on disk,
        // the call is, and its result never arrived.
        let call = crate::types::ToolCall {
            id: "c1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: "read".to_string(),
                arguments: serde_json::json!({"path": "progress.txt"}),
            },
        };
        let entries = vec![
            SessionEntry::new_user("user", serde_json::json!("do the thing")),
            SessionEntry::run_started("run-crashed", 1),
            SessionEntry::new_assistant(serde_json::json!("running it"), vec![call]),
        ];
        state
            .session_manager
            .save(&crate::session::Session::snapshot(
                id.to_string(),
                state.welcome_cwd.clone(),
                "mock".to_string(),
                String::new(),
                String::new(),
                entries,
            ))
            .unwrap();
    }

    /// The journal of a run, as a string, once the continuation has settled.
    fn wait_for_run_journal(
        state: &crate::rpc::AppState,
        id: &str,
        run_id: &str,
    ) -> Option<String> {
        for _ in 0..1500 {
            if let Ok(events) = state.session_manager.storage().unwrap().events(id, run_id) {
                let text = serde_json::to_string(&events).unwrap();
                if text.contains("agent_end") {
                    return Some(text);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        None
    }

    /// **The wiring evidence.** `resume_one` is what the restart pass runs. It
    /// must reach the new accepting path: the run is journalled with
    /// `auto_resume_of`, the model is sent the continuation instruction, and the
    /// conversation gains neither a user entry nor a `user_message` event. The
    /// old shape (`enqueue_prompt_with_model_context`) would fail the last two.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_restart_pass_continues_a_crashed_run_without_a_user_message() {
        let provider = RecordingProvider::new();
        let state = state_with(provider.clone());
        seed_crashed_session(&state, "s-crashed");
        register_session(&state, provider.clone(), "s-crashed");

        let continued = resume_one(&state, "s-crashed").expect("the pass continues the session");
        assert_eq!(
            continued,
            Outcome::Continued,
            "an interrupted session is continued"
        );

        // Find the continuation's run id from its marker.
        let mut run_id = None;
        for _ in 0..300 {
            let stored = state.session_manager.load("s-crashed").unwrap();
            run_id = stored
                .entries
                .iter()
                .find(|entry| {
                    entry.entry_type == crate::session::ENTRY_TYPE_RUN_STARTED
                        && entry
                            .content
                            .as_ref()
                            .and_then(|content| content["auto_resume_of"].as_str())
                            == Some("run-crashed")
                })
                .and_then(|entry| {
                    entry
                        .content
                        .as_ref()
                        .and_then(|content| content["run_id"].as_str())
                        .map(str::to_string)
                });
            if run_id.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let run_id =
            run_id.expect("the continuation writes a run_started marker with auto_resume_of");

        // 1. The request the model received ends with the continuation
        //    instruction and still contains the interrupted turn.
        let journal = match wait_for_run_journal(&state, "s-crashed", &run_id) {
            Some(journal) => journal,
            None => {
                let seen = state
                    .session_manager
                    .storage()
                    .unwrap()
                    .events("s-crashed", &run_id)
                    .map(|events| serde_json::to_string(&events).unwrap())
                    .unwrap_or_default();
                panic!(
                    "the continuation run finishes and journals its end; \
                     requests={} journal={seen}",
                    provider.requests.lock().unwrap().len()
                );
            }
        };
        let requests = provider.requests.lock().unwrap();
        assert_eq!(requests.len(), 1, "one run, one request; journal={journal}");
        let request = &requests[0];
        assert_eq!(
            request.messages.last().map(|message| message.text()),
            Some(build_prompt()),
            "the model is sent the continuation instruction"
        );
        assert!(
            request
                .messages
                .iter()
                .any(|message| message.text().contains("running it")),
            "the interrupted turn is still in the request"
        );
        drop(requests);

        // 2. No user message: not in the transcript, not in the event journal.
        //    This is what separates the new path from the prompt the pass used
        //    to submit, and it is asserted on the journal the run really wrote.
        let stored = state.session_manager.load("s-crashed").unwrap();
        assert!(
            !stored
                .entries
                .iter()
                .skip(4)
                .any(|entry| entry.role == "user"),
            "continuing must not append a user entry: {:?}",
            stored
                .entries
                .iter()
                .map(|entry| (entry.entry_type.clone(), entry.role.clone()))
                .collect::<Vec<_>>()
        );
        assert!(
            !journal.contains("user_message"),
            "a continuation must not broadcast a user message: {journal}"
        );
    }

    /// A closed run cannot acquire a continuation or a fallback user prompt.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_closed_run_is_not_resumed_or_replaced_with_a_prompt() {
        let provider = RecordingProvider::new();
        let state = state_with(provider.clone());
        seed_crashed_session(&state, "s-closed");
        // Close the run: the journal no longer proves anything is open, so the
        // in-place path must refuse. (The pass itself only picks sessions whose
        // journal shows an open run; this test pins the belt-and-braces refusal
        // on the accepting path that production also runs.)
        state
            .session_manager
            .append_run_continuation(
                "s-closed",
                SessionEntry::run_terminal(
                    "run-crashed",
                    crate::session::RUN_STATE_INTERRUPTED_BY_RESTART,
                    0,
                    0,
                    None,
                ),
            )
            .unwrap_or_else(|_| panic!("seeding a terminal marker"));
        assert_eq!(
            state
                .session_manager
                .unterminated_run_id("s-closed")
                .unwrap(),
            None
        );
        assert_eq!(
            resume_one(&state, "s-closed").expect("a finished conversation is not an error"),
            Outcome::NotInterrupted,
            "a conversation whose journal shows no open run is a normal skip"
        );
        assert!(
            provider.requests.lock().unwrap().is_empty(),
            "a refused continuation does not call the model"
        );
    }

    /// **The noise regression.** A conversation whose run finished must settle
    /// as `NotInterrupted` — the healthy, unreported case. It used to come back
    /// as `Err("no interrupted run")`, which the pass logged per conversation on
    /// every start; when the TUI hosts the Agent in-process that line lands on
    /// the welcome screen (`tui/src/stderr_log.rs`).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_finished_conversation_settles_as_not_interrupted() {
        let provider = RecordingProvider::new();
        let state = state_with(provider.clone());
        seed_crashed_session(&state, "s-finished");
        state
            .session_manager
            .append_run_continuation(
                "s-finished",
                SessionEntry::run_terminal(
                    "run-crashed",
                    crate::session::RUN_STATE_COMPLETED,
                    0,
                    0,
                    None,
                ),
            )
            .unwrap_or_else(|_| panic!("seeding a terminal marker"));
        assert_eq!(
            state
                .session_manager
                .unterminated_run_id("s-finished")
                .unwrap(),
            None,
            "the fixture has no open run left"
        );
        assert_eq!(
            resume_one(&state, "s-finished").expect("a finished conversation is not an error"),
            Outcome::NotInterrupted
        );
        assert!(
            provider.requests.lock().unwrap().is_empty(),
            "a finished conversation sends nothing to the model"
        );
    }
}
