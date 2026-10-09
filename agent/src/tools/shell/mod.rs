//! Single-command validation, attempt capture and structured result projection.
pub(crate) mod execution;
mod guidance;
mod params;
mod projection;

use super::*;
use future_rpc::shell_result::{ShellAttempt, ShellResult, ShellStatus};
pub(super) use guidance::guidelines;
pub(crate) use params::validate;
use params::ShellParams;
use std::cell::RefCell;
use std::time::{Duration, Instant};

#[derive(Default)]
struct ExecutionFacts {
    attempts: Vec<ShellAttempt>,
    approval: Option<String>,
    note: Option<String>,
}

tokio::task_local! {
    static RESULT: RefCell<Option<ShellResult>>;
    static FACTS: RefCell<ExecutionFacts>;
    static PROCESS: RefCell<Option<ShellAttempt>>;
    static STARTED: RefCell<bool>;
    static TRUNCATED: RefCell<bool>;
    static OUTPUT_LIMIT: usize;
    static DEADLINE: Instant;
    static GATE_OUTCOME: RefCell<Option<String>>;
}

/// The side channel is task-local: concurrent calls cannot mix facts. Never
/// deserialize stdout to obtain status. Non-shell tools retain their API.
pub(crate) async fn capture<F: Future>(future: F) -> (F::Output, Option<ShellResult>) {
    RESULT
        .scope(
            RefCell::new(None),
            GATE_OUTCOME.scope(RefCell::new(None), async {
                let output = future.await;
                let result = RESULT.with(|slot| slot.borrow_mut().take());
                (output, result)
            }),
        )
        .await
}

fn limit() -> usize {
    OUTPUT_LIMIT.try_with(|n| *n).unwrap_or(500_000)
}
fn bounded(text: &str, max: usize) -> String {
    text[text.ceil_char_boundary(text.len().saturating_sub(max))..].to_owned()
}
fn duration(timeout_secs: u64) -> Duration {
    let requested = Duration::from_secs(timeout_secs.max(1));
    DEADLINE
        .try_with(|deadline| {
            deadline
                .saturating_duration_since(Instant::now())
                .min(requested)
        })
        .unwrap_or(requested)
}
fn cancelled() -> bool {
    TOOL_SCOPE
        .try_with(|scope| scope.interrupt_flag.load(Ordering::Relaxed))
        .unwrap_or(false)
}
fn mark_started() {
    let _ = STARTED.try_with(|slot| *slot.borrow_mut() = true);
}

fn record_process(status: ShellStatus, code: Option<i32>, output: &str) {
    let _ = PROCESS.try_with(|slot| {
        *slot.borrow_mut() = Some(ShellAttempt {
            status,
            exit_code: code,
            output: bounded(output, limit()),
            output_truncated: output.len() > limit()
                || TRUNCATED.try_with(|flag| *flag.borrow()).unwrap_or(false),
            duration_ms: 0,
            escalated: false,
        })
    });
}
fn record_approval(approved: bool, note: &str) {
    let _ = FACTS.try_with(|facts| {
        let mut facts = facts.borrow_mut();
        facts.approval = Some(if approved { "approved" } else { "denied" }.into());
        facts.note = Some(bounded(note, 2048));
    });
}
fn latest_attempt() -> Option<ShellAttempt> {
    FACTS
        .try_with(|facts| facts.borrow().attempts.last().cloned())
        .ok()
        .flatten()
}

pub(crate) fn record_gate_outcome(status: &str) {
    let _ = GATE_OUTCOME.try_with(|slot| *slot.borrow_mut() = Some(status.into()));
}
fn record_approval_end(status: &str, note: &str) {
    let _ = FACTS.try_with(|facts| {
        let mut facts = facts.borrow_mut();
        facts.approval = Some(status.into());
        facts.note = Some(bounded(note, 2048));
    });
}
fn empty_result(command: String) -> Result<ShellResult> {
    let cwd = active_workspace()?.to_string_lossy().into_owned();
    if serde_json::to_string(&cwd)?.len() > 262_144 {
        return Err(anyhow!(
            "Workspace result metadata exceeds the call result budget"
        ));
    }
    Ok(ShellResult {
        command,
        cwd,
        duration_ms: 0,
        status: ShellStatus::NotStarted,
        exit_code: None,
        is_soft_fail: false,
        is_error: true,
        attempts: vec![],
        approval: None,
        note: None,
    })
}

