#![cfg(all(test, feature = "gui"))]
use super::test_support::{mock_agent, seed_thread, seed_workspace, Reply, TestHome};
use super::*;
use serde_json::json;

#[cfg(feature = "gui")]
#[tokio::test]
async fn foreign_session_metadata_does_not_overwrite_a_conversation_title() {
    let home = TestHome::new("metadata-source-fence");
    let mock = mock_agent();
    let workspace = seed_workspace(home.path(), "source");
    let thread = seed_thread(&workspace.id, Some("session"));
    crate::store::record_thread_source(&thread.id, "session", "original-database", "generation")
        .unwrap();
    mock.push_data(
        "get_session_presence",
        json!({"databaseId":"foreign-database","restoreGeneration":"generation",
            "sessions":[{"sessionId":"session","status":"present"}]}),
    );
    let mut state = crate::agent_bridge::test_support::get_state_payload("session", false);
    state["sessionName"] = "foreign title".into();
    mock.push_typed_data("get_state", state);
    assert!(crate::commands::get_thread_agent_state(thread.id.clone())
        .await
        .is_err());
    assert_eq!(
        crate::store::get_thread(&thread.id).unwrap().unwrap().title,
        thread.title
    );
    assert!(mock.requests_of("get_state").is_empty());
}

#[cfg(feature = "gui")]
#[tokio::test]
async fn history_does_not_follow_a_reconnected_cache_to_another_database() {
    // The wait below is the **escape hatch**, not this test's subject: it answers "did the
    // spawned read reach the mock at all", and its expiry means a hang, not a slow host. Task
    // scheduling under the instrumented suite is not this test's business, so the budget is
    // armed — measured red here on 2026-11-01 at 5 s while the full instrumented lib suite ran
    // (`Elapsed(())` at this line), green alone. Same family as
    // `crate::test_support::wait_budget` documents.
    let _waits = crate::test_support::wait_budget::WaitBudgetGuard::armed(
        std::time::Duration::from_secs(120),
    );
    let home = TestHome::new("history-instance-fence");
    let mock = mock_agent();
    let workspace = seed_workspace(home.path(), "source");
    let thread = seed_thread(&workspace.id, Some("session"));
    let (entered, observed) = tokio::sync::oneshot::channel();
    let (answer, data) = tokio::sync::oneshot::channel();
    mock.push("get_session_presence", Reply::Deferred { entered, data });
    let thread_id = thread.id.clone();
    let read = tokio::spawn(async move {
        crate::commands::get_session_entries_page(thread_id, None, 10, None).await
    });
    tokio::time::timeout(
        crate::test_support::wait_budget::wait_budget(std::time::Duration::from_secs(5)),
        observed,
    )
    .await
    .unwrap()
    .unwrap();
    for _ in 0..3 {
        mock.push_data("get_agent_readiness",json!({
                "version":crate::build_info::VERSION,"agentInstanceId":"replacement-agent",
                "protocolVersion":future_rpc::connection::AGENT_PROTOCOL_VERSION,
                "dataRootId":"0".repeat(64),"storageSchemaVersion":future_rpc::connection::AGENT_STORAGE_SCHEMA_VERSION,
            }));
    }
    // Another caller detects the restart, then installs its new connection.
    assert!(connect_agent().await.is_err());
    let replacement = connect_agent().await.unwrap();
    assert_eq!(
        replacement.readiness().agent_instance_id,
        "replacement-agent"
    );
    mock.push("get_session_entries",Reply::BoundData {
            instance:"replacement-agent",
            data:json!({"entries":[{"id":"foreign","kind":"user","role":"user","createdAtMs":1,
                "blocks":[{"kind":"text","text":"foreign history"}]}],"hasMore":false,"nextOffset":0}),
        });
    answer
        .send(
            json!({"databaseId":"original-database","restoreGeneration":"original-generation",
            "sessions":[{"sessionId":"session","status":"present"}]}),
        )
        .unwrap();
    assert!(read.await.unwrap().is_err());
    assert_eq!(
        crate::store::thread_source(&thread.id).unwrap(),
        Some(("original-database".into(), "original-generation".into()))
    );
    assert!(crate::store::get_thread(&thread.id).unwrap().is_some());
}
