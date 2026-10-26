use super::*;
use crate::session::reset::{DropKind, ResetSelectors};
use crate::session::{Manager, SessionEntry};
use serde_json::json;
static RESET_TESTS: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

fn history() -> (tempfile::TempDir, Manager, SqliteStore) {
    let root = tempfile::tempdir().unwrap();
    let manager = Manager::new(root.path().join("sessions"));
    let store = manager.storage().unwrap().clone();
    store.reserve_segmented("s").unwrap();
    store.replace("s",vec![
        json!({"id":"info","type":"session_info","timestamp":"2026-10-26T00:00:00Z","content":{"cwd":".","model":"mock","session_name":"retained"}}),
        json!({"id":"u","type":"user","role":"user","timestamp":"2026-10-26T00:00:00Z","meta":{"run_id":"r"},"content":[{"type":"text","text":"question"}]}),
        json!({"id":"a","type":"assistant","role":"assistant","timestamp":"2026-10-26T00:00:01Z","meta":{"run_id":"r"},"content":[{"type":"reasoning","text":"thought".repeat(4000)},{"type":"text","text":"answer".repeat(4000)},{"type":"tool_call","id":"call","name":"read","args":{"path":"a"},"provider_metadata":{"signature":"opaque"}}]}),
        json!({"id":"t","type":"tool","role":"tool","timestamp":"2026-10-26T00:00:02Z","meta":{"run_id":"r"},"content":[{"type":"tool_result","tool_call_id":"call","content":"result","is_error":true}]}),
        json!({"id":"end","type":"run_terminal","timestamp":"2026-10-26T00:00:03Z","content":{"run_id":"r","state":"completed","epoch":1}}),
    ]).unwrap();
    (root, manager, store)
}
fn body(store: &SqliteStore, field: &str) -> ContentRange {
    let field = field.to_owned();
    store
        .db
        .read(move |db| super::super::resolve(db, "s", "entry", "a", &field))
        .unwrap()
        .unwrap()
}

#[test]
fn reset_rebinds_shifted_ordinals_without_copying_retained_bodies_and_saves_the_original_backup() {
    let _serial = RESET_TESTS.lock();
    let (_root, manager, store) = history();
    let retained = body(&store, "/content/1/text");
    let args = body(&store, "/content/2/args");
    let mut writer = super::super::journal::Writer::default();
    writer.persist(&store,"s",&[json!({"session_id":"s","run_id":"r","epoch":1,"idx":0,"session_idx":-1,"event_type":"provider_fact","timestamp":"2026-10-26T00:00:01Z","data":"{}"})]).unwrap();
    let original = store.entries("s").unwrap();
    let outcome = manager
        .reset_history(
            "s",
            &ResetSelectors {
                drop: vec![DropKind::Thinking],
                ..Default::default()
            },
            true,
        )
        .unwrap();
    assert!(outcome.applied);
    assert!(
        !store
            .segmented_replay("s", "r", -1, false, Some(10))
            .unwrap()
            .known,
        "a modified surviving run must retire its old replay"
    );
    assert_eq!(body(&store, "/content/0/text"), retained);
    assert_eq!(body(&store, "/content/1/args"), args);
    let after = store.entries("s").unwrap();
    let answer = after.iter().find(|entry| entry["id"] == "a").unwrap();
    assert_eq!(answer["content"][0]["type"], "text");
    assert_eq!(answer["content"][0]["text"], "answer".repeat(4000));
    assert_eq!(answer["content"][1]["args"], json!({"path":"a"}));
    assert_eq!(
        answer["content"][1]["provider_metadata"]["signature"],
        "opaque"
    );
    let inline: i64 = store.db.read(|db|Ok(db.query_row("SELECT count(*) FROM message_blocks WHERE session_id='s' AND (length(text)>0 OR length(arguments_json)>0)",[],|row|row.get(0))?)).unwrap();
    assert_eq!(
        inline, 0,
        "reset must not reintroduce canonical inline copies"
    );
    let backup: Value =
        serde_json::from_slice(&std::fs::read(outcome.backup.unwrap()).unwrap()).unwrap();
    assert_eq!(backup["entries"], json!(original));
    let pending: i64 = store
        .db
        .read(|db| {
            Ok(
                db.query_row("SELECT count(*) FROM content_operations", [], |row| {
                    row.get(0)
                })?,
            )
        })
        .unwrap();
    assert_eq!(pending, 0);
}

#[test]
fn reset_cannot_overwrite_a_revision_that_changed_after_planning() {
    let _serial = RESET_TESTS.lock();
    let (_root, manager, store) = history();
    crate::session::reset::set_reset_race_hook(
        "s",
        Box::new(|manager| {
            manager
                .append_entries(
                    "s",
                    &[SessionEntry::new_user(
                        "user",
                        json!("concurrent accepted input"),
                    )],
                )
                .unwrap();
        }),
    );
    let error = manager
        .reset_history(
            "s",
            &ResetSelectors {
                drop: vec![DropKind::Thinking],
                ..Default::default()
            },
            true,
        )
        .unwrap_err();
    assert!(error.to_string().contains(HISTORY_CHANGED));
    let entries = store.entries("s").unwrap();
    assert!(entries
        .iter()
        .any(|entry| entry["content"][0]["text"] == "concurrent accepted input"));
    assert_eq!(
        entries.iter().find(|entry| entry["id"] == "a").unwrap()["content"][0]["type"],
        "reasoning"
    );
}

