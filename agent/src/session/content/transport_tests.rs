//! Wire-preview and scoped page specifications for [`super::transport`].
//!
//! The preview must stay bounded without losing information, and a scoped page
//! must be resolved against the *committed* semantic reference, not against a
//! body that a concurrent deletion may have recycled.
use super::transport::{prepare_wire_data, INLINE_EVENT_BYTES, INLINE_FIELD_BYTES};
use super::{journal::Writer, *};
use crate::session::sqlite_store::SqliteStore;
use future_rpc::content_range::ContentRequest;
use serde_json::{json, Value};

fn event(idx: i64, kind: &str, data: Value) -> Value {
    json!({"session_id":"s","run_id":"r","epoch":1,"idx":idx,"session_idx":-1,"run_sequence":0,"event_id":format!("s:r:1:{idx}"),"timestamp":"2026-10-26T00:00:00Z","event_type":kind,"data":data.to_string()})
}

fn store() -> (tempfile::TempDir, SqliteStore) {
    let root = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&root.path().join("agent.db")).unwrap();
    store.reserve_segmented("s").unwrap();
    store.replace("s",vec![json!({"id":"info","type":"session_info","timestamp":"2026-10-26T00:00:00Z","content":{"cwd":".","model":"mock"}})]).unwrap();
    (root, store)
}

fn facts(output: &str) -> future_rpc::shell_result::ShellResult {
    future_rpc::shell_result::ShellResult {
        command: "printf hi".into(),
        cwd: "/w".into(),
        duration_ms: 3,
        status: future_rpc::shell_result::ShellStatus::Exited,
        exit_code: Some(0),
        is_soft_fail: false,
        is_error: false,
        attempts: vec![future_rpc::shell_result::ShellAttempt {
            status: future_rpc::shell_result::ShellStatus::Exited,
            exit_code: Some(0),
            duration_ms: 3,
            output: output.into(),
            output_truncated: false,
            escalated: false,
        }],
        approval: None,
        note: None,
    }
}

/// Facts whose body is far above every inline budget, so both the canonical
/// JSON and its summary view are published as references rather than inlined.
fn large_facts() -> future_rpc::shell_result::ShellResult {
    facts(&"界".repeat(120_000))
}

/// A `ContentRequest` for one reference produced by the writer.
fn request_for(reference: &Value) -> ContentRequest {
    serde_json::from_value(reference.clone()).unwrap()
}

/// Publish one `tool_end` carrying canonical facts and return the event bodies
/// the broadcaster would have announced for it.
fn publish_facts(store: &SqliteStore, result: &future_rpc::shell_result::ShellResult) -> Value {
    let summary = future_rpc::shell_result::model_summary(result);
    let mut writer = Writer::default();
    writer
        .persist(
            store,
            "s",
            &[event(
                0,
                "tool_end",
                json!({
                    "tool_id": "t",
                    "shell_result": serde_json::to_value(result).unwrap(),
                    "text": summary,
                }),
            )],
        )
        .unwrap();
    let replay = store
        .segmented_replay("s", "r", -1, false, Some(10))
        .unwrap();
    serde_json::from_str::<Value>(&replay.events[0].data).unwrap()
}

#[test]
fn a_non_object_body_is_forwarded_unchanged() {
    for value in [json!("plain text"), json!(42), json!(null), json!([1, 2])] {
        assert_eq!(prepare_wire_data(&value, "text_chunk"), value);
    }
}

#[test]
fn a_small_object_stays_inline_and_gains_its_verdict() {
    let data = json!({"text": "hi", "exit_code": 0});
    assert_eq!(
        prepare_wire_data(&data, "tool_end"),
        json!({"text":"hi","exit_code":0,"is_error":false})
    );
    assert_eq!(
        prepare_wire_data(&json!({"exit_code": 2}), "tool_result"),
        json!({"exit_code":2,"is_error":true})
    );
    // Only the two tool verdict events carry the derived field.
    assert_eq!(
        prepare_wire_data(&json!({"text": "hi"}), "text_chunk"),
        json!({"text": "hi"})
    );
}

