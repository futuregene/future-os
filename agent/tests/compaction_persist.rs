use future_agent::compaction::{
    project_prompt_context, CompactionTrigger, ContextManager, ContextPreparation,
};
use future_agent::session::{
    agent_message_to_entry, checkpoint_to_entry, latest_context_checkpoint, Manager, Session,
};
use future_agent::types::{AgentMessage, ContentBlock};

#[test]
fn compaction_appends_checkpoint_without_discarding_jsonl_history() {
    let temp = tempfile::tempdir().unwrap();
    let manager = Manager::new(temp.path().to_path_buf());
    let padding = "x".repeat(10_000);
    let mut messages = Vec::new();
    for i in 0..40 {
        messages.push(AgentMessage {
            role: "user".into(),
            content: vec![ContentBlock::text(format!("turn {i}: {padding}"))],
            ..Default::default()
        });
        messages.push(AgentMessage {
            role: "assistant".into(),
            content: vec![ContentBlock::text(format!("response {i}: {padding}"))],
            ..Default::default()
        });
    }
    for message in &mut messages {
        message.ensure_journal_entry_id();
    }

    let original_entries = messages
        .iter()
        .map(agent_message_to_entry)
        .collect::<Vec<_>>();
    let original_ids = original_entries
        .iter()
        .map(|entry| entry.id.clone())
        .collect::<Vec<_>>();
    let session = Session::snapshot(
        "test-session".into(),
        "/tmp".into(),
        "test-model".into(),
        "compaction-test".into(),
        String::new(),
        original_entries,
    );
    manager.save(&session).unwrap();

    let prompt = project_prompt_context(&messages, None, Some(300_000), 50_000);
    let checkpoint = match (ContextManager {
        enabled: true,
        reserve_tokens: 16_384,
        keep_recent_tokens: 16_384,
        context_window: 50_000,
        model: "test-model".into(),
    })
    .prepare(prompt, CompactionTrigger::Automatic, None)
    .unwrap()
    {
        ContextPreparation::Compacted { checkpoint, .. } => checkpoint,
        ContextPreparation::Unchanged { .. } => panic!("long context should compact"),
    };
    manager
        .append_entries("test-session", &[checkpoint_to_entry(&checkpoint)])
        .unwrap();

    let reloaded = manager.load("test-session").unwrap();
    let reloaded_message_ids = reloaded
        .entries
        .iter()
        .filter(|entry| matches!(entry.role.as_str(), "user" | "assistant" | "tool"))
        .map(|entry| entry.id.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        reloaded_message_ids, original_ids,
        "checkpoint commit must not delete, reorder, or re-id user-visible history"
    );
    assert!(reloaded.entries.iter().any(|entry| {
        entry
            .content
            .as_ref()
            .is_some_and(|content| content.to_string().contains("turn 0"))
    }));
    assert!(reloaded.entries.iter().any(|entry| {
        entry
            .content
            .as_ref()
            .is_some_and(|content| content.to_string().contains("turn 39"))
    }));
    assert_eq!(
        latest_context_checkpoint(&reloaded.entries)
            .expect("durable v2 checkpoint")
            .checkpoint_id,
        checkpoint.checkpoint_id
    );
}
