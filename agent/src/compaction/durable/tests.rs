use super::*;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Barrier,
};

#[test]
fn concurrent_claim_prepares_once_then_reuses_after_restart_without_a_provider() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Manager::new(dir.path().to_owned()));
    let mut raw = vec![
        AgentMessage::new_user("user", json!("original question")),
        AgentMessage::new_user("assistant", json!("verified answer")),
    ];
    for message in &mut raw {
        message.ensure_journal_entry_id();
    }
    let entries = raw
        .iter()
        .map(crate::session::agent_message_to_entry)
        .map(|entry| serde_json::to_value(entry).unwrap())
        .collect();
    store.storage().unwrap().replace("s", entries).unwrap();
    let journal = CompactionJournal::new(
        store.clone(),
        SessionPersistence::new(store, "s".into()),
        "s".into(),
        json!({"model":"m"}),
    );
    let manager = ContextManager {
        enabled: true,
        reserve_tokens: 1600,
        keep_recent_tokens: 1000,
        context_window: 8000,
        model: "m".into(),
    };
    let started = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let preparations = Arc::new(AtomicUsize::new(0));
    let pending = std::thread::spawn({
        let journal = journal.clone();
        let raw = raw.clone();
        let manager = manager.clone();
        let started = started.clone();
        let release = release.clone();
        let preparations = preparations.clone();
        move || {
            // No provider: this is the deterministic C admission path, driven from a
            // plain thread so the test keeps its concurrency shape.
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(prepare_with_journal_and_summary(
                    &manager,
                    project_prompt_context(&raw, None, None, 8000),
                    &raw,
                    CompactionTrigger::Manual,
                    CompactionPhase::Standalone,
                    None,
                    &AtomicBool::new(false),
                    Some(&|| {
                        preparations.fetch_add(1, Ordering::Relaxed);
                        started.wait();
                        release.wait();
                    }),
                    Some(&journal),
                    "first",
                    None,
                    None,
                    &[],
                    None,
                    None,
                ))
        }
    });
    started.wait();
    let duplicate = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(prepare_with_journal_and_summary(
            &manager,
            project_prompt_context(&raw, None, None, 8000),
            &raw,
            CompactionTrigger::Manual,
            CompactionPhase::Standalone,
            None,
            &AtomicBool::new(false),
            None,
            Some(&journal),
            "concurrent",
            None,
            None,
            &[],
            None,
            None,
        ));
    assert!(
        matches!(duplicate,Err(ContextError::PersistenceFailed(ref error)) if error.contains("compaction_indeterminate"))
    );
    release.wait();
    let (prepared, ticket) = pending.join().unwrap().unwrap();
    let ContextPreparation::Compacted { checkpoint, .. } = prepared else {
        panic!("checkpoint expected")
    };
    assert_eq!(checkpoint.algorithm_version, "deterministic-evidence-v1");
    ticket
        .unwrap()
        .finish(
            Some(&checkpoint),
            json!({"checkpointId":checkpoint.checkpoint_id}),
        )
        .unwrap();
    let fresh = Arc::new(Manager::new(dir.path().to_owned()));
    let journal = CompactionJournal::new(
        fresh.clone(),
        SessionPersistence::new(fresh, "s".into()),
        "s".into(),
        json!({"model":"m"}),
    );
    let (replayed, ticket) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(prepare_with_journal_and_summary(
            &manager,
            project_prompt_context(&raw, Some(&checkpoint), None, 8000),
            &raw,
            CompactionTrigger::Manual,
            CompactionPhase::Standalone,
            None,
            &AtomicBool::new(false),
            Some(&|| {
                preparations.fetch_add(1, Ordering::Relaxed);
            }),
            Some(&journal),
            "restart",
            None,
            None,
            &[],
            None,
            None,
        ))
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
    assert_eq!(preparations.load(Ordering::Relaxed), 1);
}

