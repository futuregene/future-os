//! Approval decisions: notify the agent of a pending decision, persist it, and
//! resume the owning run. Stale requests the agent already dropped are
//! reconciled by cancelling locally.

use super::client::{
    add_session_rule_command, approval_decision_command, connect_agent, get_state_command,
    RpcResponseExt,
};
use crate::store;

/// Inject a same-run "allow in this workspace/chat" rule into the thread's live
/// agent session (in tandem with writing the rule file). Best-effort: a missing
/// session or offline agent just means the rule only applies from the next
/// prompt (when the file is re-read).
pub async fn inject_session_rule(
    thread_id: &str,
    path: &str,
    access: &str,
) -> Result<(), crate::AppError> {
    let Some(thread) = store::get_thread(thread_id)? else {
        return Ok(());
    };
    let session_id = thread.agent_session_id.unwrap_or(thread.id);
    let mut client = connect_agent().await?;
    client
        .execute_command(add_session_rule_command(
            path.to_string(),
            access.to_string(),
            session_id,
        ))
        .await
        .map_err(|error| format!("Unable to inject session rule: {error}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the session rule.")?;
    Ok(())
}

async fn notify_agent_approval_decision(
    approval: &store::ApprovalRequestRecord,
    input: &store::DecideApprovalRequestInput,
) -> Result<(), crate::AppError> {
    let thread = store::get_thread(&approval.thread_id)?
        .ok_or_else(|| "Approval thread could not be loaded.".to_string())?;
    let mut client = connect_agent().await?;
    client
        .execute_command(approval_decision_command(
            approval.id.clone(),
            input.status.clone(),
            input.decision_note.clone().unwrap_or_default(),
            thread.agent_session_id.unwrap_or(thread.id),
        ))
        .await
        .map_err(|error| format!("Unable to send approval decision to Future Agent: {error}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the approval decision.")?;
    Ok(())
}

/// Record an approval decision: notify the agent while the request is still
/// pending, persist the decision, and resume the owning run. A request the
/// agent already dropped is reconciled by cancelling it locally.
pub async fn decide_approval(
    input: store::DecideApprovalRequestInput,
) -> Result<store::ApprovalRequestRecord, crate::AppError> {
    let current = store::get_approval_request(&input.approval_request_id)?
        .ok_or_else(|| "Approval request could not be loaded.".to_string())?;
    if current.status == "pending" {
        if let Err(error) = notify_agent_approval_decision(&current, &input).await {
            if is_stale_approval_error(&error.to_string()) {
                return store::decide_approval_request(store::DecideApprovalRequestInput {
                    approval_request_id: input.approval_request_id,
                    status: "cancelled".to_string(),
                    decision_note: Some("Cancelled because the approval request is no longer active in Future Agent.".to_string()),
                });
            }
            return Err(error);
        }
    }
    let updated = store::decide_approval_request(input)?;
    if let Some(run_id) = &updated.run_id {
        // Compare-and-set: resume the owning run only while it is non-terminal,
        // so a concurrent `abort_run` that already set `cancelled` is never
        // overwritten back to `running` by this late read-then-write (B-13).
        let _ = store::update_run_status_if_active(store::UpdateRunStatusInput {
            run_id: run_id.clone(),
            status: "running".to_string(),
            error_message: None,
            error_type: None,
        });
    }
    Ok(updated)
}

fn is_stale_approval_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    normalized.contains("approval request") && normalized.contains("not pending")
}

/// Reconcile locally-pending approvals against the Agent's authoritative
/// pending set (`get_state.pendingApprovals`).
///
/// Called once after the Agent becomes reachable at startup — after
/// `reconcile_interrupted_runs`, so parked runs are reanimated first — and on
/// every watchdog tick. Startup convergence deliberately leaves pending
/// approvals alone; only this pass, holding the Agent's answer, may settle
/// them:
///
/// - A request still pending Agent-side keeps its card. Its owning run was
///   reanimated back to `running` (or never left it), so CAS it to
///   `waiting_approval` to reflect the wait.
/// - A request the Agent no longer holds was settled while the GUI was down
///   (decided elsewhere, aborted, or lost to an Agent restart) — cancel the
///   stale local row.
/// - A request pending Agent-side with no local row (the GUI crashed between
///   broadcast and persistence) is rebuilt from the payload the Agent serves.
///
/// Transport failures are skipped silently — the next tick retries. Acting
/// only on a definitive Agent answer is what makes this safe: a flaky
/// connection can never wrongly cancel a request the Agent is parked on.
pub async fn reconcile_pending_approvals() {
    let Ok(pending) = store::list_pending_approval_requests() else {
        return;
    };

    // Group by thread so each session is queried exactly once.
    let mut by_thread: std::collections::HashMap<String, Vec<store::ApprovalRequestRecord>> =
        std::collections::HashMap::new();
    for approval in pending {
        by_thread
            .entry(approval.thread_id.clone())
            .or_default()
            .push(approval);
    }

    for (thread_id, approvals) in by_thread {
        let Ok(Some(thread)) = store::get_thread(&thread_id) else {
            continue;
        };
        let Some(session_id) = thread
            .agent_session_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
        else {
            continue;
        };
        let Ok(mut client) = connect_agent().await else {
            continue;
        };
        let response = match client
            .execute_command(get_state_command(session_id.clone()))
            .await
        {
            Ok(response) => response.into_inner(),
            Err(_) => continue, // Agent unreachable — retry on the next tick.
        };
        if !response.success {
            // Session unresolvable — leave the rows pending; the run watchdog
            // settles the owning runs, and a later tick retries the approval.
            continue;
        }
        let state: serde_json::Value = future_rpc::decode::response_data(&response);
        let agent_payloads: Vec<&serde_json::Value> = state
            .get("pendingApprovals")
            .and_then(|value| value.as_array())
            .map(|array| array.iter().collect())
            .unwrap_or_default();
        let agent_ids: std::collections::HashSet<&str> = agent_payloads
            .iter()
            .filter_map(|payload| payload["approval_request_id"].as_str())
            .collect();

        for approval in &approvals {
            if agent_ids.contains(approval.id.as_str()) {
                if let Some(run_id) = &approval.run_id {
                    let _ = store::update_run_status_if_active(store::UpdateRunStatusInput {
                        run_id: run_id.clone(),
                        status: "waiting_approval".to_string(),
                        error_message: None,
                        error_type: None,
                    });
                }
            } else {
                let _ = store::decide_approval_request(store::DecideApprovalRequestInput {
                    approval_request_id: approval.id.clone(),
                    status: "cancelled".to_string(),
                    decision_note: Some(
                        "Settled or cancelled while FutureOS was not running.".to_string(),
                    ),
                });
            }
        }

        // Self-heal: requests the Agent still holds but the GUI never
        // persisted. The approval's thread is derived from its run, so only
        // heal when the parked run has a local row (sessions without local
        // runs are covered by session import, not this path).
        let local_ids: std::collections::HashSet<&str> = approvals
            .iter()
            .map(|approval| approval.id.as_str())
            .collect();
        let active_run_id = state
            .get("activeRun")
            .and_then(|run| run.get("runId"))
            .and_then(|id| id.as_str())
            .map(str::to_string);
        for payload in agent_payloads {
            let Some(id) = payload["approval_request_id"].as_str() else {
                continue;
            };
            if local_ids.contains(id) {
                continue;
            }
            heal_pending_approval_from_agent(active_run_id.as_deref(), id, payload);
        }
    }
}

/// Rebuild a locally-missing approval row from the payload the Agent serves
/// for a still-parked request. `approval_request_id` is pre-validated by the
/// caller (it only calls here for payloads carrying one). Mirrors the field
/// mapping in `persist::persist_approval_request` — the payload is the exact
/// `approval_request` event data the Agent broadcast.
fn heal_pending_approval_from_agent(
    active_run_id: Option<&str>,
    approval_request_id: &str,
    payload: &serde_json::Value,
) {
    let Some(run_id) = active_run_id else {
        return;
    };
    let Ok(Some(_run)) = store::get_run(run_id) else {
        return;
    };
    let approval_request_id = approval_request_id.to_string();
    let tool_name = payload
        .get("tool_name")
        .and_then(|value| value.as_str())
        .unwrap_or("tool");
    let action_value = payload.get("action");

    if let Err(error) = store::ensure_approval_request(store::EnsureApprovalRequestInput {
        approval_request_id: Some(approval_request_id.clone()),
        run_id: run_id.to_string(),
        tool_call_id: payload
            .get("tool_id")
            .and_then(|value| value.as_str())
            .filter(|id| !id.is_empty())
            .map(str::to_string),
        kind: payload
            .get("kind")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| "tool".to_string()),
        title: payload
            .get("title")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("Approve `{tool_name}`")),
        summary: payload
            .get("summary")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        risk_level: payload
            .get("risk_level")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        requested_action: payload.get("requested_action").map(compact_json),
        action_category: action_value
            .and_then(|action| action.get("category"))
            .and_then(|category| category.as_str())
            .map(str::to_string),
        action_payload: action_value.map(compact_json),
        sandbox_boundary: payload.get("sandbox_boundary").map(compact_json),
        save_suggestion: payload
            .get("save_suggestion")
            .filter(|value| value.is_object())
            .map(compact_json),
        reviewer: payload
            .get("reviewer")
            .and_then(|value| value.as_str())
            .map(str::to_string),
    }) {
        eprintln!("FutureOS approval heal failed for {approval_request_id}: {error}");
        return;
    }
    // Same CAS as the live persist path: the parked run awaits this decision.
    let _ = store::update_run_status_if_active(store::UpdateRunStatusInput {
        run_id: run_id.to_string(),
        status: "waiting_approval".to_string(),
        error_message: None,
        error_type: None,
    });
    eprintln!("FutureOS rebuilt missing approval card {approval_request_id} from agent state");
}