#[test]
fn a_large_field_is_referenced_and_its_preview_keeps_the_shape_of_its_value() {
    let data = json!({
        "text": "你".repeat(INLINE_FIELD_BYTES),
        "usage": {"probe": vec!["x"; INLINE_FIELD_BYTES]},
        "tool_args": {"path": "/tmp/x", "cwd": "/w", "command": "y".repeat(40_000)},
        "exit_code": 0,
    });
    let wire = prepare_wire_data(&data, "provider_fact");
    // A string field previews as empty text, a structured field as null, and
    // tool arguments keep only the path-ish scalars a UI needs to label them.
    assert_eq!(wire["text"], "");
    assert_eq!(wire["usage"], Value::Null);
    assert_eq!(wire["tool_args"], json!({"path":"/tmp/x","cwd":"/w"}));
    assert_eq!(wire["exit_code"], 0);
    assert_eq!(
        wire["content_refs"],
        json!({
            "text": {"field":"/data/text"},
            "usage": {"field":"/data/usage"},
            "tool_args": {"field":"/data/tool_args"},
        })
    );
    // The preview is bounded: no field body was forwarded twice.
    assert!(wire.to_string().len() < INLINE_EVENT_BYTES / 2 + 3 * 64);
}

#[test]
fn path_previews_are_bounded_escaped_and_dropped_when_they_are_not_objects() {
    // A scalar `tool_args` has no path to preview.
    let scalar = prepare_wire_data(&json!({"tool_args": "z".repeat(40_000)}), "tool_start");
    assert_eq!(scalar["tool_args"], json!(""));
    // A path longer than the preview budget is dropped rather than truncated.
    let long = prepare_wire_data(
        &json!({"tool_args": {"path": "/".to_string() + &"a".repeat(4096), "blob": "b".repeat(40_000)}}),
        "tool_start",
    );
    assert_eq!(long["tool_args"], json!({}));
    // JSON-pointer escaping keeps the reference resolvable through RFC 6901.
    let escaped = prepare_wire_data(&json!({"a/b~c": "q".repeat(40_000)}), "provider_fact");
    assert_eq!(
        escaped["content_refs"]["a/b~c"]["field"],
        "/data/a~1b~0c"
    );
}

#[test]
fn the_aggregate_budget_references_the_field_that_crosses_it() {
    // Each field alone fits; together they exceed half of the inline budget.
    let data: Value = (0..12)
        .map(|index| (format!("f{index}"), json!("x".repeat(1400))))
        .collect::<serde_json::Map<_, _>>()
        .into();
    let wire = prepare_wire_data(&data, "provider_fact");
    let references = wire["content_refs"].as_object().unwrap();
    assert!(!references.is_empty());
    // Every field is still announced: referenced fields lose their inline
    // body, but the field itself never silently disappears.
    let mut announced = references
        .keys()
        .chain(wire.as_object().unwrap().keys().filter(|key| *key != "content_refs"))
        .cloned()
        .collect::<Vec<_>>();
    announced.sort();
    announced.dedup();
    assert_eq!(announced.len(), 12);
}

#[test]
fn stamped_references_carry_the_broadcast_cursor_and_page_budget() {
    let identity = future_rpc::content_range::StorageIdentity {
        database_id: "db".into(),
        restore_generation: "gen".into(),
    };
    let mut wire = json!({"content_refs":{"text":{"field":"/data/text"}},"text":""});
    let mut source = json!({"entry_id":"e","content_key":"k"});
    source["content_bytes"] = json!(9);
    super::transport::stamp_wire_data(&mut wire, &source, &identity, 2, 7);
    assert_eq!(
        wire,
        json!({
            "content_refs":{"text":{
                "field":"/data/text","databaseId":"db","restoreGeneration":"gen",
                "epoch":2,"cursor":7,"offset":0,"maxBytes":65536,
            }},
            "text":"",
            "content_format": FORMAT_VERSION,
            "entry_id":"e",
            "content_key":"k",
            "content_bytes":9,
        })
    );
}

#[test]
fn canonical_facts_page_as_json_and_as_the_model_summary() {
    let (_root, store) = store();
    let result = large_facts();
    let bodies = publish_facts(&store, &result);
    let facts_request = request_for(&bodies["content_refs"]["shell_result"]);
    let page = store
        .event_content_range("s", "r", facts_request.clone())
        .unwrap();
    assert_eq!(page["encoding"], "json");
    // The facts body is far larger than one page and is still served whole by
    // the page loop, in order, as the JSON it was stored as.
    assert!(page["totalBytes"].as_i64().unwrap() > READ_BYTES as i64);
    assert_eq!(page["totalBytes"], json!(serde_json::to_string(&result).unwrap().len() as i64));
    assert_eq!(page["hasMore"], json!(true));
    assert_eq!(
        collect(&store, &facts_request),
        serde_json::to_string(&result).unwrap()
    );

    let summary = future_rpc::shell_result::model_summary(&result);
    let summary_request = request_for(&bodies["content_refs"]["text"]);
    let page = store
        .event_content_range("s", "r", summary_request.clone())
        .unwrap();
    assert_eq!(page["encoding"], "text");
    assert_eq!(page["totalBytes"], json!(summary.len() as i64));
    assert_eq!(collect(&store, &summary_request), summary);
}