#[test]
fn an_abandoned_stage_cleans_only_its_own_pins_and_preserves_published_content() {
    let _serial = RESET_TESTS.lock();
    let (_root, _manager, store) = history();
    let before = store.entries("s").unwrap();
    let operation = uuid::Uuid::new_v4().to_string();
    let prefix = format!("reset:{operation}:");
    let (op, p) = (operation.clone(), prefix.clone());
    store.db.call(move |db| {
        db.execute("INSERT INTO content_operations(operation_id,session_id,owner_prefix,expected_revision,kind) SELECT ?1,'s',?2,revision,'reset' FROM sessions WHERE id='s'",params![op,p])?;
        Ok(())
    }).unwrap();
    super::super::entries::stage_named(
        &store,
        "s",
        "checkpoint",
        &format!("{prefix}unpublished"),
        "/content/0/text",
        "orphan staged bytes",
        false,
    )
    .unwrap();
    super::super::entries::stage_named(
        &store,
        "s",
        "checkpoint",
        "unrelated-checkpoint",
        "summary",
        "must survive",
        false,
    )
    .unwrap();
    store
        .recover_content_operations(&crate::service::StartupCancellation::default())
        .unwrap();
    assert_eq!(store.entries("s").unwrap(), before);
    let (operations,unrelated):(i64,i64) = store.db.read(|db|Ok((db.query_row("SELECT count(*) FROM content_operations",[],|row|row.get(0))?,db.query_row("SELECT count(*) FROM content_references WHERE owner_kind='checkpoint' AND owner_id='unrelated-checkpoint'",[],|row|row.get(0))?))).unwrap();
    assert_eq!((operations, unrelated), (0, 1));
}

#[test]
fn reset_retains_an_unsealed_recovered_prefix_without_sealing_its_unpublished_tail() {
    let _serial = RESET_TESTS.lock();
    let root = tempfile::tempdir().unwrap();
    let manager = Manager::new(root.path().join("sessions"));
    let store = manager.storage().unwrap().clone();
    store.reserve_segmented("s").unwrap();
    store.replace("s",vec![json!({"id":"info","type":"session_info","timestamp":"2026-10-26T00:00:00Z","content":{"cwd":".","model":"mock"}})]).unwrap();
    store.commit("s",vec![json!({"id":"u","type":"user","role":"user","timestamp":"2026-10-26T00:00:00Z","meta":{"run_id":"r"},"content":[{"type":"text","text":"question"}]}),json!({"id":"start","type":"run_started","timestamp":"2026-10-26T00:00:00Z","content":{"run_id":"r","epoch":1}})],vec![]).unwrap();
    let event = |idx, kind, data: Value| json!({"session_id":"s","run_id":"r","epoch":1,"idx":idx,"session_idx":-1,"event_type":kind,"timestamp":"2026-10-26T00:00:01Z","data":data.to_string()});
    let mut writer = super::super::journal::Writer::default();
    writer.persist(&store,"s",&[
        event(0,"message_start",json!({"entry_id":"a"})),
        event(1,"thinking_delta",json!({"entry_id":"a","content_key":"a:thinking:b","content_offset":0,"content_bytes":7,"content_first_idx":1,"text":"thought"})),
        event(2,"text_chunk",json!({"entry_id":"a","content_key":"a:text:b","content_offset":0,"content_bytes":4,"content_first_idx":2,"text":"kept"})),
    ]).unwrap();
    store
        .db
        .call(|db| {
            super::super::stream::commit_batch(
                db,
                &super::super::stream::StreamIdentity {
                    session_id: "s".into(),
                    run_id: "r".into(),
                    epoch: 1,
                },
                vec![super::super::stream::ContentAppend {
                    key: "a:text:b".into(),
                    offset: 4,
                    text: "unpublished".into(),
                    seal: None,
                }],
                vec![],
            )
            .map(|_| ())
        })
        .unwrap();
    store.recover_segmented_runs().unwrap();
    let retained = body(&store, "/content/1/text");
    assert_eq!(retained.length, 4);
    manager
        .reset_history(
            "s",
            &ResetSelectors {
                drop: vec![DropKind::Thinking],
                ..Default::default()
            },
            true,
        )
        .unwrap();
    assert_eq!(body(&store, "/content/0/text"), retained);
    let after = store.entries("s").unwrap();
    assert_eq!(
        after.iter().find(|entry| entry["id"] == "a").unwrap()["content"][0]["text"],
        "kept"
    );
}

