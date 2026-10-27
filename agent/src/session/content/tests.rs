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
            bind(&tx, "new", "stream", "live-block", "", range)?;
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
            bind(&tx, "new", "stream", "arguments", "", range)?;
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