#[test]
fn the_summary_is_composed_from_every_canonical_page_before_it_is_paged() {
    let (_root, store) = store();
    // The stored facts are larger than one internal read page, so the summary
    // can only be produced by reading all of them; the summary itself is then
    // served in many small pages.
    let result = large_facts();
    let bodies = publish_facts(&store, &result);
    let facts = store
        .event_content_range(
            "s",
            "r",
            request_for(&bodies["content_refs"]["shell_result"]),
        )
        .unwrap();
    assert!(facts["totalBytes"].as_i64().unwrap() > READ_BYTES as i64);
    let summary = future_rpc::shell_result::model_summary(&result);
    let mut request = request_for(&bodies["content_refs"]["text"]);
    request.max_bytes = 7;
    let mut assembled = String::new();
    let mut offset = 0;
    loop {
        request.offset = offset;
        let page = store
            .event_content_range("s", "r", request.clone())
            .unwrap();
        assert!(page["nextOffset"].as_i64().unwrap() > offset);
        assert_eq!(page["totalBytes"].as_i64().unwrap(), summary.len() as i64);
        assembled.push_str(page["text"].as_str().unwrap());
        offset = page["nextOffset"].as_i64().unwrap();
        if page["hasMore"] == json!(false) {
            break;
        }
    }
    assert_eq!(assembled, summary);
    // An offset inside a multi-byte scalar cannot be served.
    request.offset = i64::try_from(
        summary
            .as_bytes()
            .iter()
            .position(|byte| *byte >= 0x80)
            .expect("the summary keeps a multi-byte tail"),
    )
    .unwrap()
        + 1;
    request.max_bytes = 1;
    let error = store.event_content_range("s", "r", request).unwrap_err();
    assert!(
        format!("{error:#}").contains("invalid summary content offset"),
        "{error:#}"
    );
}

/// A body whose committed segments disappeared cannot be summarized: the page
/// loop fails instead of returning a plausible truncated summary.
#[test]
fn a_summary_cannot_be_served_when_its_canonical_pages_disappear() {
    let (_root, store) = store();
    let bodies = publish_facts(&store, &large_facts());
    store
        .db
        .call(|db| {
            db.execute(
                "DELETE FROM content_segments WHERE content_id=(SELECT content_id FROM stream_content_ranges WHERE field='/data/shell_result' LIMIT 1)",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    let error = store
        .event_content_range("s", "r", request_for(&bodies["content_refs"]["text"]))
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("canonical facts range is incomplete")
            || format!("{error:#}").contains("missing")
            || format!("{error:#}").contains("segment"),
        "{error:#}"
    );
}

#[test]
fn a_page_budget_that_cannot_fit_one_character_is_rejected() {
    let (_root, store) = store();
    let bodies = publish_facts(&store, &large_facts());
    let mut request = request_for(&bodies["content_refs"]["text"]);
    // The summary starts with ASCII, so a one-byte page is servable; a
    // zero-progress page is not.
    request.max_bytes = 1;
    assert_eq!(
        store
            .event_content_range("s", "r", request.clone())
            .unwrap()["nextOffset"],
        1
    );
    request.max_bytes = 0;
    assert!(store.event_content_range("s", "r", request.clone()).is_err());
    request.max_bytes = READ_BYTES as i64 + 1;
    assert!(store.event_content_range("s", "r", request.clone()).is_err());
    request.max_bytes = 8;
    request.offset = -1;
    assert!(store.event_content_range("s", "r", request).is_err());
}

#[test]
fn request_validation_rejects_every_out_of_range_scope() {
    let (_root, store) = store();
    let bodies = publish_facts(&store, &large_facts());
    let valid = request_for(&bodies["content_refs"]["text"]);
    let mut broken = valid.clone();
    broken.field = "data/text".into();
    assert!(store.event_content_range("s", "r", broken).is_err());
    let mut broken = valid.clone();
    broken.field = format!("/data/{}", "x".repeat(600));
    assert!(store.event_content_range("s", "r", broken).is_err());
    let mut broken = valid.clone();
    broken.epoch = -2;
    assert!(store.event_content_range("s", "r", broken).is_err());
    assert!(store
        .event_content_range("", "r", valid.clone())
        .is_err());
    // The uncommitted cursor of a live reference cannot be resolved: the
    // journal must be flushed before a reader asks for it.
    let mut broken = valid.clone();
    broken.cursor += 1;
    assert!(store.event_content_range("s", "r", broken).is_err());
    assert!(store.event_content_range("s", "r", valid).is_ok());
}

/// The stream reader refuses an entry-only encoding (`transport.rs`, the
/// `ToolArgumentsStringV1` arm). That refusal is a defensive guard: no writer
/// emits such an encoding for a stream body, and the schema makes the two
/// stored authorities for a range encoding unable to carry one. This test pins
/// both halves of that argument, so the guard can never become the only reason
/// the arm is unreachable *and* wrong.
#[test]
fn an_entry_only_encoding_is_unrepresentable_for_a_stream_range() {
    let (_root, store) = store();
    publish_facts(&store, &large_facts());
    // A published context is immutable, so a stored encoding cannot be
    // rewritten into an entry-only one after the fact.
    let error = store
        .db
        .call(|db| {
            db.execute(
                "UPDATE stream_contexts SET descriptor_json=json_set(descriptor_json,'$[2]','tool_args_string_v1')",
                [],
            )?;
            Ok(())
        })
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("stream contexts are immutable"),
        "{error:#}"
    );
    // The multi-field authority constrains the encoding vocabulary itself.
    let error = store
        .db
        .call(|db| {
            db.execute(
                "INSERT INTO stream_change_content(change_id,field,encoding,content_id,byte_offset,byte_length) VALUES ((SELECT change_id FROM stream_content_ranges LIMIT 1),'/data/other','tool_args_string_v1',(SELECT id FROM content_objects LIMIT 1),0,1)",
                [],
            )?;
            Ok(())
        })
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("CHECK constraint failed"),
        "{error:#}"
    );
    // Every encoding a writer can publish parses back.
    for encoding in ["text", "json", "shell_summary"] {
        assert!(super::stream::Encoding::parse(encoding).is_ok());
    }
    assert!(super::stream::Encoding::parse("tool_args_string_v1").is_ok());
}

