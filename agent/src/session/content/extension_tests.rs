//! Authored for the final acceptance pass; no tests during implementation.
use super::*;
use crate::session::sqlite_store::SqliteStore;
use serde_json::{json, Value};

fn store() -> (tempfile::TempDir, SqliteStore) {
    let root = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&root.path().join("agent.db")).unwrap();
    store.reserve_segmented("s").unwrap();
    store.replace("s",vec![json!({"id":"info","type":"session_info","timestamp":"2026-10-26T00:00:00Z","content":{"model":"mock"}})]).unwrap();
    (root, store)
}

#[test]
fn large_protocol_extensions_images_and_unknown_blocks_round_trip_without_large_headers() {
    let (_root, store) = store();
    let body = "协议/图片~正文".repeat(5_000);
    let blocks = json!([
        {"type":"reasoning","text":"思考","signature":body,"provider_metadata":{"vendor/opaque~key":{"data":body,"kind":"signed"}}},
        {"type":"image","data":body,"mime_type":"image/png"},
        {"type":"image_url","image_url":{"url":format!("data:image/png;base64,{body}")}},
        {"type":"future_block","text":{"opaque":body},"args":{"nested":[body]},"x/y~z":[body]},
        body,
    ]);
    let entry = json!({"id":"a","type":"assistant","role":"assistant","timestamp":"2026-10-26T00:00:01Z",
        "meta":{"run_id":"r","attachments":[{"data":body}],"vendor/opaque~key":{"state":body}},"content":blocks});
    store.append("s", vec![entry.clone()]).unwrap();
    let actual = store
        .entries("s")
        .unwrap()
        .into_iter()
        .find(|entry| entry["id"] == "a")
        .unwrap();
    assert_eq!(actual["content"], entry["content"]);
    assert_eq!(actual["meta"], entry["meta"]);
    store.db.read(|db| {
        let max:i64 = db.query_row("SELECT max(length(CAST(metadata_json AS BLOB))) FROM message_blocks WHERE session_id='s'",[],|row|row.get(0))?;
        assert!(max < 4096);
        let max:i64 = db.query_row("SELECT max(length(CAST(metadata_json AS BLOB))) FROM entries WHERE session_id='s'",[],|row|row.get(0))?;
        assert!(max < 4096);
        let max:i64 = db.query_row("SELECT max(length(CAST(body AS BLOB))) FROM content_segments",[],|row|row.get(0))?;
        assert!(max <= SEGMENT_BYTES as i64);
        Ok(())
    }).unwrap();
}

#[test]
fn a_large_terminal_error_has_one_body_shared_by_history_and_replay() {
    let (_root, store) = store();
    let error = "错误原因；".repeat(10_000);
    store.append("s",vec![json!({"id":"start","type":"run_started","timestamp":"2026-10-26T00:00:00Z","content":{"run_id":"r","epoch":1}})]).unwrap();
    store.commit_terminal_entries("s",vec![json!({"id":"end","type":"run_terminal","timestamp":"2026-10-26T00:00:01Z","content":{"run_id":"r","state":"error","error":error}})]).unwrap();
    let entries = store.entries("s").unwrap();
    assert_eq!(entries.last().unwrap()["content"]["error"], error);
    let replay = store
        .segmented_replay("s", "r", -1, false, Some(10))
        .unwrap();
    assert_eq!(replay.events.len(), 1);
    let data: Value = serde_json::from_str(&replay.events[0].data).unwrap();
    assert!(data["content_refs"]["error"].is_object());
    let mut request: future_rpc::content_range::ContentRequest =
        serde_json::from_value(data["content_refs"]["error"].clone()).unwrap();
    let mut body = String::new();
    loop {
        let page = store
            .event_content_range("s", "r", request.clone())
            .unwrap();
        body.push_str(page["text"].as_str().unwrap());
        if page["hasMore"] == false {
            break;
        }
        request.offset = page["nextOffset"].as_i64().unwrap();
    }
    assert_eq!(body, error);
    store.db.read(|db| {
        let owners:(i64,i64) = db.query_row("SELECT r.content_id,b.content_id FROM content_references r JOIN stream_change_content b ON b.field='/data/error' WHERE r.session_id='s' AND r.owner_id='end' AND r.field='/content/error'",[],|row|Ok((row.get(0)?,row.get(1)?)))?;
        assert_eq!(owners.0,owners.1);
        let count:i64 = db.query_row("SELECT count(*) FROM content_objects",[],|row|row.get(0))?;
        assert_eq!(count,1);
        Ok(())
    }).unwrap();
}