/// One admission against a journal, through the deterministic (provider-free)
/// path the ticket tests use.
fn admit_once(
    manager: &ContextManager,
    raw: &[AgentMessage],
    prompt: PromptContext,
    journal: &CompactionJournal,
    operation: &str,
) -> Result<(ContextPreparation, Option<CompactionTicket>), ContextError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(prepare_with_journal_and_summary(
            manager,
            prompt,
            raw,
            CompactionTrigger::Manual,
            CompactionPhase::Standalone,
            None,
            &AtomicBool::new(false),
            None,
            Some(journal),
            operation,
            None,
            None,
            &[],
            None,
            None,
        ))
}

/// A recorded compaction must not be replayed over a *later* one.
///
/// The receipt is keyed by the operation's policy, and the policy does not
/// include which checkpoint the prompt already carries — so a run whose prompt
/// was compacted again afterwards still finds the older receipt. Replaying it
/// would stamp the older summary over the newer checkpoint and, worse, restore
/// the metered input count that no longer describes the prompt. The admission
/// therefore answers `Unchanged` with the prompt it was given (minus the
/// measurement) and hands back a ticket whose recorded result the caller
/// replays.
#[test]
fn a_replay_that_would_undo_a_later_checkpoint_keeps_the_prompt_and_the_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Manager::new(dir.path().to_owned()));
    let mut raw = vec![
        AgentMessage::new_user("user", json!("original question")),
        AgentMessage::new_user("assistant", json!("verified answer")),
    ];
    for message in &mut raw {
        message.ensure_journal_entry_id();
    }
    let entries = raw
        .iter()
        .map(crate::session::agent_message_to_entry)
        .map(|entry| serde_json::to_value(entry).unwrap())
        .collect();
    store.storage().unwrap().replace("s", entries).unwrap();
    let journal = CompactionJournal::new(
        store.clone(),
        SessionPersistence::new(store, "s".into()),
        "s".into(),
        json!({"model":"m"}),
    );
    let manager = ContextManager {
        enabled: true,
        reserve_tokens: 1600,
        keep_recent_tokens: 1000,
        context_window: 8000,
        model: "m".into(),
    };

    // Record the receipt this history produces.
    let (fresh, ticket) = admit_once(
        &manager,
        &raw,
        project_prompt_context(&raw, None, None, 8000),
        &journal,
        "first",
    )
    .unwrap();
    let ContextPreparation::Compacted { checkpoint, .. } = fresh else {
        panic!("checkpoint expected");
    };
    ticket
        .unwrap()
        .finish(Some(&checkpoint), json!({"checkpointId":checkpoint.checkpoint_id}))
        .unwrap();

    // The same history, now carrying a *different* checkpoint: the one a later
    // run with other settings produced.
    let mut later = (*checkpoint).clone();
    later.entry_id = "later-checkpoint-entry".to_string();
    later.checkpoint_id = "later-checkpoint-id".to_string();
    let incoming = project_prompt_context(&raw, Some(&later), Some(999), 8000);
    assert_eq!(
        incoming.usage.input_tokens,
        Some(999),
        "the incoming prompt carries a metered count of its own"
    );

    let (replayed, ticket) = admit_once(&manager, &raw, incoming, &journal, "later").unwrap();
    let ContextPreparation::Unchanged { prompt } = replayed else {
        panic!("a replay that would undo the later checkpoint must leave the prompt unchanged");
    };
    assert_eq!(
        prompt.usage.input_tokens, None,
        "the stale measurement is dropped rather than reused"
    );
    let ticket = ticket.expect("a cached claim still hands back a ticket");
    assert!(
        ticket.cached_result().is_some(),
        "the ticket replays the recorded result"
    );

    // Failing a replayed ticket is a no-op: the good receipt is what a retry
    // finds, so a caller that calls `fail` on it cannot burn the record.
    ticket.fail("this retry claimed to have failed");
    let (again, _) = admit_once(
        &manager,
        &raw,
        project_prompt_context(&raw, None, None, 8000),
        &journal,
        "third",
    )
    .unwrap();
    let ContextPreparation::Compacted { checkpoint: kept, .. } = again else {
        panic!("the receipt must still replay");
    };
    assert_eq!(
        kept.checkpoint_id, checkpoint.checkpoint_id,
        "a failure reported for a cached ticket must not rewrite the receipt"
    );
}
