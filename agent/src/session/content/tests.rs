//! Specification tests, authored during implementation and run at final validation.
use super::*;

fn connection() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE sessions(id TEXT PRIMARY KEY,revision INTEGER NOT NULL DEFAULT 0); CREATE TABLE entries(session_id TEXT,entry_id TEXT,UNIQUE(session_id,entry_id)); CREATE TABLE compaction_operations(session_id TEXT,input_key TEXT,PRIMARY KEY(session_id,input_key)); CREATE TABLE runs(session_id TEXT,run_id TEXT,PRIMARY KEY(session_id,run_id)); CREATE TABLE queued_run_receipts(session_id TEXT,run_id TEXT,PRIMARY KEY(session_id,run_id)); INSERT INTO sessions(id) VALUES('parent'),('child');").unwrap();
    db.execute_batch(include_str!("schema.sql")).unwrap();
    db
}

#[test]
fn utf8_segments_are_bounded_append_only_and_page_without_loss() {
    let mut db = connection();
    let text = "中𠮷a".repeat(8000);
    let tx = db.transaction().unwrap();
    let id = create(&tx).unwrap();
    let mut digest = ContentDigest::default();
    let mut at = 0;
    while at < text.len() {
        let mut end = (at + WRITE_BYTES).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        append(&tx, id, at as i64, &text[at..end]).unwrap();
        digest.push(&text[at..end]).unwrap();
        at = end;
    }
    seal(&tx, id, digest).unwrap();
    tx.commit().unwrap();
    let largest: i64 = db
        .query_row(
            "SELECT max(length(CAST(body AS BLOB))) FROM content_segments",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(largest <= SEGMENT_BYTES as i64);
    let mut result = String::new();
    let mut offset = 0;
    while offset < text.len() as i64 {
        let page = read(&db, id, offset, 1237).unwrap();
        assert!(page.sealed);
        assert!(page.next_offset > offset);
        offset = page.next_offset;
        result.push_str(&page.text);
    }
    assert_eq!(result, text);
    assert!(read(&db, id, 1, 100).is_err());
}

#[test]
fn retries_are_idempotent_and_gaps_overlaps_and_mutation_fail() {
    let mut db = connection();
    let tx = db.transaction().unwrap();
    let id = create(&tx).unwrap();
    append(&tx, id, 0, "abc").unwrap();
    append(&tx, id, 0, "abc").unwrap();
    assert!(append(&tx, id, 0, "abd").is_err());
    assert!(append(&tx, id, 2, "cd").is_err());
    assert!(append(&tx, id, 4, "e").is_err());
    let mut hash = ContentDigest::default();
    hash.push("abc").unwrap();
    seal(&tx, id, hash.clone()).unwrap();
    seal(&tx, id, hash).unwrap();
    append(&tx, id, 0, "abc").unwrap();
    assert!(append(&tx, id, 3, "d").is_err());
    assert_eq!(
        tx.query_row("SELECT count(*) FROM content_segments", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn parent_deletion_cannot_collect_shared_child_content() {
    let mut db = connection();
    let tx = db.transaction().unwrap();
    let id = create(&tx).unwrap();
    append(&tx, id, 0, "shared").unwrap();
    let range = ContentRange {
        id,
        offset: 0,
        length: 6,
    };
    bind(&tx, "parent", "entry", "p", "/content/0/text", range).unwrap();
    bind(&tx, "child", "entry", "c", "/content/0/text", range).unwrap();
    assert!(resolve(&tx, "child", "entry", "p", "/content/0/text")
        .unwrap()
        .is_none());
    tx.execute("DELETE FROM sessions WHERE id='parent'", [])
        .unwrap();
    assert_eq!(collect(&tx, &[id]).unwrap(), 0);
    assert_eq!(read(&tx, id, 0, 100).unwrap().text, "shared");
    tx.execute("DELETE FROM sessions WHERE id='child'", [])
        .unwrap();
    assert_eq!(collect(&tx, &[id]).unwrap(), 1);
    assert_eq!(
        tx.query_row("SELECT count(*) FROM content_segments", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn rollback_removes_segments_references_and_format_together() {
    let mut db = connection();
    {
        let tx = db.transaction().unwrap();
        enable(&tx, "parent").unwrap();
        let id = create(&tx).unwrap();
        let range = append(&tx, id, 0, "not committed").unwrap();
        bind(&tx, "parent", "entry", "e", "/content/0/text", range).unwrap();
    }
    assert_eq!(format(&db, "parent").unwrap(), 1);
    for table in ["content_objects", "content_segments", "content_references"] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    let tx = db.transaction().unwrap();
    tx.execute("INSERT INTO entries VALUES('parent','entry')", [])
        .unwrap();
    assert!(enable(&tx, "parent").is_err());
}

#[test]
fn structured_stream_owners_isolate_scopes_and_protect_staged_bodies() {
    let mut db = connection();
    let tx = db.transaction().unwrap();
    enable(&tx, "parent").unwrap();
    enable(&tx, "child").unwrap();
    let scope = stream::StreamIdentity {
        session_id: "parent".into(),
        run_id: "r".into(),
        epoch: 0,
    };
    let body = create(&tx).unwrap();
    let range = append(&tx, body, 0, "staged").unwrap();
    stream_body::bind(&tx, &scope, "same-key", range, stream::Encoding::Text).unwrap();
    assert_eq!(
        stream_body::get(&tx, &scope, "same-key")
            .unwrap()
            .unwrap()
            .0,
        range
    );
    for other in [
        stream::StreamIdentity {
            session_id: "child".into(),
            ..scope.clone()
        },
        stream::StreamIdentity {
            run_id: "other".into(),
            ..scope.clone()
        },
        stream::StreamIdentity {
            epoch: 1,
            ..scope.clone()
        },
    ] {
        assert!(stream_body::get(&tx, &other, "same-key").unwrap().is_none());
    }
    assert_eq!(collect(&tx, &[body]).unwrap(), 0);
    assert_eq!(
        tx.query_row("SELECT count(*) FROM content_references", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(tx.query_row("SELECT count(*) FROM stream_body_owners WHERE typeof(stream_id)='integer' AND typeof(key_id)='integer'", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    tx.execute("DELETE FROM stream_runs", []).unwrap();
    assert_eq!(
        tx.query_row(
            "SELECT count(*) FROM content_collection WHERE content_id=?1",
            [body],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(collect(&tx, &[body]).unwrap(), 1);
}

#[test]
fn structured_stream_reservations_rollback_without_publishing_or_reusing_ids() {
    let mut db = connection();
    {
        let tx = db.transaction().unwrap();
        enable(&tx, "parent").unwrap();
        let scope = stream::StreamIdentity {
            session_id: "parent".into(),
            run_id: "r".into(),
            epoch: 0,
        };
        let body = create(&tx).unwrap();
        let range = append(&tx, body, 0, "uncommitted").unwrap();
        stream_body::bind(&tx, &scope, "key", range, stream::Encoding::Text).unwrap();
    }
    for table in [
        "stream_runs",
        "stream_symbols",
        "stream_body_owners",
        "stream_changes",
        "content_objects",
    ] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn terminal_stream_owners_allow_exact_retry_but_reject_new_or_growing_bodies() {
    let mut db = connection();
    let tx = db.transaction().unwrap();
    enable(&tx, "parent").unwrap();
    let scope = stream::StreamIdentity {
        session_id: "parent".into(),
        run_id: "r".into(),
        epoch: 0,
    };
    let body = create(&tx).unwrap();
    let range = append(&tx, body, 0, "prefix").unwrap();
    stream_body::bind(&tx, &scope, "key", range, stream::Encoding::Text).unwrap();
    stream::append_boundary(&tx, &scope, "agent_end", serde_json::json!({})).unwrap();
    stream_body::bind(&tx, &scope, "key", range, stream::Encoding::Text).unwrap();
    assert!(stream_body::bind(&tx, &scope, "new", range, stream::Encoding::Text).is_err());
    assert!(stream_body::extend(&tx, &scope, "key", body, range.length + 1).is_err());
    assert_eq!(
        stream_body::get(&tx, &scope, "key").unwrap().unwrap().0,
        range
    );
}

#[test]
fn bounded_read_rejects_a_missing_committed_segment() {
    let mut db = connection();
    let tx = db.transaction().unwrap();
    let id = create(&tx).unwrap();
    append(&tx, id, 0, "first").unwrap();
    append(&tx, id, 5, "second").unwrap();
    tx.execute(
        "DELETE FROM content_segments WHERE content_id=?1 AND byte_offset=5",
        [id],
    )
    .unwrap();
    assert!(read(&tx, id, 0, 100).is_err());
}

#[test]
fn segmented_transcripts_round_trip_extensions_and_reuse_sealed_stream_bodies() {
    use crate::session::sqlite_store::SqliteStore;
    use serde_json::json;
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    store
        .db
        .call(|db| {
            let tx = db.transaction()?;
            tx.execute("INSERT INTO sessions(id) VALUES('new')", [])?;
            enable(&tx, "new")?;
            let id = create(&tx)?;
            append(&tx, id, 0, "shared text")?;
            let mut digest = ContentDigest::default();
            digest.push("shared text")?;
            let range = seal(&tx, id, digest)?;
            let stream = stream::StreamIdentity {
                session_id: "new".into(),
                run_id: "r".into(),
                epoch: 0,
            };
            bind(
                &tx,
                "new",
                "stream",
                &stream::owner(&stream, "live-block"),
                "",
                range,
            )?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();
    let entry = json!({"id":"a","type":"assistant","role":"assistant","timestamp":"2026-10-26T00:00:00Z","meta":{"run_id":"r"},"content":[
        {"type":"text","text":"shared text","provider_metadata":{"signature":"opaque"}},
        {"type":"tool_call","id":"t","name":"shell","args":{"command":"printf test"}},
        {"type":"future_block","opaque":{"keep":true}}
    ]});
    store.append("new", vec![entry.clone()]).unwrap();
    store.append("new", vec![entry.clone()]).unwrap();
    assert_eq!(store.entries("new").unwrap(), vec![entry.clone()]);
    store.db.read(|db| {
        assert_eq!(db.query_row("SELECT count(*) FROM content_objects",[],|r|r.get::<_,i64>(0))?,2);
        assert_eq!(db.query_row("SELECT count(*) FROM message_blocks WHERE text IS NOT NULL OR arguments_json IS NOT NULL",[],|r|r.get::<_,i64>(0))?,0);
        Ok(())
    }).unwrap();
    let mut changed = entry;
    changed["content"][0]["text"] = json!("different");
    assert!(store.append("new", vec![changed]).is_err());
    assert_eq!(
        store.entries("new").unwrap()[0]["content"][0]["text"],
        "shared text"
    );
}

#[test]
fn large_entry_staging_preserves_utf8_with_bounded_segment_rows() {
    use crate::session::sqlite_store::SqliteStore;
    use serde_json::json;
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    store
        .db
        .call(|db| {
            let tx = db.transaction()?;
            tx.execute("INSERT INTO sessions(id) VALUES('new')", [])?;
            enable(&tx, "new")?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();
    let text = "工具𠮷".repeat(100_000);
    let entry = json!({"id":"tool","type":"tool","role":"tool","timestamp":"2026-10-26T00:00:00Z","meta":{"run_id":"r"},"content":[{"type":"tool_result","tool_call_id":"t","content":text,"is_error":false}]});
    store.append("new", vec![entry.clone()]).unwrap();
    assert_eq!(store.entries("new").unwrap(), vec![entry]);
    store
        .db
        .read(|db| {
            let max: i64 = db.query_row(
                "SELECT max(length(CAST(body AS BLOB))) FROM content_segments",
                [],
                |r| r.get(0),
            )?;
            assert!(max <= SEGMENT_BYTES as i64);
            Ok(())
        })
        .unwrap();
}

#[test]
fn streamed_tool_arguments_are_shared_despite_json_whitespace() {
    use crate::session::sqlite_store::SqliteStore;
    use serde_json::json;
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    store
        .db
        .call(|db| {
            let tx = db.transaction()?;
            tx.execute("INSERT INTO sessions(id) VALUES('new')", [])?;
            enable(&tx, "new")?;
            let id = create(&tx)?;
            let raw = "{ \"command\" : \"pwd\" }";
            append(&tx, id, 0, raw)?;
            let digest = JsonContentDigest::verified(raw, &json!({"command":"pwd"}))?;
            let range = seal_json(&tx, id, digest)?;
            let stream = stream::StreamIdentity {
                session_id: "new".into(),
                run_id: "r".into(),
                epoch: 0,
            };
            bind(
                &tx,
                "new",
                "stream",
                &stream::owner(&stream, "arguments"),
                "",
                range,
            )?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();
    let entry = json!({"id":"a","type":"assistant","role":"assistant","timestamp":"2026-10-26T00:00:00Z","meta":{"run_id":"r"},"content":[{"type":"tool_call","id":"t","name":"shell","args":{"command":"pwd"}}]});
    store.append("new", vec![entry.clone()]).unwrap();
    store.append("new", vec![entry.clone()]).unwrap();
    assert_eq!(store.entries("new").unwrap(), vec![entry]);
    assert_eq!(
        store
            .db
            .read(|db| Ok(
                db.query_row("SELECT count(*) FROM content_objects", [], |r| r
                    .get::<_, i64>(0))?
            ))
            .unwrap(),
        1
    );
}

#[test]
fn revised_final_arguments_do_not_alias_different_streamed_input() {
    assert!(JsonContentDigest::verified(
        r#"{"command":"pwd"}"#,
        &serde_json::json!({"command":"delete"})
    )
    .is_err());
}

/// The projections registered on every application connection are read from
/// SQL by the reader and the exporter, so each one must agree with the Rust
/// implementation it wraps — and refuse input it cannot interpret.
#[test]
fn the_registered_sql_projections_agree_with_their_rust_implementations() {
    use crate::session::sqlite_store::SqliteStore;
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    let result = future_rpc::shell_result::ShellResult {
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
            output: "hi\n".into(),
            output_truncated: false,
            escalated: false,
        }],
        approval: None,
        note: None,
    };
    let raw = serde_json::to_string(&result).unwrap();
    let expected = future_rpc::shell_result::model_summary(&result);
    store
        .db
        .read(move |db| {
            let summary: String =
                db.query_row("SELECT future_shell_summary(?1)", [&raw], |row| row.get(0))?;
            assert_eq!(summary, expected);
            assert!(!expected.is_empty());
            // Uninterpretable facts are an error, never an empty projection.
            let broken = db.query_row("SELECT future_shell_summary('not facts')", [], |row| {
                row.get::<_, String>(0)
            });
            assert!(broken.is_err());

            // Interrupted tool input is repaired through the same versioned
            // interpretation the model loop uses; a bare body is preserved.
            let repaired: String = db.query_row(
                "SELECT future_tool_args_string_v1('{\"command\":\"pwd\"')",
                [],
                |row| row.get(0),
            )?;
            // The projection answers the *encoded* repaired body.
            assert_eq!(
                serde_json::from_str::<String>(&repaired).unwrap(),
                "{\"command\":\"pwd\"}"
            );
            let empty: String =
                db.query_row("SELECT future_tool_args_string_v1('')", [], |row| {
                    row.get(0)
                })?;
            assert_eq!(serde_json::from_str::<String>(&empty).unwrap(), "{}");
            assert_eq!(empty, "\"{}\"");
            Ok(())
        })
        .unwrap();
}

/// A shell result on an entry is canonical facts: the object is staged as its
/// own JSON body, and the `content` text that merely repeats the model summary
/// is bound to the *summary view* of those facts instead of being copied.
#[test]
fn a_shell_result_entry_binds_its_facts_and_summary_as_bodies() {
    use crate::session::sqlite_store::SqliteStore;
    use serde_json::json;
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    store
        .db
        .call(|db| {
            let tx = db.transaction()?;
            tx.execute("INSERT INTO sessions(id) VALUES('new')", [])?;
            enable(&tx, "new")?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();
    let result = future_rpc::shell_result::ShellResult {
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
            output: "hi\n".into(),
            output_truncated: false,
            escalated: false,
        }],
        approval: None,
        note: None,
    };
    let summary = future_rpc::shell_result::model_summary(&result);
    let entry = json!({"id":"tool","type":"tool","role":"tool","timestamp":"2026-10-26T00:00:00Z",
        "meta":{"run_id":"r","shell_result":serde_json::to_value(&result).unwrap()},
        "content":[{"type":"tool_result","tool_call_id":"t","content":summary,"is_error":false}]});
    store.append("new", vec![entry.clone()]).unwrap();
    // The transcript still reads back losslessly, including the facts object.
    assert_eq!(store.entries("new").unwrap(), vec![entry]);
    // Both bodies are references: the facts as JSON, the repeated text as the
    // summary projection of those same facts.
    store
        .db
        .read(|db| {
            let mut statement = db.prepare(
                "SELECT field,encoding FROM content_references WHERE session_id='new' AND owner_kind='entry' ORDER BY field",
            )?;
            let rows = statement
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            assert_eq!(
                rows,
                vec![
                    ("/content/0/content".to_string(), "shell_summary".to_string()),
                    ("/meta/shell_result".to_string(), "json".to_string()),
                ]
            );
            // Both references point at the same stored facts: the summary view
            // is a projection, never a second copy of the body.
            let objects: i64 =
                db.query_row("SELECT count(*) FROM content_objects", [], |r| r.get(0))?;
            assert_eq!(objects, 1);
            let distinct: i64 = db.query_row(
                "SELECT count(DISTINCT content_id) FROM content_references WHERE session_id='new'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(distinct, 1, "one object serves facts and summary");
            Ok(())
        })
        .unwrap();
}
