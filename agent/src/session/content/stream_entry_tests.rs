//! Source-bound transcript publication; run during the final acceptance pass.
use super::{journal::Writer, stream_entry::StreamEntry};
use crate::{
    session::sqlite_store::SqliteStore,
    types::{AgentMessage, ContentBlock},
};
use serde_json::{json, Value};

fn store() -> (tempfile::TempDir, SqliteStore) {
    let root = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&root.path().join("agent.db")).unwrap();
    store.reserve_segmented("s").unwrap();
    store.replace("s",vec![json!({"id":"info","type":"session_info","timestamp":"2026-10-26T00:00:00Z","content":{"model":"mock"}})]).unwrap();
    (root, store)
}
fn event(idx: i64, kind: &str, data: Value) -> Value {
    json!({"session_id":"s","run_id":"r","epoch":1,"idx":idx,"event_type":kind,"timestamp":"2026-10-26T00:00:00Z","data":data})
}
fn message(blocks: Vec<ContentBlock>, keys: Value) -> AgentMessage {
    AgentMessage {
        role: "assistant".into(),
        content: blocks,
        metadata: Some(
            [
                (AgentMessage::JOURNAL_ENTRY_ID_KEY.into(), json!("m")),
                (AgentMessage::STREAM_BLOCKS_KEY.into(), keys),
            ]
            .into_iter()
            .collect(),
        ),
        ..Default::default()
    }
}

#[test]
fn final_assistant_header_shares_text_and_provider_ranges_without_staging_more_bodies() {
    let (_root, store) = store();
    let text = "流畅正文".repeat(20_000);
    let provider = json!({"anthropic":{"signature":"签名".repeat(10_000)}});
    let mut writer = Writer::default();
    writer.persist(&store,"s",&[
        event(0,"message_start",json!({"entry_id":"m"})),
        event(1,"thinking_start",json!({"entry_id":"m","content_key":"m:thinking:b","content_offset":0,"content_bytes":0})),
        event(2,"thinking_delta",json!({"entry_id":"m","content_key":"m:thinking:b","content_offset":0,"content_bytes":text.len(),"text":text})),
        event(3,"thinking_end",json!({"entry_id":"m","content_key":"m:thinking:b","content_offset":text.len(),"content_bytes":0,"content_seal":true,"provider_metadata":provider})),
        event(4,"message_end",json!({"entry_id":"m","complete":true})),
    ]).unwrap();
    let before: i64 = store
        .db
        .read(|db| Ok(db.query_row("SELECT count(*) FROM content_objects", [], |row| row.get(0))?))
        .unwrap();
    let message = message(
        vec![ContentBlock::reasoning(
            &text,
            provider.as_object().unwrap().clone(),
        )],
        json!(["m:thinking:b"]),
    );
    let recipe = StreamEntry::from_assistant(&message, "r", 1).unwrap();
    assert_eq!(recipe.header.content.as_ref().unwrap()[0]["text"], "");
    assert_eq!(
        recipe.header.content.as_ref().unwrap()[0]["provider_metadata"],
        json!({})
    );
    store.append_stream_entry("s", recipe).unwrap();
    let actual = store
        .entries("s")
        .unwrap()
        .into_iter()
        .find(|entry| entry["id"] == "m")
        .unwrap();
    assert_eq!(actual["content"][0]["text"], text);
    assert_eq!(actual["content"][0]["provider_metadata"], provider);
    let after: i64 = store
        .db
        .read(|db| Ok(db.query_row("SELECT count(*) FROM content_objects", [], |row| row.get(0))?))
        .unwrap();
    assert_eq!(before, after);
    assert!(store
        .append_stream_entry(
            "s",
            StreamEntry::from_assistant(&message, "other-run", 1).unwrap()
        )
        .is_err());
    assert!(store
        .append_stream_entry("s", StreamEntry::from_assistant(&message, "r", 2).unwrap())
        .is_err());
}

#[test]
fn unfinished_argument_string_reuses_the_raw_range_and_preserves_the_model_interpretation() {
    let (_root, store) = store();
    let raw = "{\"path\":\"unfinished";
    let mut expected = json!(raw);
    future_rpc::tool_arguments::repair_string_v1(&mut expected);
    let mut writer = Writer::default();
    writer.persist(&store,"s",&[
        event(0,"message_start",json!({"entry_id":"m"})),
        event(1,"tool_start",json!({"entry_id":"m","phase":"input","tool_id":"t","tool_name":"read","content_key":"m:1:tool:0","content_offset":0,"content_bytes":raw.len(),"tool_args":raw})),
        event(2,"message_end",json!({"entry_id":"m","complete":false})),
    ]).unwrap();
    let message = message(
        vec![ContentBlock::tool_call(
            "t",
            "read",
            expected.clone(),
            Default::default(),
        )],
        json!([null]),
    );
    store
        .append_stream_entry("s", StreamEntry::from_assistant(&message, "r", 1).unwrap())
        .unwrap();
    let actual = store
        .entries("s")
        .unwrap()
        .into_iter()
        .find(|entry| entry["id"] == "m")
        .unwrap();
    assert_eq!(actual["content"][0]["args"], expected);
    let count: i64 = store
        .db
        .read(|db| Ok(db.query_row("SELECT count(*) FROM content_objects", [], |row| row.get(0))?))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn header_publication_requires_the_committed_message_boundary() {
    let (_root, store) = store();
    let mut writer = Writer::default();
    writer.persist(&store,"s",&[
        event(0,"message_start",json!({"entry_id":"m"})),
        event(1,"text_start",json!({"entry_id":"m","content_key":"m:text:b","content_offset":0,"content_bytes":0})),
        event(2,"text_chunk",json!({"entry_id":"m","content_key":"m:text:b","content_offset":0,"content_bytes":4,"text":"body"})),
    ]).unwrap();
    let message = message(vec![ContentBlock::text("body")], json!(["m:text:b"]));
    assert!(store
        .append_stream_entry("s", StreamEntry::from_assistant(&message, "r", 1).unwrap())
        .is_err());
    assert!(!store
        .entries("s")
        .unwrap()
        .iter()
        .any(|entry| entry["id"] == "m"));
}
