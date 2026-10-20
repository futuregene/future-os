#![cfg(all(test, feature = "gui"))]
use super::test_support::{mock_agent, seed_run, seed_thread, seed_workspace, TestHome};
use super::*;
#[cfg(feature = "gui")]
#[tokio::test]
async fn gui_prompt_notifies_acceptance_before_stream_completion() {
    let home = TestHome::new("gui-prompt-ack");
    let mock = mock_agent();
    let workspace = seed_workspace(home.path(), "ws");
    let thread = seed_thread(&workspace.id, None);
    crate::store::rename_thread(crate::store::RenameThreadInput {
        thread_id: thread.id.clone(),
        title: "New Chat".into(),
    })
    .expect("rename");
    let run = seed_run(&thread.id);
    mock.push_data(
        "new_session",
        serde_json::json!({"sessionId": "sess-gui-ack"}),
    );
    let (message, thread_id, run_id) = (
        "hello from the test".to_string(),
        thread.id.clone(),
        run.id.clone(),
    );
    let accepted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let signal = accepted.clone();
    let channel = tauri::ipc::Channel::new(move |_| {
        signal.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    });
    // The mock accepts prompts but its default stream remains pending.
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        crate::commands::forward_prompt_acceptance(
            AgentPromptRequest {
                message,
                model_context: String::new(),
                attachments: None,
                thread_id,
                session_id: None,
                run_id: Some(run_id),
                model_id: None,
                thinking_level: None,
            },
            Some(channel),
        ),
    )
    .await;
    assert!(result.is_err(), "the response must still be streaming");
    assert!(
        accepted.load(std::sync::atomic::Ordering::SeqCst),
        "commands reached: {:?}",
        mock.requests()
            .iter()
            .map(|request| &request.r#type)
            .collect::<Vec<_>>()
    );
}
