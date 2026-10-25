//! `acknowledge_recovery` — the one explicit, auditable release of a recovery
//! fence.
//!
//! A restart that cut a tool call off leaves its effect unknown, and
//! [`RecoverySafety`](super::super::recovery_safety) refuses every mutating tool
//! for the rest of that session. That fence is deliberate — nobody was present
//! when the side effect may have happened, so replaying it silently is exactly
//! what must not happen. What was missing is a way out that a person authorizes
//! and that leaves a record: without one, "the agent restarted while a shell was
//! running" permanently ends the conversation.
//!
//! This handler appends one `custom` journal entry naming precisely the calls
//! the fence is holding, plus who released them and why. `RecoverySafety::load`
//! reads it back and lowers exactly those calls from "session-wide fence" to
//! "questioned once, then allowed" — the same tier a user-aborted operation
//! already has. Nothing is written as verified, and no call is replayed here.

use std::sync::Arc;

use super::super::recovery_safety::RecoverySafety;
use crate::rpc::{RpcCommand, RpcResponse, ServerSession};
use crate::session::{SessionEntry, ENTRY_NAME_RECOVERY_ACKNOWLEDGED};
use parking_lot::RwLock;

pub(crate) fn handle_acknowledge_recovery(
    session: &Arc<RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    let session = session.read();
    let safety = match RecoverySafety::load(
        &session.session_manager,
        &session.session_id,
        &session.cwd,
        false,
    ) {
        Ok(safety) => safety,
        Err(error) => {
            return RpcResponse::build_fail(
                id,
                "acknowledge_recovery",
                &format!("unable to read this session's recovery state: {error:#}"),
            )
        }
    };
    let blocked = safety.blocked_calls();
    if blocked.is_empty() {
        // Idempotent, and honest about it: there is nothing left to release
        // because the fence is already down (or never existed).
        return RpcResponse::ok(
            id,
            "acknowledge_recovery",
            serde_json::json!({
                "released": [],
                "already_released": true,
                "replayed": false,
            }),
        );
    }
    let calls = blocked
        .iter()
        .map(|call| {
            serde_json::json!({
                "run_id": call.run_id.as_str(),
                "call_id": call.call_id.as_str(),
                "tool": call.tool.as_str(),
            })
        })
        .collect::<Vec<_>>();
    let by = if cmd.created_by.is_empty() {
        "cli"
    } else {
        cmd.created_by.as_str()
    };
    let entry = SessionEntry::recovery_acknowledged(
        ENTRY_NAME_RECOVERY_ACKNOWLEDGED,
        serde_json::Value::Array(calls.clone()),
        by,
        &cmd.message,
    );
    let entry_id = entry.id.clone();
    if let Err(error) = session
        .session_manager
        .append_entries_synced(&session.session_id, &[entry])
    {
        return RpcResponse::build_fail(
            id,
            "acknowledge_recovery",
            &format!("unable to record the acknowledgement: {error:#}"),
        );
    }
    // Deliberately no `mark_verified`: the released calls stay unproven. Each
    // is questioned once on its next attempt, and the rest of the session's
    // tools work again from the next run.
    RpcResponse::ok(
        id,
        "acknowledge_recovery",
        serde_json::json!({
            "released": calls,
            "already_released": false,
            "replayed": false,
            "entry_id": entry_id,
        }),
    )
}
