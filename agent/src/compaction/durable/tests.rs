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
            prepare_with_journal(
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
            )
        }
    });
    started.wait();
    let duplicate = prepare_with_journal(
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
    );
    assert!(
        matches!(duplicate,Err(ContextError::PersistenceFailed(ref error)) if error.contains("compaction_indeterminate"))
    );
    release.wait();
    let (prepared, ticket) = pending.join().unwrap().unwrap();
    let ContextPreparation::Compacted { checkpoint, .. } = prepared else {
        panic!("checkpoint expected")
    };
    assert_eq!(checkpoint.algorithm_version, "deterministic-s2-evidence-v1");
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
    let (replayed, ticket) = prepare_with_journal(
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
    )
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