/// Preserve approval provenance when the normal pre-execution hook stops a call.
pub(crate) fn record_gated(args: &serde_json::Value, note: &str) {
    let command = args
        .get("command")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if let Ok(mut result) = empty_result(command.into()) {
        result.approval = GATE_OUTCOME
            .try_with(|slot| slot.borrow_mut().take())
            .ok()
            .flatten();
        if approval_cancelled(&result) {
            result.status = ShellStatus::Cancelled;
        }
        result.note = Some(bounded(note, 2048));
        result.update_verdict();
        let _ = RESULT.try_with(|slot| *slot.borrow_mut() = Some(result));
    }
}

pub(crate) async fn execute(args: serde_json::Value) -> Result<String> {
    let params = ShellParams::parse(args)?;
    let timeout = params.timeout.unwrap_or(120);
    let start = Instant::now();
    let mut result = empty_result(params.command.clone())?;
    result.approval = GATE_OUTCOME
        .try_with(|slot| slot.borrow_mut().take())
        .ok()
        .flatten();
    let run = async {
        let output = run_command(&params, timeout).await;
        let facts = FACTS.with(|slot| std::mem::take(&mut *slot.borrow_mut()));
        (output, facts)
    };
    // Reserve output for both the original process and its possible retry.
    let (output, facts) = DEADLINE
        .scope(
            start + Duration::from_secs(timeout),
            OUTPUT_LIMIT.scope(
                250_000,
                FACTS.scope(RefCell::new(ExecutionFacts::default()), run),
            ),
        )
        .await;
    result.attempts = facts.attempts;
    result.approval = facts.approval.or(result.approval);
    result.note = facts.note;
    if let Some(attempt) = result.attempts.last() {
        result.status = attempt.status;
        result.exit_code = attempt.exit_code;
        result.is_soft_fail = attempt.status == ShellStatus::Exited
            && attempt.exit_code == Some(1)
            && is_soft_fail_command(&params.command);
    }
    if approval_cancelled(&result) {
        result.status = ShellStatus::Cancelled;
    }
    if let Err(error) = &output {
        result.note = Some(bounded(&error.to_string(), 2048));
    }
    result.duration_ms = start.elapsed().as_millis() as u64;
    result.update_verdict();
    // JSON escaping can expand output beyond its raw-byte budget.
    while serde_json::to_vec(&result)?.len() > 1_048_576 {
        for attempt in &mut result.attempts {
            attempt.output = bounded(&attempt.output, attempt.output.len() / 2);
            attempt.output_truncated = true;
        }
    }
    let text = projection::text(&result);
    let _ = RESULT.try_with(|slot| *slot.borrow_mut() = Some(result));
    match output {
        Ok(_) => Ok(text),
        Err(_) => Err(anyhow!(text)),
    }
}

fn approval_cancelled(result: &ShellResult) -> bool {
    matches!(
        result.approval.as_deref(),
        Some("cancelled" | "context_invalidated")
    )
}

async fn run_command(params: &ShellParams, timeout: u64) -> Result<String> {
    let capability = if let Some(permissions) = &params.additional_permissions {
        let sandbox = TOOL_SCOPE
            .try_with(|scope| scope.sandbox.clone())
            .unwrap_or_default();
        let prepared =
            crate::sandbox::windows_request::prepare(&sandbox, &params.command, permissions)?;
        if prepared.needs_approval() {
            Some(consume_windows_capability(&prepared).ok_or_else(|| {
                anyhow!("additional write permission is missing an exact approval receipt")
            })?)
        } else {
            None
        }
    } else {
        None
    };
    execution::run_shell_with_capability(
        &params.command,
        timeout,
        params.escalated.unwrap_or(false),
        params.justification.as_deref().unwrap_or(""),
        capability.as_ref(),
    )
    .await
}

fn append_output(buf: &mut Vec<u8>, chunk: &[u8]) {
    let max = limit();
    if buf.len() + chunk.len() > max {
        let _ = TRUNCATED.try_with(|flag| *flag.borrow_mut() = true);
        let drop = (buf.len() + chunk.len()).saturating_sub(max).min(buf.len());
        buf.drain(..drop);
    }
    buf.extend_from_slice(&chunk[chunk.len().saturating_sub(max)..]);
}

#[cfg(test)]
mod tests;
