//! Sessionless, explicitly invoked developer maintenance; never opens a runtime
//! session just to clean its stored data.
use super::{AppState, RpcCommand, RpcResponse};
use serde_json::json;
use std::sync::{atomic::Ordering, Arc};

static STEP: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
struct Admission(Arc<std::sync::atomic::AtomicBool>);
impl Drop for Admission {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub(crate) fn handle(state: &AppState, cmd: &RpcCommand, id: &str) -> String {
    let Some(_step) = STEP.try_lock() else {
        return if cmd.custom_instructions == "session" {
            RpcResponse::ok(
                id,
                "organize_database",
                json!({"status":"skipped","reason":"busy","hasMore":false}),
            )
        } else {
            RpcResponse::build_fail(
                id,
                "organize_database",
                "another database maintenance step is in progress",
            )
        };
    };
    let result = (|| -> anyhow::Result<serde_json::Value> {
        match cmd.custom_instructions.as_str() {
            "plan" => state.session_manager.database_organization_plan(),
            "session" => {
                anyhow::ensure!(!cmd.session_id.is_empty(), "session identity is required");
                let live = state.sessions.read().get(&cmd.session_id).cloned();
                let _admission = if let Some(live) = live {
                    let sess = live.write();
                    if sess.runtime.snapshot().is_some()
                        || sess.is_streaming.load(Ordering::Acquire)
                        || !sess.scheduler.queued().is_empty()
                        || sess.compaction_in_progress.load(Ordering::Acquire)
                        || sess.history_reset_in_progress.load(Ordering::Acquire)
                        || sess.deleting
                    {
                        return Ok(json!({"status":"skipped","reason":"busy","hasMore":false}));
                    }
                    sess.history_reset_in_progress
                        .store(true, Ordering::Release);
                    let guard = Admission(sess.history_reset_in_progress.clone());
                    let (persistence, broadcaster) =
                        (sess.persistence.clone(), sess.broadcaster.clone());
                    drop(sess);
                    persistence.barrier()?;
                    broadcaster.flush_journal()?;
                    Some(guard)
                } else {
                    None
                };
                state
                    .session_manager
                    .organize_database_session(&cmd.session_id)
            }
            "finish" => state.session_manager.finish_database_organization(),
            _ => anyhow::bail!("unknown database organization phase"),
        }
    })();
    match result {
        Ok(data) => RpcResponse::ok(id, "organize_database", data),
        Err(error) if cmd.custom_instructions == "session" => RpcResponse::ok(
            id,
            "organize_database",
            json!({"status":"failed","reason":format!("{error:#}"),"hasMore":false}),
        ),
        Err(error) => RpcResponse::build_fail(id, "organize_database", &format!("{error:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::commands::test_support::{make_app_state, make_cmd, parse_response};

    #[test]
    fn sessionless_steps_skip_busy_runtime_report_failures_and_release_admission() {
        let state = make_app_state();
        let live = state.sessions.read()["default"].clone();
        live.read().is_streaming.store(true, Ordering::Release);
        let mut cmd = make_cmd("organize_database");
        cmd.custom_instructions = "session".into();
        cmd.session_id = "default".into();
        let response = parse_response(&handle(&state, &cmd, "probe"));
        assert_eq!(response["data"]["status"], "skipped");
        assert!(!live
            .read()
            .history_reset_in_progress
            .load(Ordering::Acquire));
        live.read().is_streaming.store(false, Ordering::Release);
        cmd.session_id = "missing".into();
        let response = parse_response(&handle(&state, &cmd, "probe"));
        assert_eq!(response["success"], true);
        assert_eq!(response["data"]["status"], "failed");
        assert!(!state.sessions.read().contains_key("missing"));
        cmd.session_id.clear();
        cmd.custom_instructions = "plan".into();
        let response = parse_response(&handle(&state, &cmd, "probe"));
        assert_eq!(response["success"], true);
        assert!(response["data"]["sessionIds"].is_array());
        assert!(response["data"]["storage"]["totalBytes"].is_number());
    }
}
