//! Tauri IPC adapter for shared application operations.
use crate::store;
pub use future_host::application::approvals::{SaveApprovalRuleInput, SaveApprovalRulesInput};
#[tauri::command]
pub fn list_approval_requests(
    thread_id: String,
) -> Result<Vec<store::ApprovalRequestRecord>, crate::AppError> {
    future_host::application::approvals::list_approval_requests(thread_id)
}

#[tauri::command]
pub fn list_pending_approval_requests() -> Result<Vec<store::ApprovalRequestRecord>, crate::AppError>
{
    future_host::application::approvals::list_pending_approval_requests()
}

#[tauri::command]
pub async fn decide_approval_request(
    input: store::DecideApprovalRequestInput,
) -> Result<store::ApprovalRequestRecord, crate::AppError> {
    future_host::application::approvals::decide_approval_request(input).await
}

#[tauri::command]
pub async fn save_approval_rule(input: SaveApprovalRuleInput) -> Result<(), crate::AppError> {
    future_host::application::approvals::save_approval_rule(input).await
}

#[tauri::command]
pub async fn save_approval_rules(input: SaveApprovalRulesInput) -> Result<(), crate::AppError> {
    future_host::application::approvals::save_approval_rules(input).await
}

#[tauri::command]
pub fn list_approval_assessments(
    run_id: String,
) -> Result<Vec<store::ApprovalAssessmentRecord>, crate::AppError> {
    future_host::application::approvals::list_approval_assessments(run_id)
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
    fn async_command_wrappers_reject_malformed_bodies() {
        crate::commands::ipc_harness::assert_all_reject_bad_body(
            tauri::generate_handler![
                decide_approval_request,
                save_approval_rule,
                save_approval_rules
            ],
            &[
                "decide_approval_request",
                "save_approval_rule",
                "save_approval_rules",
            ],
        );
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

    /// The three approval *reads* plus the two rule writes the panel calls. Each
    /// read is narrowed to the id the caller passed, and the rule writes are
    /// proven by the file the workspace now holds — the one artifact both
    /// arguments (path and access) end up in.
    #[tokio::test]
    async fn the_approval_commands_forward_their_arguments() {
        let _lock = crate::commands::agent_mock::mock_agent_lock();
        let (_home, thread) = seeded("cmd_approval_reads");
        with_approval(&thread, "run_reads");
        crate::commands::agent_mock::ensure_mock_agent();
        // A second conversation, so "everything" and "this thread" differ.
        let other = crate::store::create_thread(store::CreateThreadInput {
            mode: "chat".into(),
            title: Some("Other".into()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        })
        .expect("create thread");

        let mine = list_approval_requests(thread.id.clone()).expect("thread approvals");
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].run_id.as_deref(), Some("run_reads"));
        assert_eq!(mine[0].kind, "shell");
        assert!(
            list_approval_requests(other.id.clone())
                .expect("other thread")
                .is_empty(),
            "another conversation's approvals are not this conversation's"
        );

        assert_eq!(
            list_pending_approval_requests().expect("pending").len(),
            1,
            "a freshly requested approval is pending"
        );

        // Nothing has assessed this run yet: the answer is the empty list the
        // context panel renders as "no assessment", not an error.
        assert!(list_approval_assessments("run_reads".into())
            .expect("assessments")
            .is_empty());

        let workspace = crate::store::get_workspace(&thread.workspace_id)
            .expect("get workspace")
            .expect("the workspace exists");
        // Both writes reach the live agent too, and that half is best-effort:
        // the rule file is the durable answer this test can assert.
        let _ = save_approval_rule(SaveApprovalRuleInput {
            thread_id: thread.id.clone(),
            path: "src/**".into(),
            access: "read".into(),
        })
        .await;
        let _ = save_approval_rules(SaveApprovalRulesInput {
            thread_id: thread.id.clone(),
            rules: vec![future_host::application::approvals::SaveApprovalRuleEntry {
                path: "docs/**".into(),
                access: "write".into(),
            }],
        })
        .await;
        let rules = std::fs::read_to_string(
            std::path::Path::new(&workspace.path).join(".future/approval_rule.json"),
        )
        .expect("the workspace now holds an approval rule file");
        assert!(rules.contains("src/**"), "{rules}");
        assert!(rules.contains("docs/**"), "{rules}");

        // A conversation that does not exist is reported as such rather than
        // written to some other workspace.
        let ghost = save_approval_rule(SaveApprovalRuleInput {
            thread_id: "thread_absent".into(),
            path: "src/**".into(),
            access: "read".into(),
        })
        .await
        .expect_err("an unknown conversation must not be written to");
        assert!(
            ghost.to_string().contains("Thread could not be loaded"),
            "{ghost}"
        );
    }
}
