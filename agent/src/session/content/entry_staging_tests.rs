//! Concurrency postconditions of staging a field before header publication.
//! Authored for the final test phase; no tests during implementation.
use super::*;
use serde_json::json;

fn published(sealed: bool) -> (tempfile::TempDir, SqliteStore, StagingOwner) {
    let root = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&root.path().join("agent.db")).unwrap();
    store.reserve_segmented("s").unwrap();
    store.replace("s",vec![json!({"id":"info","type":"session_info","timestamp":"2026-10-26T00:00:00Z","content":{}})]).unwrap();
    let owner=store.db.call(move |db| {
        let tx=crate::session::database::begin_write_transaction(db)?;
        let id=create(&tx)?;
        let text=if sealed {"complete body"} else {"partial"};
        let range=append(&tx,id,0,text)?;
        if sealed {
            let mut digest=ContentDigest::default();digest.push(text)?;
            seal(&tx,id,digest)?;
        }
        super::super::bind(&tx,"s","entry","a","/content/0/text",range)?;
        crate::session::sqlite_store::insert_transcript_entries(&tx,"s",vec![json!({"id":"a","type":"assistant","role":"assistant","timestamp":"2026-10-26T00:00:01Z","content":[{"type":"text","text":""}]})])?;
        tx.commit()?;
        Ok(StagingOwner {session:"s".into(),kind:"entry",owner:"a".into(),field:"/content/0/text".into(),body:id,encoding:super::super::stream::Encoding::Text})
    }).unwrap();
    (root, store, owner)
}

#[test]
fn a_late_identical_piece_cannot_shrink_a_concurrently_published_reference() {
    let (_root, store, owner) = published(true);
    // A retry read offset=0 before the first producer finished. The original
    // producer has now sealed/published, so this is an identical old prefix.
    store
        .db
        .call(move |db| append_piece(db, &owner, 0, "complete"))
        .unwrap();
    assert_eq!(
        store.entries("s").unwrap().last().unwrap()["content"][0]["text"],
        "complete body"
    );
}

#[test]
fn publication_between_staging_pieces_prevents_extension_in_the_same_transaction() {
    let (_root, store, owner) = published(false);
    assert!(store
        .db
        .call(move |db| append_piece(db, &owner, 7, " unacknowledged tail"))
        .is_err());
    assert_eq!(
        store.entries("s").unwrap().last().unwrap()["content"][0]["text"],
        "partial"
    );
}

/// A reset copies checkpoint-owned references instead of re-staging them, so the
/// copy replaces each referenced field with a placeholder. A `/content/<n>/error`
/// field is the exception in one direction: the message stays, truncated to 512
/// characters — a parser downstream reads it as text and must not receive a
/// megabyte of it.
#[test]
fn a_staged_error_reference_keeps_its_message_truncated_to_512_characters() {
    let root = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&root.path().join("agent.db")).unwrap();
    store.reserve_segmented("s").unwrap();
    store
        .replace(
            "s",
            vec![json!({"id":"info","type":"session_info","timestamp":"2026-10-26T00:00:00Z","content":{}})],
        )
        .unwrap();
    store
        .db
        .call(|db| {
            let tx = crate::session::database::begin_write_transaction(db)?;
            let id = create(&tx)?;
            let range = append(&tx, id, 0, "refused by the sandbox")?;
            // A reset stages its copies under the `checkpoint` owner kind, with
            // the operation's prefix in front of the entry id.
            super::super::bind(&tx, "s", "checkpoint", "ckpt-a", "/content/error", range)?;
            tx.commit()?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();

    let long = "e".repeat(600);
    // The guard names the field exactly , and only a non-conversation
    // entry keeps an object under  (normalisation rewrites conversational ones).
    let entry = json!({
        "id": "a",
        "type": "compaction_failure",
        "timestamp": "2026-10-26T00:00:01Z",
        "content": {"error": long}
    });
    let prepared = prepare_staged(&store, "s", vec![entry], "ckpt-").unwrap();
    let error = prepared[0]["content"]["error"]
        .as_str()
        .expect("the error field survives as text");
    assert_eq!(
        error.chars().count(),
        512,
        "a staged error is truncated, not dropped: {} characters",
        error.chars().count()
    );
    assert!(error.starts_with("eeee"), "{error}");
}

/// The state an interrupted earlier attempt leaves behind: a body holding a
/// **prefix** of one field and never sealed, bound to an entry whose header has
/// not been written yet. `stage_named` sees exactly this when a process dies
/// between the append loop and the seal.
fn staged_prefix(prefix: &str) -> (tempfile::TempDir, SqliteStore, i64) {
    let root = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&root.path().join("agent.db")).unwrap();
    store.reserve_segmented("s").unwrap();
    store
        .replace(
            "s",
            vec![json!({"id":"info","type":"session_info","timestamp":"2026-10-26T00:00:00Z","content":{}})],
        )
        .unwrap();
    let piece = prefix.to_owned();
    let body = store
        .db
        .call(move |db| {
            let tx = crate::session::database::begin_write_transaction(db)?;
            let id = create(&tx)?;
            let range = append(&tx, id, 0, &piece)?;
            super::super::bind(&tx, "s", "entry", "a", "/content/0/text", range)?;
            tx.commit()?;
            Ok(id)
        })
        .unwrap();
    (root, store, body)
}

fn body_of(store: &SqliteStore, body: i64) -> super::super::ContentPage {
    store
        .db
        .call(move |db| super::super::read(db, body, 0, 4096))
        .unwrap()
}

/// A retry of an unsealed staging verifies the bytes an earlier attempt left
/// and then finishes the field.
///
/// Without the verification the retry would append from its own remembered
/// offset and publish bytes that never agree with what is stored — the failure
/// mode is a body that reads as two different texts depending on which page you
/// look at. So the assertion is on the *content* of the finished body, not on
/// the fact that the call returned.
#[test]
fn a_retry_of_an_unsealed_staging_verifies_the_prefix_and_finishes_it() {
    let (_root, store, body) = staged_prefix("partial");
    let range = stage_named(
        &store,
        "s",
        "entry",
        "a",
        "/content/0/text",
        "partial and the rest",
        false,
    )
    .unwrap();
    assert_eq!(range.length, 20);
    let page = body_of(&store, body);
    assert_eq!(page.text, "partial and the rest");
    assert!(
        page.sealed,
        "the retry seals the body the first attempt left open"
    );
}

/// The same retry with a prefix that disagrees is refused, and refuses before
/// writing anything.
#[test]
fn a_retry_whose_staged_prefix_disagrees_is_refused_without_appending() {
    let (_root, store, body) = staged_prefix("partial");
    let error = stage_named(
        &store,
        "s",
        "entry",
        "a",
        "/content/0/text",
        "different and the rest",
        false,
    )
    .unwrap_err();
    let message = format!("{error:#}");
    assert!(
        message.contains("conflicting staged entry prefix"),
        "{message}"
    );
    let page = body_of(&store, body);
    assert_eq!(page.text, "partial", "the refused retry appended nothing");
    assert!(!page.sealed, "and it left the body as it found it");
}
