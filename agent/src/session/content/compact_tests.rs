use super::*;
use crate::session::content::{
    self,
    stream::{self, ContentAppend, StreamIdentity},
};

fn db() -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE sessions(id TEXT PRIMARY KEY,revision INTEGER NOT NULL DEFAULT 0); CREATE TABLE entries(session_id TEXT,entry_id TEXT,UNIQUE(session_id,entry_id)); CREATE TABLE compaction_operations(session_id TEXT,input_key TEXT,PRIMARY KEY(session_id,input_key)); CREATE TABLE runs(session_id TEXT,run_id TEXT,PRIMARY KEY(session_id,run_id)); CREATE TABLE queued_run_receipts(session_id TEXT,run_id TEXT,PRIMARY KEY(session_id,run_id)); INSERT INTO sessions(id) VALUES ('s');").unwrap();
    db.execute_batch(include_str!("schema.sql")).unwrap();
    let tx = db.transaction().unwrap();
    content::enable(&tx, "s").unwrap();
    tx.commit().unwrap();
    db
}
fn scope() -> StreamIdentity {
    StreamIdentity {
        session_id: "s".into(),
        run_id: "r".into(),
        epoch: 1,
    }
}
fn change(cursor: i64, key: &str) -> Change {
    Change {
        first_cursor: cursor,
        last_cursor: cursor,
        event_type: "text_chunk".into(),
        metadata: json!({"timestamp":"2026-10-27T00:00:00.123456789+08:00","run_sequence":0,"data":{"type":"text_chunk","entry_id":"message","tool_id":null,"content_key":key,"content_offset":cursor,"content_bytes":1,"content_first_idx":cursor,"text":"","false":false,"empty":[],"future":{"signature":"  raw\\nbytes  "}}}),
        bodies: vec![stream::BodyLink {
            field: "/data/text".into(),
            encoding: Encoding::Text,
            key: key.into(),
            offset: cursor,
            length: 1,
        }],
    }
}
fn append(offset: i64, key: &str) -> ContentAppend {
    ContentAppend {
        key: key.into(),
        offset,
        text: "x".into(),
        seal: None,
    }
}

