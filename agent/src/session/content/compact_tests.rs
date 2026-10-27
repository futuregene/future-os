use super::*;
use crate::session::content::{
    self,
    stream::{self, ContentAppend, StreamIdentity},
};

fn db() -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE sessions(id TEXT PRIMARY KEY,revision INTEGER NOT NULL DEFAULT 0); CREATE TABLE entries(session_id TEXT,entry_id TEXT,UNIQUE(session_id,entry_id)); CREATE TABLE compaction_operations(session_id TEXT,input_key TEXT,PRIMARY KEY(session_id,input_key)); INSERT INTO sessions(id) VALUES ('s');").unwrap();
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
    db.execute("INSERT INTO stream_contexts(stream_id,digest,descriptor_json) VALUES (1,?1,'[{},null,null]')",[Sha256::digest(descriptor.as_bytes()).as_slice()]).unwrap();
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
fn query_plans_use_short_context_cursor_and_reverse_body_indices() {
    let db = db();
    for (sql,expected) in [
        ("SELECT c.id FROM stream_contexts x INDEXED BY stream_contexts_tool JOIN stream_changes c INDEXED BY stream_changes_context ON c.context_id=x.id WHERE x.stream_id=1 AND x.entry_id='m' AND x.tool_id='t' AND c.event_kind=13 ORDER BY c.last_cursor DESC LIMIT 1", "stream_changes_context"),
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
