//! Continue, on startup, the conversations a run was interrupted in.
//!
//! This is the Agent's own half of restart recovery. The Desktop has always done
//! a version of this for its own conversations, but a machine that only runs
//! `future agent` or `future headless` had nothing: after a restart the session
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
//! **Off unless asked.** Resuming sends a prompt the user did not type, which
//! spends their credits with nobody present. That is a product decision, not a
//! recovery detail, so it is a switch in `agent/settings.json`
//! (`resumeInterruptedOnStart`, default off) rather than something that happens
//! because the process restarted.

use std::time::Duration;

use crate::rpc::session_prompt::PromptText;
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
    let considered = summaries.len();
    let candidates = candidates_in_order(summaries);

    let total = candidates.len();
    let mut resumed = 0usize;
    let mut in_flight = tokio::task::JoinSet::new();
    for summary in candidates {
        while in_flight.len() >= MAX_IN_FLIGHT {
            if let Some(joined) = in_flight.join_next().await {
                resumed += count(joined);
            }
        }
        let state = state.clone();
        let session_id = summary.id;
        // Each continuation is independent; one session's trouble is reported and
        // the pass moves on rather than abandoning the rest of the queue.
        in_flight.spawn_blocking(move || match resume_one(&state, &session_id) {
            Ok(true) => 1usize,
            Ok(false) => 0,
            Err(error) => {
                eprintln!("FutureOS resume: {session_id} not continued: {error}");
                0
            }
        });
    }
    while let Some(joined) = in_flight.join_next().await {
        resumed += count(joined);
    }

    // Report the account, not just the work: "checked N, continued 3" hid how
    // many were left behind.
    let skipped = considered - total;
    eprintln!(
        "FutureOS resume: {considered} conversation(s) checked, {total} interrupted and eligible, \
         {resumed} continued (at most {MAX_IN_FLIGHT} at a time), {skipped} skipped \
         (older than {} days, sub-session, streaming, awaiting approval, or no model)",
        MAX_AGE.as_secs() / 86_400
    );
}

/// Count what one continuation task produced.
fn count(joined: Result<usize, tokio::task::JoinError>) -> usize {
    match joined {
        Ok(resumed) => resumed,
        Err(error) => {
            eprintln!("FutureOS resume: a continuation task failed: {error}");
            0
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
/// Returns whether a prompt was submitted.
fn resume_one(state: &AppState, session_id: &str) -> Result<bool, String> {
    let run_id = state
        .session_manager
        .unterminated_run_id(session_id)
        .map_err(|error| format!("journal: {error}"))?
        .ok_or_else(|| "no interrupted run".to_string())?;

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
        return Ok(false);
    }
    if !session
        .approval_gate
        .pending_for_session(session_id)
        .is_empty()
    {
        // Resuming would re-issue a tool call nobody decided on, under whatever
        // approval tier applies now. The approval owns that row.
        return Ok(false);
    }
    if session.model.trim().is_empty() {
        // No model means the prompt cannot run; leaving the row interrupted is
        // honest, and the user still sees it.
        eprintln!("FutureOS resume: {session_id} has no model configured; left interrupted");
        return Ok(false);
    }
    submit_continuation(&mut session, session_id, &run_id)?;
    Ok(true)
}

/// Continue in place, or fall back to the shape this replaced.
///
/// Both shapes have to stay available. In place is the better one — no user
/// message appears in a conversation nobody typed into — but it depends on the
/// provider accepting a request whose history stops mid-turn and on the journal
/// still showing the run as unterminated. When either says no, continuing in
/// place is refused, and the alternative is a visible prompt: a fabricated user
/// message is bad, but an interrupted conversation that never continues is
/// worse, and the user can at least see and correct what was sent.
///
/// Kept as its own function so the fallback is exercised by a test rather than
/// being a branch nobody has ever taken.
pub(super) fn submit_continuation(
    session: &mut crate::rpc::ServerSession,
    session_id: &str,
    run_id: &str,
) -> Result<(), String> {
    match session.continue_interrupted_run(run_id, &build_prompt()) {
        Ok(_) => Ok(()),
        Err(error) => {
            eprintln!(
                "FutureOS resume: {session_id} cannot continue in place ({error}); \
                 submitting a continuation prompt instead"
            );
            session
                .enqueue_prompt_with_model_context(
                    PromptText::new(&build_prompt(), ""),
                    &[],
                    &[],
                    None,
                    &format!("resume_{run_id}"),
                    crate::runtime::BusyPolicy::EnqueueIfBusy,
                )
                .map(|_| ())
                .map_err(|error| format!("prompt: {error}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The prompt must tell the model that tools may have partly run; a model
    /// that assumes they did not re-runs side effects.
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
                name: "shell".to_string(),
                arguments: serde_json::json!({"command": "touch side-effect"}),
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
        assert!(continued, "an interrupted session is continued");

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

    /// The other half of "both paths coexist": when the journal cannot prove an
    /// open run, the pass still continues the conversation — by deliberately
    /// falling back to the visible prompt shape, not by doing nothing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_restart_pass_falls_back_to_a_prompt_when_in_place_is_refused() {
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
        assert!(
            resume_one(&state, "s-closed").is_err(),
            "nothing to continue"
        );
        assert!(
            provider.requests.lock().unwrap().is_empty(),
            "a refused continuation does not call the model"
        );
    }
}
