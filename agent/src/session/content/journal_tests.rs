//! Canonical writer integration specifications. Executed in the final pass.
use super::{journal::Writer, stream::*, *};
use crate::session::sqlite_store::SqliteStore;
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

#[test]
fn metadata_byte_admission_counts_escaping_unicode_and_numeric_boundaries() {
    let controls: String = (0u8..=31).map(char::from).collect();
    for value in [
        json!({"escaped\"key\\":controls,"unicode":"思考🙂\u{2028}","empty":[],"nil":null,"flags":[true,false]}),
        json!([i64::MIN, i64::MAX, u64::MAX, 0, -1, 1.0, 1e-200, 1e200]),
        json!({"text":"\"\\\n🙂".repeat(4096),"nested":[{"blocks":[{},[],"",null]}]}),
    ] {
        assert_eq!(
            super::journal::json_bytes(&value),
            serde_json::to_vec(&value).unwrap().len()
        );
    }
}

#[test]
fn bounded_facts_and_seal_share_one_commit_and_retry_after_atomic_failure() {
    let (_root, store) = store();
    let events = [
        event(
            0,
            "thinking_delta",
            json!({"entry_id":"a","content_key":"a:thinking:0","content_offset":0,"content_bytes":3,"text":"one","extension":false}),
        ),
        event(
            1,
            "thinking_end",
            json!({"entry_id":"a","content_key":"a:thinking:0","content_offset":3,"content_bytes":0,"content_seal":true}),
        ),
        event(2, "usage", json!({"output_tokens":1})),
    ];
    let mut writer = Writer::default();
    store.db.call(|db| {
        db.execute_batch("CREATE TRIGGER reject_usage BEFORE INSERT ON stream_changes WHEN NEW.event_kind=16 BEGIN SELECT RAISE(ABORT,'injected'); END;")?;
        Ok(())
    }).unwrap();
    assert!(writer.persist(&store, "s", &events).is_err());
    store
        .db
        .read(|db| {
            for table in [
                "stream_changes",
                "stream_contexts",
                "stream_symbols",
                "stream_body_owners",
                "content_objects",
            ] {
                assert_eq!(
                    db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                        .get::<_, i64>(0))?,
                    0
                );
            }
            Ok(())
        })
        .unwrap();
    store
        .db
        .call(|db| {
            db.execute_batch("DROP TRIGGER reject_usage")?;
            Ok(())
        })
        .unwrap();
    let commits = store.db.metrics()["foregroundCommit"]["count"]
        .as_u64()
        .unwrap();
    writer.persist(&store, "s", &events).unwrap();
    assert_eq!(
        store.db.metrics()["foregroundCommit"]["count"]
            .as_u64()
            .unwrap()
            - commits,
        1
    );
    let replay = store.segmented_replay("s", "r", -1, false, None).unwrap();
    assert_eq!(replay.events.len(), 3);
    assert_eq!(replay.events[0].event_type, "thinking_delta");
    assert_eq!(replay.events[1].event_type, "thinking_end");
    assert_eq!(replay.events[2].event_type, "usage");
    let data: Value = serde_json::from_str(&replay.events[0].data).unwrap();
    assert_eq!(data["text"], "one");
    assert_eq!(data["extension"], false);
}

#[test]
fn oversized_fragment_flushes_its_pending_prefix_before_staging() {
    let (_root, store) = store();
    let large = "x".repeat(WRITE_BYTES + 7);
    let mut writer = Writer::default();
    writer.persist(&store, "s", &[
        event(0, "text_chunk", json!({"content_key":"b","content_offset":0,"content_bytes":1,"text":"a"})),
        event(1, "text_chunk", json!({"content_key":"b","content_offset":1,"content_bytes":large.len(),"text":large})),
        event(2, "text_end", json!({"content_key":"b","content_offset":1+large.len(),"content_bytes":0,"content_seal":true})),
    ]).unwrap();
    let scope = StreamIdentity {
        session_id: "s".into(),
        run_id: "r".into(),
        epoch: 1,
    };
    let range = store
        .db
        .read(move |db| {
            let facts = read_changes(db, &scope, -1, 10)?;
            assert_eq!(facts.changes.len(), 3);
            assert_eq!(facts.changes[2].event_type, "text_end");
            Ok(super::resolve(db, "s", "stream", &owner(&scope, "b"), "")?.unwrap())
        })
        .unwrap();
    assert_eq!(
        super::journal::read_range(&store, range).unwrap(),
        format!("a{large}")
    );
}

