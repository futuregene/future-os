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

/// A tool call whose arguments arrived as an interrupted *string* keeps that
/// shape: the published reference uses the entry-only encoding, and the entry
/// reads back the same string instead of a reparsed object.
#[test]
fn streamed_string_arguments_are_published_with_the_entry_only_encoding() {
    let (_root, store) = store();
    let raw = r#"{"command":"printf hi"}"#;
    let mut writer = Writer::default();
    writer
        .persist(
            &store,
            "s",
            &[
                event(0, "message_start", json!({"entry_id": "m"})),
                event(1, "tool_start", json!({"entry_id":"m","phase":"input","tool_id":"t","tool_name":"shell","tc_index":0,"content_key":"m:1:tool:0","content_offset":0,"content_bytes":0,"tool_args":""})),
                event(2, "tool_delta", json!({"entry_id":"m","tool_id":"t","tc_index":0,"content_key":"m:1:tool:0","content_offset":0,"content_bytes":raw.len(),"text":raw,"snapshot":false})),
                event(3, "tool_input_end", json!({"entry_id":"m","tool_id":"t","tool_name":"shell","tc_index":0,"content_key":"m:1:tool:0","content_offset":raw.len(),"content_bytes":0,"content_seal":true,"tool_args":raw,"provider_metadata":{}})),
                event(4, "message_end", json!({"entry_id":"m","complete":true})),
            ],
        )
        .unwrap();
    let message = message(
        vec![ContentBlock::tool_call(
            "t",
            "shell",
            json!(raw),
            Default::default(),
        )],
        json!([null]),
    );
    let published = StreamEntry::from_assistant(&message, "r", 1).unwrap();
    store.append_stream_entry("s", published).unwrap();

    store
        .db
        .read(|db| {
            let mut statement = db.prepare(
                "SELECT field,encoding FROM content_references WHERE session_id='s' AND owner_kind='entry' AND owner_id='m' ORDER BY field",
            )?;
            let rows = statement
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            assert_eq!(
                rows,
                vec![("/content/0/args".to_string(), "tool_args_string_v1".to_string())],
                "string arguments keep their entry-only encoding"
            );
            Ok(())
        })
        .unwrap();

    let entries = store.entries("s").unwrap();
    let call = entries
        .iter()
        .find(|entry| entry["type"] == "assistant")
        .unwrap_or_else(|| panic!("no assistant entry: {entries:?}"));
    assert_eq!(
        call["content"][0]["args"],
        json!(raw),
        "the interrupted string is not reparsed into an object"
    );
}

/// A tool call whose input never sealed (the run died mid-argument) publishes
/// from the last committed *start* body instead of a boundary, and keeps the
/// entry-only encoding for string arguments.
#[test]
fn an_unsealed_tool_input_publishes_from_its_last_committed_body() {
    let (_root, store) = store();
    let raw = r#"{"command":"printf par"#;
    let mut writer = Writer::default();
    writer
        .persist(
            &store,
            "s",
            &[
                event(0, "message_start", json!({"entry_id": "m"})),
                event(1, "tool_start", json!({"entry_id":"m","phase":"input","tool_id":"t","tool_name":"shell","tc_index":0,"content_key":"m:1:tool:0","content_offset":0,"content_bytes":0,"tool_args":""})),
                event(2, "tool_delta", json!({"entry_id":"m","tool_id":"t","tc_index":0,"content_key":"m:1:tool:0","content_offset":0,"content_bytes":raw.len(),"text":raw,"snapshot":false})),
                event(3, "message_end", json!({"entry_id":"m","complete":true})),
            ],
        )
        .unwrap();
    let message = message(
        vec![ContentBlock::tool_call(
            "t",
            "shell",
            json!(raw),
            Default::default(),
        )],
        json!([null]),
    );
    let published = StreamEntry::from_assistant(&message, "r", 1).unwrap();
    store.append_stream_entry("s", published).unwrap();

    store
        .db
        .read(|db| {
            let mut statement = db.prepare(
                "SELECT field,encoding FROM content_references WHERE session_id='s' AND owner_kind='entry' AND owner_id='m' ORDER BY field",
            )?;
            let rows = statement
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            assert_eq!(
                rows,
                vec![("/content/0/args".to_string(), "tool_args_string_v1".to_string())],
                "the interrupted string keeps its entry-only encoding"
            );
            Ok(())
        })
        .unwrap();
    let entries = store.entries("s").unwrap();
    let call = entries
        .iter()
        .find(|entry| entry["type"] == "assistant")
        .unwrap_or_else(|| panic!("no assistant entry: {entries:?}"));
    // The truncated arguments stay a *string* (they are never reparsed into an
    // object), repaired just enough to parse.
    let args = call["content"][0]["args"].as_str().unwrap_or_else(|| {
        panic!(
            "arguments must stay a string: {}",
            call["content"][0]["args"]
        )
    });
    assert!(args.starts_with(r#"{"command":"printf par"#), "{args}");
    assert!(
        serde_json::from_str::<Value>(args).is_ok(),
        "the interrupted input is repaired to parseable JSON: {args}"
    );
}
