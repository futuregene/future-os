use super::*;
use crate::llm::schema::{FinishReason, ModelRequest, ModelStreamEvent};
use crate::types::LLMProvider;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio_stream::wrappers::ReceiverStream;

struct GatedProvider {
    calls: AtomicUsize,
    started: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
#[async_trait::async_trait]
impl LLMProvider for GatedProvider {
    async fn stream_model(
        &self,
        _: ModelRequest,
    ) -> anyhow::Result<ReceiverStream<ModelStreamEvent>> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.started.notify_one();
        self.release.notified().await;
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        tx.try_send(ModelStreamEvent::TextDelta {id:"s".into(),text:"## Objective\n- continue\n\n## Important Details\n- keep requirements\n\n## Work State\n### Completed\n- old work\n### Active\n- current task\n### Blocked\n- none\n\n## Next Move\n1. continue\n\n## Relevant Files\n- none".into()}).unwrap();
        tx.try_send(ModelStreamEvent::Finish {
            reason: FinishReason::Stop,
            usage: None,
        })
        .unwrap();
        Ok(ReceiverStream::new(rx))
    }
}

#[tokio::test]
async fn concurrent_claim_only_calls_model_once_then_reuses_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Manager::new(dir.path().to_owned()));
    let mut raw = vec![
        AgentMessage::new_user("user", json!("original question")),
        AgentMessage::new_user("assistant", json!("verified answer")),
    ];
    for m in &mut raw {
        m.ensure_journal_entry_id();
    }
    let entries = raw
        .iter()
        .map(crate::session::agent_message_to_entry)
        .map(|e| serde_json::to_value(e).unwrap())
        .collect();
    store.storage().unwrap().replace("s", entries).unwrap();
    let persistence = SessionPersistence::new(store.clone(), "s".into());
    let journal =
        CompactionJournal::new(store.clone(), persistence, "s".into(), json!({"model":"m"}));
    let manager = ContextManager {
        enabled: true,
        reserve_tokens: 1600,
        keep_recent_tokens: 1000,
        context_window: 8000,
        model: "m".into(),
    };
    let provider = Arc::new(GatedProvider {
        calls: AtomicUsize::new(0),
        started: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let pending = tokio::spawn({
        let journal = journal.clone();
        let raw = raw.clone();
        let manager = manager.clone();
        let provider = provider.clone();
        async move {
            prepare_with_journal(
                &manager,
                project_prompt_context(&raw, None, None, 8000),
                &raw,
                CompactionTrigger::Manual,
                CompactionPhase::Standalone,
                None,
                provider.as_ref(),
                &AtomicBool::new(false),
                None,
                None,
                None,
                Some(&journal),
                "first",
            )
            .await
        }
    });
    provider.started.notified().await;
    let duplicate = prepare_with_journal(
        &manager,
        project_prompt_context(&raw, None, None, 8000),
        &raw,
        CompactionTrigger::Manual,
        CompactionPhase::Standalone,
        None,
        provider.as_ref(),
        &AtomicBool::new(false),
        None,
        None,
        None,
        Some(&journal),
        "concurrent",
    )
    .await;
    assert!(
        matches!(duplicate,Err(ContextError::PersistenceFailed(ref error)) if error.contains("compaction_indeterminate"))
    );
    assert_eq!(provider.calls.load(Ordering::Relaxed), 1);
    provider.release.notify_one();
    let (prepared, ticket) = pending.await.unwrap().unwrap();
    let ContextPreparation::Compacted { checkpoint, .. } = prepared else {
        panic!("checkpoint expected")
    };
    ticket
        .unwrap()
        .finish(
            Some(&checkpoint),
            json!({"checkpointId":checkpoint.checkpoint_id}),
        )
        .unwrap();
    let fresh = Arc::new(Manager::new(dir.path().to_owned()));
    let fresh_journal = CompactionJournal::new(
        fresh.clone(),
        SessionPersistence::new(fresh, "s".into()),
        "s".into(),
        json!({"model":"m"}),
    );
    let (replayed, ticket) = prepare_with_journal(
        &manager,
        project_prompt_context(&raw, Some(&checkpoint), None, 8000),
        &raw,
        CompactionTrigger::Manual,
        CompactionPhase::Standalone,
        None,
        provider.as_ref(),
        &AtomicBool::new(false),
        None,
        None,
        None,
        Some(&fresh_journal),
        "restart",
    )
    .await
    .unwrap();
    let ContextPreparation::Compacted {
        checkpoint: replayed,
        ..
    } = replayed
    else {
        panic!("cached checkpoint expected")
    };
    assert_eq!(replayed.checkpoint_id, checkpoint.checkpoint_id);
    assert!(ticket.unwrap().cached_result().is_some());
    assert_eq!(provider.calls.load(Ordering::Relaxed), 1);
}