#[test]
fn explicit_terminal_commit_is_atomic_with_replay_and_retries_without_duplicates() {
    let (_root, store) = store();
    store.append("s", vec![json!({"id":"start","type":"run_started","timestamp":"2026-10-26T00:00:00Z","content":{"run_id":"r","epoch":1,"run_sequence":4}})]).unwrap();
    let mut writer = Writer::default();
    writer
        .persist(&store, "s", &[event(0, "usage", json!({"input_tokens":1}))])
        .unwrap();
    let terminal = json!({"id":"end","type":"run_terminal","timestamp":"2026-10-26T00:00:01Z","content":{"run_id":"r","state":"completed","run_tokens":17,"run_duration_ms":42}});
    store.db.call(|db| {db.execute_batch("CREATE TRIGGER reject_end BEFORE INSERT ON stream_changes WHEN NEW.event_type='agent_end' BEGIN SELECT RAISE(ABORT,'injected'); END;")?;Ok(())}).unwrap();
    assert!(store
        .commit_terminal_entries("s", vec![terminal.clone()])
        .is_err());
    assert!(!store
        .entries("s")
        .unwrap()
        .iter()
        .any(|entry| entry["type"] == "run_terminal"));
    assert_eq!(
        store
            .segmented_replay("s", "r", -1, false, None)
            .unwrap()
            .events
            .len(),
        1
    );
    store
        .db
        .call(|db| {
            db.execute_batch("DROP TRIGGER reject_end")?;
            Ok(())
        })
        .unwrap();
    store
        .commit_terminal_entries("s", vec![terminal.clone()])
        .unwrap();
    store.commit_terminal_entries("s", vec![terminal]).unwrap();
    // No live publisher ran. A restart/reconnect still observes the same end.
    store.recover_segmented_runs().unwrap();
    let replay = store.segmented_replay("s", "r", -1, false, None).unwrap();
    assert_eq!(replay.events.len(), 2);
    let end = &replay.events[1];
    assert_eq!(end.idx, 1);
    assert_eq!(end.event_type, "agent_end");
    assert_eq!(end.run_sequence, 4);
    let data: Value = serde_json::from_str(&end.data).unwrap();
    assert_eq!(data["state"], "completed");
    assert_eq!(data["usage"]["output_tokens"], 17);
    assert_eq!(data["duration_ms"], 42);
    assert!(writer
        .persist(
            &store,
            "s",
            &[event(
                2,
                "text_chunk",
                json!({"text":"late","content_key":"late","content_offset":0,"content_bytes":4})
            )]
        )
        .is_err());
    assert_eq!(
        store
            .segmented_replay("s", "r", -1, false, None)
            .unwrap()
            .events
            .len(),
        2
    );
}

