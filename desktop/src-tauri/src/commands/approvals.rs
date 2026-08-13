//! Approval-request Tauri commands. `decide_approval_request` delegates its
//! agent + store orchestration to [`crate::agent_bridge`].

use serde::Deserialize;

use crate::{agent_bridge, store};

#[tauri::command]
pub fn list_approval_requests(
    thread_id: String,
) -> Result<Vec<store::ApprovalRequestRecord>, crate::AppError> {
    store::list_approval_requests(&thread_id)
}

/// Pending approvals across ALL threads — the sidebar badge source, so an
/// approval raised in a background conversation is visible without opening it.
#[tauri::command]
pub fn list_pending_approval_requests() -> Result<Vec<store::ApprovalRequestRecord>, crate::AppError>
{
    store::list_pending_approval_requests()
}

#[tauri::command]
pub async fn decide_approval_request(
    input: store::DecideApprovalRequestInput,
) -> Result<store::ApprovalRequestRecord, crate::AppError> {
    agent_bridge::decide_approval(input).await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveApprovalRuleInput {
    /// Thread the rule was created from — resolves the target workspace dir.
    pub thread_id: String,
    /// Path glob (workspace-relative, or `~`/absolute), possibly user-edited.
    pub path: String,
    /// "read" | "write".
    pub access: String,
}

/// "Allow in this workspace/chat": append an allow rule to the workspace's
/// `.future/approval_rule.json` (persist, read next prompt) AND inject it into
/// the live agent session (same-run effect — APPROVAL_PLAN.md §6).
#[tauri::command]
pub async fn save_approval_rule(input: SaveApprovalRuleInput) -> Result<(), crate::AppError> {
    let workspace_id = store::get_thread(&input.thread_id)?
        .map(|thread| thread.workspace_id)
        .ok_or_else(|| "Thread could not be loaded.".to_string())?;
    let workspace = store::get_workspace(&workspace_id)?
        .ok_or_else(|| "Workspace could not be loaded.".to_string())?;
    crate::approval_rules::append_workspace_allow_rule(
        &workspace.path,
        &input.path,
        &input.access,
    )?;
    // Same-run effect (best-effort — persistence above already succeeded).
    agent_bridge::inject_session_rule(&input.thread_id, &input.path, &input.access).await
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]
    use super::*;
    use crate::auth_store::test_support::HomeGuard;

    fn seeded(label: &str) -> (HomeGuard, store::ThreadRecord) {
        let home = HomeGuard::new(label);
        crate::store::initialize_app_store().expect("init store");
        let ws = crate::store::create_workspace(store::CreateWorkspaceInput {
            name: Some("WS".into()),
            path: std::env::temp_dir()
                .join(format!("futureos-cmd-approval-ws-{}", std::process::id()))
                .display()
                .to_string(),
            description: None,
            create_directory: Some(true),
        })
        .expect("create workspace");
        let thread = crate::store::create_thread(store::CreateThreadInput {
            mode: "workspace".into(),
            title: Some("Approvals".into()),
            workspace_id: Some(ws.id.clone()),
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        })
        .expect("create thread");
        (home, thread)
    }

    fn with_approval(thread: &store::ThreadRecord, id: &str) {
        crate::store::create_run(store::CreateRunInput {
            id: Some(id.into()),
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .expect("create run");
        crate::store::ensure_approval_request(store::EnsureApprovalRequestInput {
            approval_request_id: Some(format!("approval_{id}")),
            run_id: id.into(),
            tool_call_id: None,
            kind: "shell".into(),
            title: "Deploy".into(),
            summary: None,
            risk_level: None,
            requested_action: None,
            action_category: None,
            action_payload: None,
            sandbox_boundary: None,
            save_suggestion: None,
            reviewer: None,
        })
        .expect("ensure approval");
    }

    #[test]
    fn list_approval_requests_returns_the_threads_pending_approval() {
        let (_home, thread) = seeded("cmd_approvals");
        assert!(list_approval_requests(thread.id.clone())
            .expect("list empty")
            .is_empty());
        with_approval(&thread, "run_1");
        let listed = list_approval_requests(thread.id.clone()).expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title, "Deploy");
    }

    #[test]
    fn pending_approvals_span_threads() {
        let (_home, thread) = seeded("cmd_approvals_pending");
        with_approval(&thread, "run_2");
        let pending = list_pending_approval_requests().expect("pending");
        assert_eq!(pending.len(), 1);
    }

    #[tokio::test]
    async fn decide_approval_notifies_the_agent_and_persists() {
        use crate::commands::agent_mock::{mock_agent_lock, script_mock_agent, MockScript};
        use std::collections::HashMap;

        let _lock = mock_agent_lock();
        let (_home, thread) = seeded("cmd_approval_decide");
        with_approval(&thread, "run_3");
        crate::commands::agent_mock::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([("approval_decision".to_string(), "{}".to_string())]),
            ..Default::default()
        });
        let updated = decide_approval_request(store::DecideApprovalRequestInput {
            approval_request_id: "approval_run_3".into(),
            status: "approved".into(),
            decision_note: Some("ok".into()),
        })
        .await
        .expect("decide");
        assert_eq!(updated.status, "approved");
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn decide_approval_cancels_when_the_agent_says_stale() {
        use crate::commands::agent_mock::{mock_agent_lock, script_mock_agent, MockScript};
        use std::collections::HashMap;

        let _lock = mock_agent_lock();
        let (_home, thread) = seeded("cmd_approval_stale");
        with_approval(&thread, "run_4");
        crate::commands::agent_mock::ensure_mock_agent();
        script_mock_agent(MockScript {
            errors: HashMap::from([(
                "approval_decision".to_string(),
                "approval request is not pending".to_string(),
            )]),
            ..Default::default()
        });
        let updated = decide_approval_request(store::DecideApprovalRequestInput {
            approval_request_id: "approval_run_4".into(),
            status: "approved".into(),
            decision_note: None,
        })
        .await
        .expect("decide stale");
        assert_eq!(updated.status, "cancelled");
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn save_approval_rule_writes_and_injects() {
        use crate::commands::agent_mock::{mock_agent_lock, script_mock_agent, MockScript};
        use std::collections::HashMap;

        let _lock = mock_agent_lock();
        let (_home, thread) = seeded("cmd_approval_rule");
        crate::commands::agent_mock::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([("add_session_rule".to_string(), "{}".to_string())]),
            ..Default::default()
        });
        save_approval_rule(SaveApprovalRuleInput {
            thread_id: thread.id.clone(),
            path: "src/**".into(),
            access: "read".into(),
        })
        .await
        .expect("save rule");
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn save_approval_rule_errors_for_unknown_thread() {
        let _home = HomeGuard::new("cmd_approval_rule_ghost");
        crate::store::initialize_app_store().expect("init store");
        assert!(save_approval_rule(SaveApprovalRuleInput {
            thread_id: "ghost".into(),
            path: "src/**".into(),
            access: "read".into(),
        })
        .await
        .is_err());
    }

    #[tokio::test]
    async fn save_approval_rule_rejects_an_invalid_access_scope() {
        let (_home, thread) = seeded("cmd_approval_rule_bad_access");
        // The persistence layer rejects a non-read/write access before any
        // agent call happens.
        assert!(save_approval_rule(SaveApprovalRuleInput {
            thread_id: thread.id.clone(),
            path: "src/**".into(),
            access: "execute".into(),
        })
        .await
        .is_err());
    }
}
