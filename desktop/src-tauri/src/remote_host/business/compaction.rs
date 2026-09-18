//! Manual context compaction requested from a paired phone.
//!
//! The phone owns the trigger (a `/` slash action) but not the operation: the
//! Desktop resolves the same session-scoped `compact` RPC the GUI uses, so both
//! clients accept the operation identically and correlate the terminal
//! `compaction_*` event by the returned operation id.
use crate::remote::protocol::IncomingCmd;
use crate::remote::services::ReplySink;

use super::reply;

pub(super) async fn execute(cmd: &IncomingCmd, sink: &dyn ReplySink) {
    let session_id = cmd.session_id.trim();
    if session_id.is_empty() {
        reply(
            sink,
            false,
            serde_json::Value::Null,
            Some("missing session"),
        )
        .await;
        return;
    }
    match crate::agent_bridge::compact_agent_session(session_id.to_string()).await {
        Ok(acknowledgement) => reply(sink, true, acknowledgement, None).await,
        Err(error) => {
            reply(
                sink,
                false,
                serde_json::Value::Null,
                Some(&error.to_string()),
            )
            .await
        }
    }
}