#[test]
fn canonical_pricing_reads_usage_and_model_changes_without_raw_event_copies() {
    let (_root, store) = store();
    let mut writer = Writer::default();
    writer
        .persist(
            &store,
            "s",
            &[
                event(0, "model_changed", json!({"model":"mock/new"})),
                event(1, "usage", json!({"input_tokens":5,"output_tokens":7})),
                event(
                    2,
                    "text_chunk",
                    json!({"text":"body","content_key":"b","content_offset":0,"content_bytes":4}),
                ),
            ],
        )
        .unwrap();
    let rows: Vec<Value> = store
        .pricing_event_payloads("s")
        .unwrap()
        .into_iter()
        .map(|raw| serde_json::from_str(&raw).unwrap())
        .collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["event_type"], "model_changed");
    assert_eq!(rows[0]["data"]["model"], "mock/new");
    assert_eq!(rows[1]["data"]["output_tokens"], 7);
    let raw: i64 = store
        .db
        .read(|db| {
            Ok(db.query_row(
                "SELECT count(*) FROM run_events WHERE session_id='s'",
                [],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(raw, 0);
}

#[test]
fn recovery_shares_large_provider_state_instead_of_copying_it_into_a_header() {
    let (_root, store) = store();
    let provider = json!({"anthropic":{"signature":"签名".repeat(20_000),"opaque":{"data":"opaque".repeat(8_000)}}});
    store.append("s",vec![json!({"id":"start","type":"run_started","timestamp":"2026-10-26T00:00:00Z","content":{"run_id":"r","epoch":1}})]).unwrap();
    let mut writer = Writer::default();
    writer.persist(&store,"s",&[
        event(0,"message_start",json!({"entry_id":"m"})),
        event(1,"thinking_start",json!({"entry_id":"m","block_id":"b","content_key":"b","content_offset":0,"content_bytes":0})),
        event(2,"thinking_delta",json!({"entry_id":"m","block_id":"b","content_key":"b","content_offset":0,"content_bytes":3,"text":"思"})),
        event(3,"thinking_end",json!({"entry_id":"m","block_id":"b","content_key":"b","content_offset":3,"content_bytes":0,"content_seal":true,"provider_metadata":provider})),
        event(4,"message_end",json!({"entry_id":"m","complete":true})),
    ]).unwrap();
    store.recover_segmented_runs().unwrap();
    let recovered = store
        .entries("s")
        .unwrap()
        .into_iter()
        .find(|entry| entry["id"] == "m")
        .unwrap();
    assert_eq!(recovered["content"][0]["provider_metadata"], provider);
    store.db.read(|db| {
        let (entry,stream):(i64,i64) = db.query_row("SELECT r.content_id,b.content_id FROM content_references r JOIN stream_content_ranges b ON b.field='/data/provider_metadata' WHERE r.session_id='s' AND r.owner_id='m' AND r.field='/content/0/provider_metadata'",[],|row|Ok((row.get(0)?,row.get(1)?)))?;
        assert_eq!(entry,stream);
        let bytes:i64 = db.query_row("SELECT length(metadata_json) FROM message_blocks WHERE session_id='s' AND entry_position=(SELECT position FROM entries WHERE session_id='s' AND entry_id='m') AND ordinal=0",[],|row|row.get(0))?;
        assert!(bytes < 4096);
        Ok(())
    }).unwrap();
}
#[test]
fn consecutive_fragments_publish_a_single_range_and_reuse_the_sealed_transcript_body() {
    let (_root, store) = store();
    let events = vec![
        event(
            0,
            "text_start",
            json!({"block_id":"b","content_key":"b","content_offset":0,"content_bytes":0,"content_first_idx":0}),
        ),
        event(
            1,
            "text_chunk",
            json!({"block_id":"b","content_key":"b","content_offset":0,"content_bytes":3,"content_first_idx":1,"text":"你"}),
        ),
        event(
            2,
            "text_chunk",
            json!({"block_id":"b","content_key":"b","content_offset":3,"content_bytes":3,"content_first_idx":2,"text":"好"}),
        ),
        event(
            3,
            "text_end",
            json!({"block_id":"b","content_key":"b","content_offset":6,"content_bytes":0,"content_first_idx":3,"content_seal":true}),
        ),
    ];
    let mut writer = Writer::default();
    writer.persist(&store, "s", &events).unwrap();
    // A replayed write batch is distinct from a transport replay; a caller
    // retries it from the last uncommitted writer state, never from an already
    // advanced producer. Low-level committed range retries are specified in stream.rs.
    let identity = StreamIdentity {
        session_id: "s".into(),
        run_id: "r".into(),
        epoch: 1,
    };
    let page = store
        .db
        .read(move |db| read_changes(db, &identity, -1, 10))
        .unwrap();
    assert_eq!(page.durable_cursor, 3);
    assert_eq!(page.changes.len(), 3);
    assert_eq!(
        (page.changes[1].first_cursor, page.changes[1].last_cursor),
        (1, 2)
    );
    let body = page.changes[1].bodies[0].1.id;
    store.commit("s",vec![json!({"id":"answer","type":"assistant","role":"assistant","timestamp":"2026-10-26T00:00:00Z","meta":{"run_id":"r"},"content":[{"type":"text","text":"你好","provider_metadata":{"signature":"opaque"}}]})],vec![]).unwrap();
    let referenced = store
        .db
        .read(move |db| resolve(db, "s", "entry", "answer", "/content/0/text"))
        .unwrap()
        .unwrap();
    assert_eq!(referenced.id, body);
    let replay = store
        .segmented_replay("s", "r", 1, false, Some(10))
        .unwrap();
    assert_eq!(replay.events[0].idx, 2);
    let data: Value = serde_json::from_str(&replay.events[0].data).unwrap();
    assert_eq!(data["text"], "你好");
    assert_eq!(data["content_first_idx"], 1);
    let entries = store.entries("s").unwrap();
    assert_eq!(entries[1]["content"][0]["text"], "你好");
    assert_eq!(
        entries[1]["content"][0]["provider_metadata"]["signature"],
        "opaque"
    );
    let raw: i64 = store
        .db
        .read(|db| Ok(db.query_row("SELECT count(*) FROM run_events", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(raw, 0);
}

#[test]
fn shell_stream_and_entry_share_facts_and_derive_the_model_summary() {
    let (_root, store) = store();
    let facts = future_rpc::shell_result::ShellResult {
        command: "printf retained".into(),
        cwd: ".".into(),
        duration_ms: 1,
        status: future_rpc::shell_result::ShellStatus::Exited,
        exit_code: Some(0),
        is_soft_fail: false,
        is_error: false,
        attempts: vec![future_rpc::shell_result::ShellAttempt {
            status: future_rpc::shell_result::ShellStatus::Exited,
            exit_code: Some(0),
            duration_ms: 1,
            output: "retained output".into(),
            output_truncated: false,
            escalated: false,
        }],
        approval: None,
        note: None,
    };
    let summary = future_rpc::shell_result::model_summary(&facts);
    let mut writer = Writer::default();
    writer
        .persist(
            &store,
            "s",
            &[event(
                0,
                "tool_end",
                json!({"tool_id":"t","tool_name":"shell","text":summary,"shell_result":facts}),
            )],
        )
        .unwrap();
    store.commit("s",vec![json!({"id":"result","type":"tool","role":"tool","timestamp":"2026-10-26T00:00:00Z","meta":{"run_id":"r","shell_result":facts},"content":[{"type":"tool_result","tool_call_id":"t","content":summary}]})],vec![]).unwrap();
    let entries = store.entries("s").unwrap();
    assert_eq!(
        entries[1]["meta"]["shell_result"],
        serde_json::to_value(&facts).unwrap()
    );
    assert_eq!(entries[1]["content"][0]["content"], summary);
    let count: i64 = store
        .db
        .read(|db| Ok(db.query_row("SELECT count(*) FROM content_objects", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(count, 1);
    let replay = store.segmented_replay("s", "r", -1, false, None).unwrap();
    let data: Value = serde_json::from_str(&replay.events[0].data).unwrap();
    assert_eq!(data["text"], summary);
    assert_eq!(data["shell_result"], serde_json::to_value(facts).unwrap());
}

#[test]
fn startup_recovers_only_the_published_prefix_into_the_same_message_identity() {
    let (_root, store) = store();
    store.commit("s",vec![json!({"id":"started","type":"run_started","role":"system","timestamp":"2026-10-26T00:00:00Z","content":{"run_id":"r","run_sequence":1,"epoch":1}})],vec![]).unwrap();
    let mut writer = Writer::default();
    writer.persist(&store,"s",&[
        event(0,"message_start",json!({"entry_id":"message"})),
        event(1,"text_start",json!({"entry_id":"message","block_id":"b","content_key":"message:text:b","content_offset":0,"content_bytes":0,"content_first_idx":1})),
        event(2,"text_chunk",json!({"entry_id":"message","block_id":"b","content_key":"message:text:b","content_offset":0,"content_bytes":6,"content_first_idx":2,"text":"保留"})),
    ]).unwrap();
    // A large event may have staged more bytes without publishing its change.
    // Those bytes are not covered by the durable semantic watermark.
    let identity = StreamIdentity {
        session_id: "s".into(),
        run_id: "r".into(),
        epoch: 1,
    };
    store
        .db
        .call(move |db| {
            commit_batch(
                db,
                &identity,
                vec![ContentAppend {
                    key: "message:text:b".into(),
                    offset: 6,
                    text: "未发布".into(),
                    seal: None,
                }],
                vec![],
            )
        })
        .unwrap();
    store.recover_segmented_runs().unwrap();
    store.recover_segmented_runs().unwrap();
    let entries = store.entries("s").unwrap();
    let recovered = entries
        .iter()
        .filter(|entry| entry["id"] == "message")
        .collect::<Vec<_>>();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0]["content"][0]["text"], "保留");
    assert_eq!(recovered[0]["meta"]["recovered_partial"], true);
    let terminal = entries
        .iter()
        .find(|entry| entry["type"] == "run_terminal")
        .unwrap();
    assert_eq!(
        terminal["content"]["state"],
        crate::session::RUN_STATE_INTERRUPTED_BY_RESTART
    );
    let replay = store.segmented_replay("s", "r", -1, false, None).unwrap();
    assert_eq!(replay.events.last().unwrap().event_type, "agent_end");
}

#[test]
fn json_identity_ignores_whitespace_and_nested_object_key_order() {
    let first: Value = serde_json::from_str(r#"{"z":1,"a":{"second":2,"first":1}}"#).unwrap();
    let second: Value =
        serde_json::from_str(r#"{ "a": { "first":1, "second":2 }, "z":1 }"#).unwrap();
    assert_eq!(json_digest(&first).unwrap(), json_digest(&second).unwrap());
    let (_root, store) = store();
    let stream_owner = super::stream::owner(
        &StreamIdentity {
            session_id: "s".into(),
            run_id: "r".into(),
            epoch: 1,
        },
        "json-a",
    );
    let a = entries::stage_named(
        &store,
        "s",
        "stream",
        &stream_owner,
        "",
        &first.to_string(),
        true,
    )
    .unwrap();
    let b = entries::stage_named(
        &store,
        "s",
        "entry",
        "json-b",
        "/content/0/args",
        &second.to_string(),
        true,
    )
    .unwrap();
    assert_eq!(a.id, b.id);
}

#[test]
fn large_facts_replay_as_scoped_pages_without_hydrating_the_body() {
    let (_root, store) = store();
    let body = "你a".repeat(100_000);
    let mut writer = Writer::default();
    writer
        .persist(
            &store,
            "s",
            &[event(
                0,
                "tool_end",
                json!({"tool_id":"t", "text":body, "exit_code":1}),
            )],
        )
        .unwrap();
    let replay = store
        .segmented_replay("s", "r", -1, false, Some(10))
        .unwrap();
    let data: Value = serde_json::from_str(&replay.events[0].data).unwrap();
    assert!(data.to_string().len() < super::transport::INLINE_EVENT_BYTES);
    assert_eq!(data["text"], "");
    assert_eq!(data["is_error"], true);
    let mut request: future_rpc::content_range::ContentRequest =
        serde_json::from_value(data["content_refs"]["text"].clone()).unwrap();
    request.max_bytes = 3;
    let page = store
        .event_content_range("s", "r", request.clone())
        .unwrap();
    assert_eq!(page["text"], "你");
    assert_eq!(page["nextOffset"], 3);
    assert_eq!(page["totalBytes"], body.len());
    assert_eq!(page["hasMore"], true);
    request.offset = 3;
    assert_eq!(
        store
            .event_content_range("s", "r", request.clone())
            .unwrap()["text"],
        "a"
    );
    request.offset = 1;
    assert!(store
        .event_content_range("s", "r", request.clone())
        .is_err());
    request.offset = 0;
    assert!(store
        .event_content_range("s", "wrong-run", request.clone())
        .is_err());
    assert!(store
        .event_content_range("wrong-session", "r", request.clone())
        .is_err());
    request.restore_generation.push('x');
    assert!(store.event_content_range("s", "r", request).is_err());
}

#[test]
fn a_published_prefix_page_etag_survives_later_appends_and_sealing() {
    let (_root, store) = store();
    let mut writer = Writer::default();
    writer.persist(&store,"s",&[event(0,"text_chunk",json!({"text":"你","content_key":"b","content_offset":0,"content_bytes":3,"content_first_idx":0}))]).unwrap();
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
    let before = store
        .event_content_range("s", "r", request.clone())
        .unwrap();
    writer.persist(&store,"s",&[
        event(1,"text_chunk",json!({"text":"好","content_key":"b","content_offset":3,"content_bytes":3,"content_first_idx":1})),
        event(2,"text_end",json!({"content_key":"b","content_offset":6,"content_bytes":0,"content_first_idx":2,"content_seal":true})),
    ]).unwrap();
    let after = store.event_content_range("s", "r", request).unwrap();
    assert_eq!(before, after);
    assert_eq!(after["text"], "你");
    assert_eq!(after["totalBytes"], 3);
}

#[test]
fn aggregate_wire_budget_fields_are_persisted_even_below_the_individual_threshold() {
    let (_root, store) = store();
    let source: Value = (0..12)
        .map(|index| (format!("field{index}"), json!("x".repeat(1500))))
        .collect::<serde_json::Map<_, _>>()
        .into();
    let wire = super::transport::prepare_wire_data(&source, "provider_fact");
    let fields = wire["content_refs"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert!(!fields.is_empty());
    let mut raw = event(0, "provider_fact", source);
    raw["content_fields"] = json!(fields);
    let mut writer = Writer::default();
    writer.persist(&store, "s", &[raw]).unwrap();
    let storage = store.storage_identity().unwrap();
    for field in fields {
        let page = store
            .event_content_range(
                "s",
                "r",
                future_rpc::content_range::ContentRequest {
                    database_id: storage.database_id.clone(),
                    restore_generation: storage.restore_generation.clone(),
                    epoch: 1,
                    cursor: 0,
                    field: format!("/data/{field}"),
                    offset: 0,
                    max_bytes: 2000,
                },
            )
            .unwrap();
        assert_eq!(page["text"], "x".repeat(1500));
    }
}

#[test]
fn canonical_replay_enforces_byte_budget_before_constructing_a_whole_run() {
    let (_root, store) = store();
    let mut writer = Writer::default();
    let fragment = "x".repeat(super::WRITE_BYTES);
    for cursor in 0..140 {
        writer
            .persist(
                &store,
                "s",
                &[event(
                    cursor,
                    "text_chunk",
                    json!({
                        "content_key":"long","content_offset":cursor*super::WRITE_BYTES as i64,
                        "content_bytes":fragment.len(),"content_first_idx":cursor,"text":fragment,
                    }),
                )],
            )
            .unwrap();
    }
    let first = store
        .segmented_replay("s", "r", -1, false, Some(50_001))
        .unwrap();
    let bytes: usize = first
        .events
        .iter()
        .map(|event| event.data.len() + super::transport::EVENT_WIRE_OVERHEAD)
        .sum();
    assert!(bytes > super::transport::REPLAY_PAGE_BYTES);
    assert!(first.events.len() < 140);
    assert!(bytes < super::transport::REPLAY_PAGE_BYTES + super::WRITE_BYTES * 2);
    let last = first.events.last().unwrap().idx;
    let next = store
        .segmented_replay("s", "r", last - 1, false, Some(50_001))
        .unwrap();
    assert_eq!(next.events[0].idx, last);
    assert_eq!(next.events.last().unwrap().idx, 139);
    assert!(store.segmented_replay("s", "r", -1, false, None).is_err());
}

#[test]
fn replay_reads_all_inline_bodies_in_one_bounded_job() {
    let (_root, store) = store();
    let mut writer = Writer::default();
    let fragment = "x".repeat(128);
    for cursor in 0..64 {
        writer
            .persist(
                &store,
                "s",
                &[event(
                    cursor,
                    "text_chunk",
                    json!({
                        "content_key":"short","content_offset":cursor*128,
                        "content_bytes":fragment.len(),"content_first_idx":cursor,"text":fragment,
                    }),
                )],
            )
            .unwrap();
    }
    let before = store.db.metrics()["readerJob"]["count"].as_u64().unwrap();
    let replay = store
        .segmented_replay("s", "r", -1, false, Some(1_000))
        .unwrap();
    let after = store.db.metrics()["readerJob"]["count"].as_u64().unwrap();
    assert_eq!(replay.events.len(), 64);
    assert!(
        after - before <= 5,
        "64 inline replay events used {} read jobs",
        after - before
    );
}

fn start_recoverable_run(store: &SqliteStore) {
    store.commit("s",vec![json!({"id":"started","type":"run_started","role":"system","timestamp":"2026-10-26T00:00:00Z","content":{"run_id":"r","run_sequence":1,"epoch":1}})],vec![]).unwrap();
}

#[test]
fn startup_recovery_preserves_user_stop_over_restart_and_repeats_idempotently() {
    let (_root, store) = store();
    start_recoverable_run(&store);
    let stop = crate::session::SessionEntry::run_stop_requested("r");
    store
        .commit("s", vec![serde_json::to_value(stop).unwrap()], vec![])
        .unwrap();
    let mut writer = Writer::default();
    writer.persist(&store,"s",&[
        event(0,"message_start",json!({"entry_id":"m"})),
        event(1,"text_chunk",json!({"entry_id":"m","content_key":"m:text:b","content_offset":0,"content_bytes":6,"content_first_idx":1,"text":"保留"})),
    ]).unwrap();
    store.recover_segmented_runs().unwrap();
    let entries = store.entries("s").unwrap();
    let terminal = entries
        .iter()
        .find(|entry| entry["type"] == "run_terminal")
        .unwrap();
    assert_eq!(
        terminal["content"]["state"],
        crate::session::RUN_STATE_CANCELLED
    );
    assert_eq!(terminal["content"]["interruption_cause"], "user_stop");
    let events = store
        .segmented_replay("s", "r", -1, false, Some(20))
        .unwrap()
        .events;
    let tail: Value = serde_json::from_str(&events.last().unwrap().data).unwrap();
    assert_eq!(tail["state"], crate::session::RUN_STATE_CANCELLED);
    assert_eq!(tail["interruption_cause"], "user_stop");
    store.recover_segmented_runs().unwrap();
    assert_eq!(store.entries("s").unwrap(), entries);
}

#[test]
fn incomplete_tool_arguments_survive_as_text_and_are_never_recovered_as_executable_calls() {
    let (_root, store) = store();
    start_recoverable_run(&store);
    let partial = r#"{"path":"unfinished"#;
    let mut writer = Writer::default();
    writer.persist(&store,"s",&[
        event(0,"message_start",json!({"entry_id":"m"})),
        event(1,"tool_start",json!({"entry_id":"m","tool_id":"tool","tool_name":"write","phase":"input","tc_index":0,"tool_args":"","content_key":"m:1:tool:0","content_offset":0,"content_bytes":0,"content_first_idx":1})),
        event(2,"tool_delta",json!({"entry_id":"m","tc_index":0,"text":partial,"content_key":"m:1:tool:0","content_offset":0,"content_bytes":partial.len(),"content_first_idx":2})),
    ]).unwrap();
    store.recover_segmented_runs().unwrap();
    let entries = store.entries("s").unwrap();
    let message = entries.iter().find(|entry| entry["id"] == "m").unwrap();
    assert_eq!(message["content"][0]["type"], "text");
    assert_eq!(message["content"][0]["text"], partial);
    assert_eq!(message["meta"]["recovered_incomplete_tool_input"], true);
    assert!(!message["content"]
        .as_array()
        .unwrap()
        .iter()
        .any(|block| block["type"] == "tool_call"));
}

#[test]
fn a_recovered_complete_tool_keeps_its_original_block_order_json_arguments_and_error_result() {
    let (_root, store) = store();
    start_recoverable_run(&store);
    let raw = r#"{"path":"a"}"#;
    let mut writer = Writer::default();
    writer.persist(&store,"s",&[
        event(0,"message_start",json!({"entry_id":"m"})),
        event(1,"text_start",json!({"entry_id":"m","block_id":"before","content_key":"m:text:before","content_offset":0,"content_bytes":0,"content_first_idx":1})),
        event(2,"tool_start",json!({"entry_id":"m","tool_id":"t","tool_name":"read","phase":"input","tc_index":0,"tool_args":"","content_key":"m:2:tool:0","content_offset":0,"content_bytes":0,"content_first_idx":2})),
        event(3,"text_start",json!({"entry_id":"m","block_id":"after","content_key":"m:text:after","content_offset":0,"content_bytes":0,"content_first_idx":3})),
        event(4,"tool_delta",json!({"entry_id":"m","tc_index":0,"text":raw,"content_key":"m:2:tool:0","content_offset":0,"content_bytes":raw.len(),"content_first_idx":4})),
        event(5,"tool_input_end",json!({"entry_id":"m","tc_index":0,"tool_id":"t","tool_name":"read","tool_args":{"path":"a"},"provider_metadata":{"opaque":"retained"},"content_key":"m:2:tool:0","content_offset":raw.len(),"content_bytes":0,"content_first_idx":5,"content_seal":true})),
        event(6,"message_end",json!({"entry_id":"m","complete":true})),
        event(7,"tool_end",json!({"entry_id":"result","tool_id":"t","text":"","is_error":true})),
    ]).unwrap();
    store.recover_segmented_runs().unwrap();
    let entries = store.entries("s").unwrap();
    let message = entries.iter().find(|entry| entry["id"] == "m").unwrap();
    let blocks = message["content"].as_array().unwrap();
    assert_eq!(
        blocks
            .iter()
            .map(|block| block["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["text", "tool_call", "text"]
    );
    assert_eq!(blocks[1]["args"], json!({"path":"a"}));
    assert_eq!(blocks[1]["provider_metadata"]["opaque"], "retained");
    assert!(message["meta"]
        .get("recovered_incomplete_tool_input")
        .is_none());
    let result = entries
        .iter()
        .find(|entry| entry["id"] == "result")
        .unwrap();
    assert_eq!(result["content"][0]["is_error"], true);
    assert_eq!(result["content"][0]["content"], "");
}

#[test]
fn recovered_tool_input_snapshots_replace_the_unfinished_prefix_in_its_original_slot() {
    let (_root, store) = store();
    start_recoverable_run(&store);
    let mut writer = Writer::default();
    writer.persist(&store,"s",&[
        event(0,"message_start",json!({"entry_id":"m"})),
        event(1,"tool_delta",json!({"entry_id":"m","tc_index":0,"text":"old","content_key":"m:1:tool:0","content_offset":0,"content_bytes":3,"content_first_idx":1})),
        event(2,"tool_delta",json!({"entry_id":"m","tc_index":0,"snapshot":true,"text":"new","content_key":"m:2:tool:0","content_offset":0,"content_bytes":3,"content_first_idx":2})),
    ]).unwrap();
    store.recover_segmented_runs().unwrap();
    let entries = store.entries("s").unwrap();
    let message = entries.iter().find(|entry| entry["id"] == "m").unwrap();
    assert_eq!(message["content"].as_array().unwrap().len(), 1);
    assert_eq!(message["content"][0]["text"], "new");
}

#[test]
fn recovery_releases_unpublished_fact_staging_only_in_the_interrupted_run_scope() {
    let (_root, store) = store();
    start_recoverable_run(&store);
    let run = StreamIdentity {
        session_id: "s".into(),
        run_id: "r".into(),
        epoch: 1,
    };
    let other = StreamIdentity {
        session_id: "s".into(),
        run_id: "other".into(),
        epoch: 1,
    };
    let owner = super::stream::owner(&run, "fact:0:text");
    let unrelated = super::stream::owner(&other, "fact:0:text");
    super::entries::stage_named(
        &store,
        "s",
        "stream",
        &owner,
        "",
        "unfinished fact body",
        false,
    )
    .unwrap();
    super::entries::stage_named(
        &store,
        "s",
        "stream",
        &unrelated,
        "",
        "other run is still owned",
        false,
    )
    .unwrap();
    // No change or stream_runs row was published for either staged body.
    store.recover_segmented_runs().unwrap();
    let (missing, preserved) = store
        .db
        .read(move |db| {
            Ok((
                super::resolve(db, "s", "stream", &owner, "")?.is_none(),
                super::resolve(db, "s", "stream", &unrelated, "")?.is_some(),
            ))
        })
        .unwrap();
    assert!(missing);
    assert!(preserved);
}

#[test]
fn coalescing_does_not_discard_changing_extension_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&directory.path().join("agent.db")).unwrap();
    store.reserve_segmented("s").unwrap();
    let mut writer = Writer::default();
    writer.persist(&store,"s",&[
        event(0,"text_chunk",json!({"content_key":"m:text:b","entry_id":"m","content_offset":0,"content_bytes":1,"text":"a","future":false})),
        event(1,"text_chunk",json!({"content_key":"m:text:b","entry_id":"m","content_offset":1,"content_bytes":1,"text":"b","future":null})),
    ]).unwrap();
    let page = store
        .segmented_replay("s", "r", -1, false, Some(8))
        .unwrap();
    assert_eq!(page.events.len(), 2);
    assert_eq!(
        serde_json::from_str::<Value>(&page.events[0].data).unwrap()["future"],
        false
    );
    assert!(serde_json::from_str::<Value>(&page.events[1].data).unwrap()["future"].is_null());
}
