//! `reset_session_history` — in-place history reset.
//!
//! Synchronous on purpose, unlike `compact`: there is no model call and no
//! minutes-long work, and the caller needs the numbers (and the backup path)
//! back in the same round trip to decide. What it shares with compaction is what
//! matters: the Agent owns the history, the rewrite happens inside the session's
//! storage, and a session with an active run (or a compaction in flight) is
//! refused instead of having its turn interrupted.

use std::sync::Arc;

use crate::rpc::{AppState, HistoryResetCommand, RpcCommand, RpcResponse, ServerSession};
use crate::session::reset::{DropKind, ResetSelectors, HISTORY_CHANGED};

fn fail(id: &str, code: &str, error: &str, data: serde_json::Value) -> String {
    RpcResponse::build_fail_code(id, "reset_session_history", code, error, data)
}

/// Test-only hook fired in the window **between** the store accepting the reset
/// and the in-memory working history being reloaded from it, for one session id.
///
/// It exists for the same reason `reset.rs`'s `RESET_RACE_HOOK` does: this window
/// is real — the rewrite has committed, the reload has not run — and something
/// else can touch the store in between (the store is a file; the reset's own
/// digest/rewrite pair exists precisely because a concurrent writer can slip into
/// that gap). Making it deterministic lets the `reload_failed` arm be tested
/// rather than argued about, because no *input* can reach it: the planner reads
/// through the same path the reload does, so damaging the store before the reset
/// fails the plan instead.
#[cfg(test)]
#[allow(clippy::type_complexity)]
pub(crate) static RELOAD_FAILURE_HOOK: parking_lot::Mutex<Option<(String, Box<dyn Fn() + Send>)>> =
    parking_lot::Mutex::new(None);

#[cfg(test)]
pub(crate) fn set_reload_failure_hook(session: &str, hook: Box<dyn Fn() + Send>) {
    *RELOAD_FAILURE_HOOK.lock() = Some((session.to_string(), hook));
}

/// Selectors as the wire carries them, validated into a plan the store can apply.
fn selectors_from(reset: &HistoryResetCommand) -> Result<ResetSelectors, String> {
    let bound = |value: Option<i64>, flag: &str| -> Result<Option<u64>, String> {
        match value {
            None => Ok(None),
            Some(value) if value >= 0 => Ok(Some(value as u64)),
            Some(value) => Err(format!("{flag} must not be negative (got {value})")),
        }
    };
    let mut drop = Vec::with_capacity(reset.drop.len());
    for name in &reset.drop {
        drop.push(DropKind::parse(name).map_err(|error| error.to_string())?);
    }
    let selectors = ResetSelectors {
        all: reset.all.unwrap_or(false),
        keep_first: bound(reset.keep_first, "--keep-first")?,
        keep_last: bound(reset.keep_last, "--keep-last")?,
        drop,
    };
    selectors.validate().map_err(|error| error.to_string())?;
    Ok(selectors)
}

pub(crate) fn handle_reset_session_history(
    _state: &AppState,
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    use std::sync::atomic::Ordering;

    let reset = match cmd.reset_history.as_ref() {
        Some(reset) => reset,
        None => {
            return fail(
                id,
                "invalid_selectors",
                "history reset selectors are required",
                serde_json::json!({}),
            )
        }
    };
    let selectors = match selectors_from(reset) {
        Ok(selectors) => selectors,
        Err(error) => {
            return fail(
                id,
                "invalid_selectors",
                &error,
                serde_json::json!({"known_drop_kinds": crate::session::reset::DROP_KIND_NAMES}),
            )
        }
    };
    let apply = reset.apply;
    let (session_manager, session_id) = {
        let sess = session.read();
        (sess.session_manager.clone(), sess.session_id.clone())
    };
    {
        let sess = session.read();
        // Same admission rule as manual compaction: never rewrite the history a
        // run is reading, and never race a compaction that is projecting it.
        if sess.runtime.snapshot().is_some() || sess.is_streaming.load(Ordering::Acquire) {
            return fail(
                id,
                "session_busy",
                "finish or stop the active run before resetting session history",
                serde_json::json!({"busy_reason": "run", "retryable": true}),
            );
        }
        if sess.compaction_in_progress.load(Ordering::Acquire) {
            return fail(
                id,
                "session_busy",
                "session context compaction is in progress",
                serde_json::json!({"busy_reason": "compaction", "retryable": true}),
            );
        }
    }

    let outcome = match session_manager.reset_history(&session_id, &selectors, apply) {
        Ok(outcome) => outcome,
        Err(error) => {
            let message = format!("{error:#}");
            if message.contains(HISTORY_CHANGED) {
                return fail(
                    id,
                    "history_changed",
                    "session history changed while the reset was prepared; run it again",
                    serde_json::json!({"retryable": true}),
                );
            }
            if message.contains("nothing would be left") {
                return fail(
                    id,
                    "nothing_retained",
                    &message,
                    serde_json::json!({"retryable": false}),
                );
            }
            return fail(
                id,
                "reset_failed",
                &message,
                serde_json::json!({"retryable": true}),
            );
        }
    };

    if outcome.applied {
        #[cfg(test)]
        {
            // Id-gated so unrelated resets never consume the hook.
            let mut slot = RELOAD_FAILURE_HOOK.lock();
            if matches!(slot.as_ref(), Some((sid, _)) if sid == &session_id) {
                if let Some((_, hook)) = slot.take() {
                    hook();
                }
            }
        }
        // The in-memory working history is what the next request sends (and what
        // a terminal rewrite would persist), so it must become the rewritten
        // history in the same breath as the store: re-read rather than reuse,
        // because the store ran the same selection and load-time repair.
        let mut sess = session.write();
        sess.history_loaded = false;
        if let Err(error) = sess.ensure_history_loaded() {
            return fail(
                id,
                "reload_failed",
                &format!("history was reset on disk but could not be reloaded: {error:#}"),
                serde_json::json!({"retryable": true,
                    "backup": outcome.backup.as_ref().map(|path| path.display().to_string())}),
            );
        }
    }

    let plan = &outcome.plan;
    let mut payload = serde_json::json!({
        "sessionId": session_id,
        "applied": outcome.applied,
        "selectors": selectors.to_json(),
        "entriesBefore": plan.entries_before,
        "entriesAfter": plan.entries_after,
        "turnsBefore": plan.turns_before,
        "turnsKept": plan.turns_kept,
        "estimatedTokensBefore": plan.estimated_tokens_before,
        "estimatedTokensAfter": plan.estimated_tokens_after,
        "dropEffects": plan.drop_effects,
        "firstKept": plan.first_kept,
        "lastKept": plan.last_kept,
        "notes": plan.notes,
        "backup": outcome.backup.as_ref().map(|path| path.display().to_string()),
    });
    if !outcome.applied {
        payload["backup"] = serde_json::Value::Null;
        payload["dryRun"] = serde_json::Value::Bool(true);
    }
    RpcResponse::ok(id, "reset_session_history", payload)
}