#[test]
fn storage_identity_fails_when_the_metadata_rows_are_missing() {
    let (_root, store) = store();
    let identity = store.storage_identity().unwrap();
    assert!(!identity.database_id.is_empty());
    assert!(!identity.restore_generation.is_empty());
    store
        .db
        .call(|db| {
            db.execute("DELETE FROM storage_meta WHERE key='restore_generation'", [])?;
            Ok(())
        })
        .unwrap();
    let error = store.storage_identity().unwrap_err();
    assert!(format!("{error:#}").contains("no rows"), "{error:#}");
    store
        .db
        .call(|db| {
            db.execute("DELETE FROM storage_meta WHERE key='database_id'", [])?;
            Ok(())
        })
        .unwrap();
    assert!(store.storage_identity().is_err());
}

#[test]
fn a_reference_whose_body_was_collected_after_publication_is_not_resolvable() {
    let (_root, store) = store();
    let bodies = publish_facts(&store, &large_facts());
    let request = request_for(&bodies["content_refs"]["text"]);
    // Retiring the stream queues its bodies for collection; the reference is
    // then stale and must not resolve to a recycled object id.
    store
        .db
        .call(|db| {
            db.execute("DELETE FROM stream_runs", [])?;
            Ok(())
        })
        .unwrap();
    assert!(store.event_content_range("s", "r", request).is_err());
}

/// Read every page of one reference: proves the page loop advances and that
/// the assembled text is exactly the stored body.
fn collect(store: &SqliteStore, request: &ContentRequest) -> String {
    let mut request = request.clone();
    let mut text = String::new();
    loop {
        let page = store
            .event_content_range("s", "r", request.clone())
            .unwrap();
        text.push_str(page["text"].as_str().unwrap());
        let total = page["totalBytes"].as_i64().unwrap();
        let next = page["nextOffset"].as_i64().unwrap();
        assert!(
            next > request.offset || next == total,
            "page made no progress"
        );
        if page["hasMore"] == serde_json::json!(false) {
            assert_eq!(next, total);
            return text;
        }
        request.offset = next;
    }
}

/// The manager-level entry forwards to the storage that owns the range, so an
/// RPC handler never has to resolve the store itself.
#[test]
fn a_manager_forwards_scoped_pages_to_its_storage() {
    let root = tempfile::tempdir().unwrap();
    let manager = crate::session::Manager::new(root.path().to_path_buf());
    let store = manager.storage().unwrap().clone();
    store.reserve_segmented("s").unwrap();
    store
        .replace(
            "s",
            vec![json!({"id":"info","type":"session_info","timestamp":"2026-10-26T00:00:00Z","content":{"cwd":".","model":"mock"}})],
        )
        .unwrap();
    let bodies = publish_facts(&store, &large_facts());
    let page = manager
        .event_content_range("s", "r", request_for(&bodies["content_refs"]["text"]))
        .unwrap();
    assert_eq!(page["encoding"], "text");
    assert_eq!(
        page["totalBytes"],
        json!(future_rpc::shell_result::model_summary(&large_facts()).len() as i64)
    );
}
