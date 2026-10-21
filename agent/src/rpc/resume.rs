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
//! activity is older than [`MAX_AGE`]; it never resumes more than
//! [`MAX_PER_START`] conversations in one start, so a machine coming up after an
//! outage does not fire one model request per conversation at once.
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

/// How many conversations one start may continue. A restart after an outage
/// must not turn into N simultaneous model requests.
pub const MAX_PER_START: usize = 3;

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
    let mut resumed = 0usize;
    for summary in summaries {
        if resumed >= MAX_PER_START {
            break;
        }
        if !summary.parent_session_id.trim().is_empty() {
            // A sub-session continues when the work that started it does; on its
            // own it would answer a question nobody re-asked.
            continue;
        }
        let age = chrono::Local::now().signed_duration_since(summary.updated_at);
        if age.num_seconds() > MAX_AGE.as_secs() as i64 {
            continue;
        }
        match resume_one(&state, &summary.id) {
            Ok(true) => resumed += 1,
            Ok(false) => {}
            Err(error) => eprintln!("FutureOS resume: {} not continued: {error}", summary.id),
        }
    }
    // Say what the pass did even when it did nothing: an enabled switch with no
    // continuation is a question ("why not?"), and silence cannot answer it.
    eprintln!(
        "FutureOS resume: {considered} conversation(s) checked, {resumed} continued \
         (limit {MAX_PER_START} per start, interruptions older than {} days ignored)",
        MAX_AGE.as_secs() / 86_400
    );
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
    session
        .enqueue_prompt_with_model_context(
            PromptText::new(&build_prompt(), ""),
            &[],
            &[],
            None,
            &format!("resume_{run_id}"),
            crate::runtime::BusyPolicy::EnqueueIfBusy,
        )
        .map_err(|error| format!("prompt: {error}"))?;
    Ok(true)
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
}
