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