#[test]
fn modified_runs_stop_replaying_immediately_and_retirement_is_bounded() {
    let _serial = RESET_TESTS.lock();
    let (_root, manager, store) = history();
    let mut writer = super::super::journal::Writer::default();
    let mut events = Vec::new();
    for idx in 0..160 {
        events.push(json!({"session_id":"s","run_id":"r","epoch":1,"idx":idx,"session_idx":-1,"event_type":"provider_fact","timestamp":"2026-10-26T00:00:01Z","data":json!({"text":format!("fact-{idx}")}).to_string()}));
    }
    writer.persist(&store, "s", &events).unwrap();
    let identity = store.storage_identity().unwrap();
    let request = future_rpc::content_range::ContentRequest {
        database_id: identity.database_id,
        restore_generation: identity.restore_generation,
        epoch: 1,
        cursor: 0,
        field: "/data/text".into(),
        offset: 0,
        max_bytes: 64,
    };
    assert_eq!(
        store
            .event_content_range("s", "r", request.clone())
            .unwrap()["text"],
        "fact-0"
    );
    // Measure one retirement slice inside one writer job so the background
    // scheduler cannot race the counters this specification compares.
    store.db.call(|db| {
        db.execute("INSERT INTO retired_streams(stream_id) SELECT id FROM stream_runs WHERE session_id='s' AND run_id='r'",[])?;
        let before:i64 = db.query_row("SELECT count(*) FROM stream_changes",[],|row|row.get(0))?;
        reclaim_retired(db)?;
        let after:i64 = db.query_row("SELECT count(*) FROM stream_changes",[],|row|row.get(0))?;
        assert!((1..=64).contains(&(before-after)));
        Ok(())
    }).unwrap();
    assert!(store.event_content_range("s", "r", request).is_err());
    assert!(
        !store
            .segmented_replay("s", "r", -1, false, Some(10))
            .unwrap()
            .known
    );
    manager
        .reset_history(
            "s",
            &ResetSelectors {
                drop: vec![DropKind::Thinking],
                ..Default::default()
            },
            true,
        )
        .unwrap();
    assert!(
        !store
            .segmented_replay("s", "r", -1, false, Some(10))
            .unwrap()
            .known
    );
}

#[test]
fn a_failed_publication_rolls_back_original_owners_before_abandoning_staging() {
    let _serial = RESET_TESTS.lock();
    let (_root, manager, store) = history();
    let before = store.entries("s").unwrap();
    let original = body(&store, "/content/1/text");
    store.db.call(|db| {
        db.execute_batch("CREATE TRIGGER reset_publish_failure BEFORE DELETE ON entries WHEN OLD.session_id='s' BEGIN SELECT RAISE(ABORT,'injected reset publication failure'); END;")?;
        Ok(())
    }).unwrap();
    let error = manager
        .reset_history(
            "s",
            &ResetSelectors {
                drop: vec![DropKind::Thinking],
                ..Default::default()
            },
            true,
        )
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("injected reset publication failure"));
    assert_eq!(store.entries("s").unwrap(), before);
    assert_eq!(body(&store, "/content/1/text"), original);
    let staged:i64 = store.db.read(|db|Ok(db.query_row("SELECT count(*) FROM content_references WHERE owner_kind='checkpoint' AND owner_id LIKE 'reset:%'",[],|row|row.get(0))?)).unwrap();
    assert_eq!(staged, 0);
}

#[test]
fn another_manager_open_does_not_reclaim_an_in_progress_stage_in_this_process() {
    let _serial = RESET_TESTS.lock();
    let (root, _manager, store) = history();
    let operation = uuid::Uuid::new_v4().to_string();
    let active = ActiveOperation::claim(&operation);
    let prefix = format!("reset:{operation}:");
    let (op, p) = (operation.clone(), prefix.clone());
    store.db.call(move |db| {
        db.execute("INSERT INTO content_operations(operation_id,session_id,owner_prefix,expected_revision,kind) SELECT ?1,'s',?2,revision,'reset' FROM sessions WHERE id='s'",params![op,p])?;
        Ok(())
    }).unwrap();
    super::super::entries::stage_named(
        &store,
        "s",
        "checkpoint",
        &format!("{prefix}pending"),
        "/content/0/text",
        "still staging",
        false,
    )
    .unwrap();
    let second = Manager::new(root.path().join("sessions"));
    second.storage().unwrap();
    let count: i64 = store
        .db
        .read(|db| {
            Ok(
                db.query_row("SELECT count(*) FROM content_operations", [], |row| {
                    row.get(0)
                })?,
            )
        })
        .unwrap();
    assert_eq!(count, 1);
    drop(active);
    store
        .recover_content_operations(&crate::service::StartupCancellation::default())
        .unwrap();
    let count: i64 = store
        .db
        .read(|db| {
            Ok(
                db.query_row("SELECT count(*) FROM content_operations", [], |row| {
                    row.get(0)
                })?,
            )
        })
        .unwrap();
    assert_eq!(count, 0);
}