#[test]
fn compact_single_ranges_share_context_and_preserve_presence_precision_and_unknowns() {
    let mut db = db();
    for cursor in 0..100 {
        stream::commit_batch(
            &mut db,
            &scope(),
            vec![append(cursor, "key")],
            vec![change(cursor, "key")],
        )
        .unwrap();
    }
    assert_eq!(
        db.query_row("SELECT count(*) FROM stream_contexts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM stream_change_content", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let ids = (1..=100).collect::<Vec<_>>();
    let encoded = read_encoded(&db, &ids).unwrap();
    assert_eq!(encoded.contexts.len(), 1);
    for (cursor, stored) in encoded.decode().unwrap().into_iter().enumerate() {
        assert_eq!(stored.metadata, change(cursor as i64, "key").metadata);
        assert_eq!(stored.bodies[0].1.offset, cursor as i64);
    }
    let id: i64 = db
        .query_row("SELECT content_id FROM stream_changes LIMIT 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    db.execute("DELETE FROM content_references", []).unwrap();
    let tx = db.transaction().unwrap();
    assert_eq!(content::collect(&tx, &[id]).unwrap(), 0);
    tx.commit().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT byte_length FROM content_objects WHERE id=?1",
            [id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        100
    );
    db.execute("DELETE FROM stream_runs", []).unwrap();
    let tx = db.transaction().unwrap();
    assert_eq!(content::collect(&tx, &[id]).unwrap(), 0);
    assert_eq!(
        tx.query_row(
            "SELECT count(*) FROM content_segments WHERE content_id=?1",
            [id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        96
    );
    tx.commit().unwrap();
    let mut deleted = 0;
    for _ in 0..24 {
        let tx = db.transaction().unwrap();
        deleted += content::collect(&tx, &[id]).unwrap();
        tx.commit().unwrap();
    }
    assert_eq!(deleted, 1);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM content_objects WHERE id=?1",
            [id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn failed_publication_cannot_reuse_rolled_back_context_or_acknowledge_body() {
    let mut db = db();
    db.execute_batch("CREATE TRIGGER reject_change BEFORE INSERT ON stream_changes BEGIN SELECT RAISE(ABORT,'fault'); END;").unwrap();
    assert!(stream::commit_batch(
        &mut db,
        &scope(),
        vec![append(0, "first")],
        vec![change(0, "first")]
    )
    .is_err());
    for table in [
        "stream_contexts",
        "stream_changes",
        "content_objects",
        "stream_runs",
    ] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    db.execute_batch("DROP TRIGGER reject_change").unwrap();
    let event = change(0, "different");
    stream::commit_batch(
        &mut db,
        &scope(),
        vec![append(0, "different")],
        vec![event.clone()],
    )
    .unwrap();
    stream::commit_batch(
        &mut db,
        &scope(),
        vec![append(0, "different")],
        vec![event.clone()],
    )
    .unwrap();
    let stored = read_ids(&db, &[1]).unwrap().pop().unwrap();
    assert_eq!(stored.metadata, event.metadata);
    let mut conflict = event;
    conflict.metadata["data"]["future"] = json!(false);
    assert!(stream::commit_batch(&mut db, &scope(), vec![], vec![conflict]).is_err());
    assert_eq!(
        db.query_row("SELECT durable_cursor FROM stream_runs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn generic_multi_field_unknown_event_and_digest_collision_are_lossless() {
    let mut db = db();
    let mut event = change(0, "key");
    event.event_type = "provider_future_event".into();
    event.metadata["data"]["other"] = json!({});
    event.bodies.push(stream::BodyLink {
        field: "/data/other".into(),
        encoding: Encoding::Json,
        key: "other".into(),
        offset: 0,
        length: 2,
    });
    stream::commit_batch(
        &mut db,
        &scope(),
        vec![
            append(0, "key"),
            ContentAppend {
                key: "other".into(),
                offset: 0,
                text: "{}".into(),
                seal: None,
            },
        ],
        vec![event.clone()],
    )
    .unwrap();
    let stored = read_ids(&db, &[1]).unwrap().pop().unwrap();
    assert_eq!(stored.event_type, "provider_future_event");
    assert_eq!(stored.metadata, event.metadata);
    assert_eq!(stored.bodies.len(), 2);
    assert_eq!(
        db.query_row("SELECT count(*) FROM stream_change_content", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    // Plant a colliding hash with a different descriptor, never accept hash
    // equality as context identity. The actual SHA algorithm need not collide.
    let next = change(1, "key");
    let body = (
        "/data/text".into(),
        ContentRange {
            id: 1,
            offset: 1,
            length: 1,
        },
        Encoding::Text,
    );
    let (descriptor, _, _) = encode(&next, Some(&body)).unwrap();
    let tx = db.transaction().unwrap();
    let (descriptor, _) = content::symbols::intern(&tx, 1, descriptor).unwrap();
    tx.execute("INSERT INTO stream_contexts(stream_id,digest,descriptor_json,symbol_bytes) VALUES (1,?1,'[{},null,null,{},0]',0)",[Sha256::digest(descriptor.as_bytes()).as_slice()]).unwrap();
    tx.commit().unwrap();
    stream::commit_batch(
        &mut db,
        &scope(),
        vec![append(1, "key")],
        vec![next.clone()],
    )
    .unwrap();
    assert_eq!(read_ids(&db, &[2]).unwrap()[0].metadata, next.metadata);
}

#[test]
fn context_ownership_and_fixed_kind_codes_are_database_constraints() {
    let mut db = db();
    for (i, kind) in KINDS.iter().enumerate() {
        let event = Change {
            first_cursor: i as i64,
            last_cursor: i as i64,
            event_type: (*kind).into(),
            metadata: json!({"data":{}}),
            bodies: vec![],
        };
        // Do not use commit_batch's terminal semantics: this verifies the
        // complete fixed code table against SQLite's generated type column.
        let tx = db.transaction().unwrap();
        tx.execute(
            "INSERT OR IGNORE INTO stream_runs(id,session_id,run_id,epoch) VALUES (1,'s','r',1)",
            [],
        )
        .unwrap();
        publish(&tx, 1, &event, vec![]).unwrap();
        tx.commit().unwrap();
        let pair: (i64, String) = db
            .query_row(
                "SELECT event_kind,event_type FROM stream_changes WHERE last_cursor=?1",
                [i as i64],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(pair, (i as i64 + 1, kind.to_string()));
    }
    db.execute(
        "INSERT INTO stream_runs(id,session_id,run_id,epoch) VALUES (2,'s','other',1)",
        [],
    )
    .unwrap();
    assert!(db
        .execute("UPDATE stream_changes SET stream_id=2 WHERE id=1", [])
        .is_err());
    assert!(db
        .execute(
            "UPDATE stream_contexts SET descriptor_json='[{},null,null]'",
            []
        )
        .is_err());
    validate_layout(&db).unwrap();
}

#[test]
fn scope_guards_reject_foreign_owner_updates_and_noop_replacements() {
    let mut db = db();
    stream::commit_batch(
        &mut db,
        &scope(),
        vec![append(0, "key")],
        vec![change(0, "key")],
    )
    .unwrap();
    db.execute(
        "INSERT INTO stream_runs(id,session_id,run_id,epoch) VALUES (2,'s','other',1)",
        [],
    )
    .unwrap();
    assert!(db
        .execute("UPDATE stream_body_owners SET stream_id=2", [])
        .is_err());
    assert!(db.execute("INSERT INTO stream_body_owners SELECT 2,key_id,content_id,byte_offset,byte_length,encoding FROM stream_body_owners", []).is_err());
    assert!(db
        .execute("UPDATE stream_changes SET event_kind=9", [])
        .is_err());
    db.execute_batch("DROP TRIGGER stream_change_scope_insert; CREATE TRIGGER stream_change_scope_insert BEFORE INSERT ON stream_changes BEGIN SELECT 1; END;").unwrap();
    assert!(validate_layout(&db).is_err());
}

#[test]
fn query_plans_use_short_context_cursor_and_reverse_body_indices() {
    let db = db();
    for (sql,expected) in [
        ("SELECT c.id FROM stream_contexts x INDEXED BY stream_contexts_tool JOIN stream_changes c INDEXED BY stream_changes_context ON c.context_id=x.id WHERE x.stream_id=1 AND x.entry_id='m' AND x.tool_id='t' AND x.event_kind IN (11,12,13) AND c.event_kind=13 ORDER BY c.last_cursor DESC LIMIT 1", "stream_changes_context"),
        ("SELECT c.id FROM stream_changes c WHERE c.stream_id=1 AND c.last_cursor>3 ORDER BY c.last_cursor LIMIT 128", "sqlite_autoindex_stream_changes_1"),
        ("SELECT 1 FROM stream_content_ranges WHERE content_id=1", "stream_changes_body"),
        ("SELECT id FROM stream_changes WHERE stream_id=1 AND event_type IN ('usage','model_changed') ORDER BY id", "stream_changes_pricing"),
    ] {
        let mut query=db.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        let plan=query.query_map([],|r|r.get::<_,String>(3)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap().join("; ");
        assert!(plan.contains(expected),"missing {expected}: {plan}");
        println!("COMPACT_QUERY_PLAN={plan}");
    }
}

#[test]
fn long_identities_are_interned_across_changing_contexts_without_losing_nul_or_presence() {
    let mut db = db();
    for cursor in 0..64 {
        let mut event = change(cursor, "unused");
        event.bodies.clear();
        event.metadata["data"]["entry_id"] = json!("long-消息\0identity");
        event.metadata["data"]["tool_id"] = json!("");
        event.metadata["data"]["content_key"] = json!(format!("block-{cursor}"));
        stream::commit_batch(&mut db, &scope(), vec![], vec![event.clone()]).unwrap();
        assert_eq!(
            read_ids(&db, &[cursor + 1]).unwrap()[0].metadata,
            event.metadata
        );
    }
    assert_eq!(
        db.query_row("SELECT count(*) FROM stream_symbols", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        66
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM stream_contexts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        64
    );
    assert!(db
        .execute("UPDATE stream_symbols SET value='corrupt'", [])
        .is_err());
    db.execute(
        "INSERT INTO stream_runs(id,session_id,run_id,epoch) VALUES (2,'s','other',1)",
        [],
    )
    .unwrap();
    let tx = db.transaction().unwrap();
    let mut event = change(0, "unused");
    event.bodies.clear();
    publish(&tx, 2, &event, vec![]).unwrap();
    tx.commit().unwrap();
    let symbol: i64 = db
        .query_row(
            "SELECT id FROM stream_symbols WHERE stream_id=1 LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let tx = db.transaction().unwrap();
    let bad = json!([{"data":{}},null,null,{"entry_id":symbol},0]).to_string();
    assert!(context(&tx, 2, &bad, 0).is_err());
    drop(tx);
    validate_layout(&db).unwrap();
}

#[test]
fn damaged_context_is_an_error_instead_of_an_empty_successful_page() {
    let mut db = db();
    let mut event = change(0, "unused");
    event.bodies.clear();
    stream::commit_batch(&mut db, &scope(), vec![], vec![event]).unwrap();
    db.pragma_update(None, "foreign_keys", false).unwrap();
    db.execute("DELETE FROM stream_contexts", []).unwrap();
    assert!(stream::read_changes(&db, &scope(), -1, 8).is_err());
    assert!(read_ids(&db, &[1]).is_err());
}