fn compact_json(value: &serde_json::Value) -> String {
    // serde_json::Value serialization is infallible (no custom Serialize impls).
    serde_json::to_string(value).expect("Value serialization is infallible")
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{
        mock_agent, seed_approval, seed_run, seed_thread, seed_workspace, Reply, TestHome,
    };
    use super::*;

    fn decide_input(id: &str, status: &str) -> store::DecideApprovalRequestInput {
        store::DecideApprovalRequestInput {
            approval_request_id: id.to_string(),
            status: status.to_string(),
            decision_note: Some("looks fine".to_string()),
        }
    }

    #[tokio::test]
    async fn inject_session_rule_targets_the_thread_session() {
        let home = TestHome::new("ap-inject");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");

        // Unknown thread: a no-op success.
        inject_session_rule("no-such-thread", "/tmp/**", "allow")
            .await
            .expect("no thread");
        assert!(mock.requests_of("add_session_rule").is_empty());

        // Thread with an agent session.
        let thread = seed_thread(&workspace.id, Some("sess-1"));
        mock.push("add_session_rule", Reply::Data("{}".to_string()));
        inject_session_rule(&thread.id, "/tmp/**", "allow")
            .await
            .expect("inject");
        let request = &mock.requests_of("add_session_rule")[0];
        assert_eq!(request.session_id, "sess-1");
        assert_eq!(request.message, "/tmp/**");
        assert_eq!(request.mode, "allow");

        // Thread without a session: the thread id doubles as the session id.
        let no_session = seed_thread(&workspace.id, None);
        mock.push("add_session_rule", Reply::Data("{}".to_string()));
        inject_session_rule(&no_session.id, "/tmp/**", "allow")
            .await
            .expect("inject");
        assert_eq!(
            mock.requests_of("add_session_rule")[1].session_id,
            no_session.id
        );

        // Transport + app-level failures surface.
        mock.push(
            "add_session_rule",
            Reply::Status(tonic::Code::Internal, "boom"),
        );
        let error = inject_session_rule(&thread.id, "/tmp/**", "allow")
            .await
            .expect_err("transport");
        assert!(
            error.to_string().contains("Unable to inject session rule"),
            "{error}"
        );

        mock.push("add_session_rule", Reply::Reject("bad rule".to_string()));
        let error = inject_session_rule(&thread.id, "/tmp/**", "allow")
            .await
            .expect_err("reject");
        assert_eq!(error.to_string(), "bad rule");
    }

    #[tokio::test]
    async fn decide_approval_notifies_persists_and_resumes_the_run() {
        let home = TestHome::new("ap-decide");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));
        let run = seed_run(&thread.id);
        let approval = seed_approval("appr-1", &run.id);

        mock.push("approval_decision", Reply::Data("{}".to_string()));
        let updated = decide_approval(decide_input("appr-1", "approved"))
            .await
            .expect("decide");
        assert_eq!(updated.status, "approved");
        let request = &mock.requests_of("approval_decision")[0];
        assert_eq!(request.entry_id, "appr-1");
        assert_eq!(request.mode, "approved");
        assert_eq!(request.message, "looks fine");
        assert_eq!(request.session_id, "sess-1");
        assert_eq!(
            store::get_run(&run.id).expect("run").expect("some").status,
            "running",
            "deciding resumes the owning run"
        );
        drop(approval);
    }

    #[tokio::test]
    async fn decide_approval_skips_notify_when_not_pending() {
        let home = TestHome::new("ap-decide-nonpending");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));
        let run = seed_run(&thread.id);
        seed_approval("appr-2", &run.id);
        store::decide_approval_request(store::DecideApprovalRequestInput {
            approval_request_id: "appr-2".to_string(),
            status: "approved".to_string(),
            decision_note: None,
        })
        .expect("pre-decide");

        // Already decided → no agent round-trip, and the store keeps the
        // first decision (decide is not a re-decision).
        let updated = decide_approval(decide_input("appr-2", "cancelled"))
            .await
            .expect("decide");
        assert_eq!(updated.status, "approved");
        assert!(mock.requests_of("approval_decision").is_empty());
    }

    #[tokio::test]
    async fn decide_approval_cancels_a_stale_request_locally() {
        let home = TestHome::new("ap-decide-stale");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));
        let run = seed_run(&thread.id);
        seed_approval("appr-3", &run.id);

        mock.push(
            "approval_decision",
            Reply::Reject("Approval request appr-3 is not pending".to_string()),
        );
        let updated = decide_approval(decide_input("appr-3", "approved"))
            .await
            .expect("stale reconciles locally");
        assert_eq!(updated.status, "cancelled");
        assert!(
            updated
                .decision_note
                .as_deref()
                .unwrap_or_default()
                .contains("no longer active"),
            "note: {:?}",
            updated.decision_note
        );
    }

    #[tokio::test]
    async fn decide_approval_error_paths() {
        let home = TestHome::new("ap-decide-errors");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));
        let run = seed_run(&thread.id);
        seed_approval("appr-4", &run.id);

        // Unknown approval id.
        let error = decide_approval(decide_input("appr-missing", "approved"))
            .await
            .expect_err("missing");
        assert_eq!(error.to_string(), "Approval request could not be loaded.");

        // Non-stale notify failure propagates (rejection).
        mock.push("approval_decision", Reply::Reject("denied".to_string()));
        let error = decide_approval(decide_input("appr-4", "approved"))
            .await
            .expect_err("reject");
        assert_eq!(error.to_string(), "denied");

        // Transport failure propagates.
        mock.push(
            "approval_decision",
            Reply::Status(tonic::Code::Internal, "boom"),
        );
        let error = decide_approval(decide_input("appr-4", "approved"))
            .await
            .expect_err("transport");
        assert!(
            error
                .to_string()
                .contains("Unable to send approval decision to Future Agent"),
            "{error}"
        );
    }

    #[test]
    fn stale_approval_error_matching() {
        assert!(is_stale_approval_error(
            "Approval request abc is not pending"
        ));
        assert!(is_stale_approval_error(
            "APPROVAL REQUEST abc NOT PENDING anymore"
        ));
        assert!(!is_stale_approval_error("approval request missing"));
        assert!(!is_stale_approval_error("not pending"));
        assert!(!is_stale_approval_error("unrelated"));
    }

    #[tokio::test]
    async fn reconcile_settles_and_heals_against_agent_state() {
        let home = TestHome::new("ap-reconcile");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));

        // kept: still pending agent-side → run parked at waiting_approval.
        let run_kept = seed_run(&thread.id);
        seed_approval("appr-kept", &run_kept.id);
        // dropped: gone agent-side → cancelled locally.
        let run_dropped = seed_run(&thread.id);
        seed_approval("appr-dropped", &run_dropped.id);
        // heal target: the agent parks a request the GUI never persisted.
        let run_heal = seed_run(&thread.id);

        mock.push_data(
            "get_state",
            serde_json::json!({
                "activeRun": {"runId": run_heal.id},
                "pendingApprovals": [
                    {"approval_request_id": "appr-kept", "tool_name": "shell"},
                    {
                        "approval_request_id": "appr-healed",
                        "tool_id": "tc-1",
                        "tool_name": "write",
                        "kind": "tool",
                        "title": "Approve write",
                        "summary": "write a file",
                        "risk_level": "high",
                        "requested_action": {"command": "ls"},
                        "action": {"category": "fs"},
                        "sandbox_boundary": {"scope": "workspace"},
                        "save_suggestion": {"scope": "session"},
                        "reviewer": "user"
                    },
                    {"no_id": true}
                ]
            }),
        );
        reconcile_pending_approvals().await;

        let kept = store::get_approval_request("appr-kept")
            .expect("query")
            .expect("exists");
        assert_eq!(
            kept.status, "pending",
            "still pending agent-side keeps the card"
        );
        assert_eq!(
            store::get_run(&run_kept.id)
                .expect("run")
                .expect("some")
                .status,
            "waiting_approval"
        );

        let dropped = store::get_approval_request("appr-dropped")
            .expect("query")
            .expect("exists");
        assert_eq!(
            dropped.status, "cancelled",
            "gone agent-side cancels the local row"
        );
        assert!(
            dropped
                .decision_note
                .as_deref()
                .unwrap_or_default()
                .contains("not running"),
            "note: {:?}",
            dropped.decision_note
        );

        let healed = store::get_approval_request("appr-healed")
            .expect("query")
            .expect("healed row");
        assert_eq!(healed.status, "pending");
        assert_eq!(healed.run_id.as_deref(), Some(run_heal.id.as_str()));
        assert_eq!(healed.tool_call_id.as_deref(), Some("tc-1"));
        assert_eq!(healed.title, "Approve write");
        assert_eq!(healed.risk_level.as_deref(), Some("high"));
        assert_eq!(healed.action_category.as_deref(), Some("fs"));
        assert!(
            healed
                .action_payload
                .as_deref()
                .unwrap_or_default()
                .contains("fs"),
            "payload: {:?}",
            healed.action_payload
        );
        assert_eq!(healed.reviewer, "user");
        assert_eq!(
            store::get_run(&run_heal.id)
                .expect("run")
                .expect("some")
                .status,
            "waiting_approval"
        );
    }

    #[tokio::test]
    async fn reconcile_skips_when_there_is_nothing_actionable() {
        let home = TestHome::new("ap-reconcile-skip");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");

        // No pending approvals at all: no agent traffic.
        reconcile_pending_approvals().await;
        assert!(mock.requests().is_empty());

        // Pending approval on a thread with no agent session → skipped.
        let no_session = seed_thread(&workspace.id, None);
        let run = seed_run(&no_session.id);
        seed_approval("appr-nosession", &run.id);
        reconcile_pending_approvals().await;
        assert!(mock.requests().is_empty());
        assert_eq!(
            store::get_approval_request("appr-nosession")
                .expect("query")
                .expect("exists")
                .status,
            "pending"
        );

        // Thread deleted under the approval (cascades the row) → skipped.
        let thread = seed_thread(&workspace.id, Some("sess-2"));
        let run2 = seed_run(&thread.id);
        seed_approval("appr-orphan", &run2.id);
        store::delete_thread(&thread.id).expect("delete thread");
        reconcile_pending_approvals().await;
        assert!(mock.requests().is_empty());
        assert!(
            store::get_approval_request("appr-orphan")
                .expect("query")
                .is_none(),
            "thread deletion cascades the approval row"
        );

        // Agent rejects get_state → rows left pending.
        let thread3 = seed_thread(&workspace.id, Some("sess-3"));
        let run3 = seed_run(&thread3.id);
        seed_approval("appr-rejected", &run3.id);
        mock.push("get_state", Reply::Reject("unknown session".to_string()));
        reconcile_pending_approvals().await;
        assert_eq!(
            store::get_approval_request("appr-rejected")
                .expect("query")
                .expect("exists")
                .status,
            "pending"
        );

        // Transport failure → rows left pending.
        mock.push("get_state", Reply::Status(tonic::Code::Unavailable, "down"));
        reconcile_pending_approvals().await;
        assert_eq!(
            store::get_approval_request("appr-rejected")
                .expect("query")
                .expect("exists")
                .status,
            "pending"
        );

        // Connect failure → skipped before any request.
        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").ok();
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");
        let before = mock.requests().len();
        reconcile_pending_approvals().await;
        if let Some(prev) = prev {
            std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);
        }
        assert_eq!(
            mock.requests().len(),
            before,
            "no traffic on connect failure"
        );
    }

    #[tokio::test]
    async fn reconcile_heal_requires_an_active_run_with_a_local_row() {
        let home = TestHome::new("ap-reconcile-heal-guards");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));
        let run = seed_run(&thread.id);
        seed_approval("appr-local", &run.id);

        // No activeRun in the agent state → the unknown payload is not healed.
        mock.push_data(
            "get_state",
            serde_json::json!({
                "pendingApprovals": [
                    {"approval_request_id": "appr-local"},
                    {"approval_request_id": "appr-no-run", "tool_name": "shell"}
                ]
            }),
        );
        reconcile_pending_approvals().await;
        assert!(
            store::get_approval_request("appr-no-run")
                .expect("query")
                .is_none(),
            "no active run → no heal"
        );

        // activeRun points at a run the GUI does not have → not healed.
        seed_approval("appr-local-2", &run.id);
        store::decide_approval_request(store::DecideApprovalRequestInput {
            approval_request_id: "appr-local".to_string(),
            status: "approved".to_string(),
            decision_note: None,
        })
        .expect("decide");
        mock.push_data(
            "get_state",
            serde_json::json!({
                "activeRun": {"runId": "run-not-local"},
                "pendingApprovals": [
                    {"approval_request_id": "appr-local-2"},
                    {"approval_request_id": "appr-stray"}
                ]
            }),
        );
        reconcile_pending_approvals().await;
        assert!(
            store::get_approval_request("appr-stray")
                .expect("query")
                .is_none(),
            "unknown local run → no heal"
        );
    }

    #[tokio::test]
    async fn reconcile_returns_silently_when_the_store_is_unreadable() {
        let _home = TestHome::new("ap-reconcile-broken");
        let _mock = mock_agent();
        let prev = super::super::test_support::break_home();
        reconcile_pending_approvals().await;
        super::super::test_support::restore_home(prev);
    }

    #[tokio::test]
    async fn reconcile_skips_an_approval_whose_thread_vanished_midway() {
        let home = TestHome::new("ap-reconcile-ghost");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-ghost"));
        let run = seed_run(&thread.id);
        seed_approval("appr-ghost", &run.id);

        // Delete ONLY the thread row (raw connection, FK off) so the pending
        // approval outlives its thread — the crash-window state the continue
        // arm guards.
        let conn =
            rusqlite::Connection::open(home.path().join(".future/app/app.db")).expect("open db");
        conn.execute_batch("PRAGMA foreign_keys = OFF;")
            .expect("fk off");
        conn.execute("DELETE FROM threads WHERE id = ?1", [&thread.id])
            .expect("delete thread row");
        drop(conn);

        reconcile_pending_approvals().await;
        assert!(
            mock.requests().is_empty(),
            "a thread-less approval is skipped without agent traffic"
        );
        assert_eq!(
            store::get_approval_request("appr-ghost")
                .expect("query")
                .expect("exists")
                .status,
            "pending",
            "the row is left pending for the next tick"
        );
    }

    #[tokio::test]
    async fn reconcile_heal_logs_and_continues_when_the_write_fails() {
        let home = TestHome::new("ap-reconcile-heal-locked");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));
        let run = seed_run(&thread.id);
        seed_approval("appr-local", &run.id);

        mock.push_data(
            "get_state",
            serde_json::json!({
                "activeRun": {"runId": run.id},
                "pendingApprovals": [
                    {"approval_request_id": "appr-local"},
                    {"approval_request_id": "appr-blocked", "tool_name": "shell"}
                ]
            }),
        );

        // Hold the write lock from a second connection: reads still work
        // (WAL), but the heal's INSERT times out and is only logged.
        let mut conn =
            rusqlite::Connection::open(home.path().join(".future/app/app.db")).expect("open db");
        conn.execute_batch("PRAGMA busy_timeout = 100;")
            .expect("busy timeout");
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Exclusive)
            .expect("exclusive lock");
        reconcile_pending_approvals().await;
        tx.rollback().expect("rollback");

        assert!(
            store::get_approval_request("appr-blocked")
                .expect("query")
                .is_none(),
            "the failed heal did not create a row"
        );
    }
}
