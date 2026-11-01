//! Specifications authored during implementation; run in the final test phase.
use super::*;

fn store() -> (tempfile::TempDir, SqliteStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    store.reserve_segmented("s").unwrap();
    store
        .replace(
            "s",
            vec![json!({"id":"info","type":"session_info",
        "timestamp":"2026-10-26T00:00:00Z","content":{"model":"mock"}})],
        )
        .unwrap();
    (dir, store)
}

fn message(id: &str) -> Value {
    json!({"id":id,"type":"assistant","role":"assistant",
        "timestamp":"2026-10-26T00:00:01+08:00","meta":{"run_id":"r"},
        "content":[{"type":"text","text":"跨页🙂".repeat(70_000)},
            {"type":"reasoning","text":"思考","provider_metadata":{"signature":"s".repeat(10_000)}},
            {"type":"tool_call","id":"tool","name":"edit","args":{"body":"正文".repeat(10_000)}},
            {"type":"image_url","image_url":{"url":"data:image/png;base64,".repeat(2000)}},
            {"type":"future_block","text":{"body":"non-string".repeat(1000)},"x/y~z":{"state":"opaque".repeat(2000)}},
            "scalar extension".repeat(1000)]})
}

#[test]
fn paged_reader_matches_lossless_compatibility_view_then_works_without_that_view() {
    let (_dir, store) = store();
    store.append("s", vec![message("a")]).unwrap();
    let expected: Vec<Value> = store
        .db
        .read(|db| {
            let mut query = db.prepare(
                "SELECT payload FROM entry_records WHERE session_id='s' ORDER BY position",
            )?;
            let rows = query
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows.into_iter()
                .map(|raw| Ok(serde_json::from_str(&raw)?))
                .collect()
        })
        .unwrap();
    let actual = store.entries("s").unwrap();
    assert_eq!(actual, expected);
    store
        .db
        .read(|db| {
            let raw: String = db.query_row(
                "SELECT payload FROM entry_header_records WHERE session_id='s' AND entry_id='a'",
                [],
                |row| row.get(0),
            )?;
            let header: Value = serde_json::from_str(&raw)?;
            assert!(
                header["content"][4]["text"].is_object(),
                "thin headers must retain the JSON-valued text sentinel"
            );
            Ok(())
        })
        .unwrap();
    store.db.call(|db| {
        db.execute_batch("DROP VIEW entry_records; DROP VIEW block_records; DROP VIEW resolved_message_blocks")?;
        Ok(())
    }).unwrap();
    assert_eq!(store.entries("s").unwrap(), expected);
}

#[test]
fn full_transcript_hydration_is_bounded_by_pages_not_entries() {
    let (_dir, store) = store();
    let messages = (0..100)
        .map(|index| {
            json!({"id":format!("m{index:03}"),"type":"user","role":"user",
                "timestamp":"2026-10-26T00:00:01Z","content":format!("message {index}")})
        })
        .collect::<Vec<_>>();
    store.append("s", messages).unwrap();
    let before = store.db.metrics()["readerJob"]["count"].as_u64().unwrap();
    let entries = store.entries("s").unwrap();
    let after = store.db.metrics()["readerJob"]["count"].as_u64().unwrap();
    assert_eq!(entries.len(), 101);
    assert!(
        after - before <= 12,
        "100 entries used {} read jobs; expected paged metadata/body assembly",
        after - before
    );
}

#[test]
fn appends_preserve_the_captured_prefix_but_a_rewrite_invalidates_it() {
    let (_dir, store) = store();
    store.append("s", vec![message("a")]).unwrap();
    let snapshot = Snapshot::capture(&store, "s").unwrap();
    let prefix = snapshot.page(&store, -1).unwrap();
    store.append("s", vec![message("b")]).unwrap();
    assert_eq!(snapshot.page(&store, -1).unwrap().len(), 2);
    assert_eq!(snapshot.entry(&store, &prefix[1]).unwrap()["id"], "a");
    let rows = store.entries("s").unwrap();
    store.replace("s", rows).unwrap();
    assert!(snapshot
        .page(&store, -1)
        .unwrap_err()
        .to_string()
        .contains("transcript changed"));
    assert!(snapshot.entry(&store, &prefix[1]).is_err());
}

#[test]
fn every_body_page_rechecks_reference_ownership_and_restore_identity() {
    let (_dir, store) = store();
    store.append("s", vec![message("a")]).unwrap();
    let scope = Snapshot::capture(&store, "s").unwrap();
    let entry = scope.page(&store, -1).unwrap().pop().unwrap();
    let reference = scope
        .references(&store, &entry, "")
        .unwrap()
        .into_iter()
        .find(|reference| reference.field == "/content/0/text")
        .unwrap();
    store.db.call(|db| {
        db.execute("UPDATE content_references SET byte_length=byte_length-1 WHERE session_id='s' AND owner_id='a' AND field='/content/0/text'",[])?;
        Ok(())
    }).unwrap();
    assert!(scope
        .body(&store, &entry, &reference)
        .unwrap_err()
        .to_string()
        .contains("reference changed"));
    store.db.call(|db| {
        db.execute("UPDATE storage_meta SET value='replaced-restore-generation' WHERE key='restore_generation'",[])?;
        Ok(())
    }).unwrap();
    assert!(scope.verify(&store).is_err());
}

#[test]
fn deletion_invalidates_a_snapshot_without_granting_another_sessions_body() {
    let (_dir, store) = store();
    store.append("s", vec![message("a")]).unwrap();
    let scope = Snapshot::capture(&store, "s").unwrap();
    let entry = scope.page(&store, -1).unwrap().pop().unwrap();
    store.delete("s").unwrap();
    assert!(scope.entry(&store, &entry).is_err());
    assert!(scope.verify(&store).is_err());
}

#[test]
fn conflicting_json_encoding_never_changes_an_already_published_field() {
    let (_dir, store) = store();
    let object = json!({"body":"opaque".repeat(2000)});
    let raw = object.to_string();
    let entry = json!({"id":"a","type":"assistant","role":"assistant",
        "timestamp":"2026-10-26T00:00:00Z","meta":{"opaque":raw},"content":"answer"});
    store.append("s", vec![entry.clone()]).unwrap();
    let mut conflicting = entry;
    conflicting["meta"]["opaque"] = object;
    assert!(store.append("s", vec![conflicting]).is_err());
    assert_eq!(store.entries("s").unwrap()[1]["meta"]["opaque"], raw);
}

#[test]
fn a_published_recovered_prefix_cannot_be_extended_by_a_generic_entry_retry() {
    let (_dir, store) = store();
    store
        .db
        .call(|db| {
            let tx = crate::session::database::begin_write_transaction(db)?;
            let id = super::super::create(&tx)?;
            let range = super::super::append(&tx, id, 0, "committed prefix")?;
            super::super::bind(&tx, "s", "entry", "recovered", "/content/0/text", range)?;
            crate::session::sqlite_store::insert_transcript_entries(
                &tx,
                "s",
                vec![
                    json!({"id":"recovered","type":"assistant","role":"assistant",
            "timestamp":"2026-10-26T00:00:01Z","content":[{"type":"text","text":""}]}),
                ],
            )?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();
    assert!(super::super::entries::stage_named(
        &store,
        "s",
        "entry",
        "recovered",
        "/content/0/text",
        "committed prefix plus unacknowledged tail",
        false
    )
    .is_err());
    let entry = store.entries("s").unwrap().pop().unwrap();
    assert_eq!(entry["content"][0]["text"], "committed prefix");
}

#[test]
fn a_reader_does_not_wait_for_an_unrelated_writer_transaction() {
    let (_dir, store) = store();
    store.append("s", vec![message("a")]).unwrap();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let writer = store.clone();
    let thread = std::thread::spawn(move || {
        writer.db.call(move |db| {
            let tx = crate::session::database::begin_write_transaction(db)?;
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            tx.commit()?;
            Ok(())
        })
    });
    started_rx.recv().unwrap();
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let reader = store.clone();
    let reader_thread = std::thread::spawn(move || {
        let result = Snapshot::capture(&reader, "s").and_then(|scope| {
            let entry = scope.page(&reader, -1)?.pop().unwrap();
            scope.entry(&reader, &entry)
        });
        let _ = result_tx.send(result);
    });
    // The timeout bounds the failure path too: release/join the writer before
    // asserting, so a reader accidentally using the writer never hangs the suite.
    let result = result_rx.recv_timeout(std::time::Duration::from_secs(10));
    release_tx.send(()).unwrap();
    thread.join().unwrap().unwrap();
    reader_thread.join().unwrap();
    assert_eq!(
        result.expect("read queued behind held writer").unwrap()["id"],
        "a"
    );
}

#[test]
fn clearing_externalized_text_or_json_is_a_conflicting_identity_even_when_the_header_matches() {
    let (_dir, store) = store();
    let entry = json!({"id":"a","type":"assistant","role":"assistant","timestamp":"2026-10-26T00:00:00Z",
        "content":[{"type":"text","text":"large body".repeat(2000)},{"type":"future","state":{"body":"large state".repeat(2000)}}]});
    store.append("s", vec![entry.clone()]).unwrap();
    let mut cleared = entry.clone();
    cleared["content"][0]["text"] = json!("");
    assert!(store.append("s", vec![cleared]).is_err());
    let mut cleared = entry.clone();
    cleared["content"][1]["state"] = json!({});
    assert!(store.append("s", vec![cleared]).is_err());
    assert_eq!(store.entries("s").unwrap()[1]["content"], entry["content"]);
}

#[test]
fn metadata_reads_ignore_ordinary_appends_but_fence_configuration_changes() {
    let (_dir, store) = store();
    let scope = Snapshot::capture(&store, "s").unwrap();
    let info = scope.page(&store, -1).unwrap().remove(0);
    let expected = store
        .db
        .read(|db| super::super::metadata::generation(db, "s"))
        .unwrap();
    store.append("s", vec![message("a")]).unwrap();
    assert!(scope
        .column(&store, &info, Column::SessionMetadata, Some(expected))
        .is_ok());
    store.append("s",vec![json!({"id":"model","type":"model_change","timestamp":"2026-10-26T00:00:02Z","content":{"model":"changed"}})]).unwrap();
    assert!(scope
        .column(&store, &info, Column::SessionMetadata, Some(expected))
        .is_err());
    assert_eq!(
        scope.entry(&store, &info).unwrap()["content"]["model"],
        "changed"
    );
}

/// Search and recall are the only history reads that walk *every* visible
/// field, so they own the guarantees that invisible extensions are reported as
/// omitted and that a byte offset can never split a UTF-8 scalar.
#[test]
fn history_search_and_recall_walk_only_the_visible_fields() {
    let (_dir, store) = store();
    store.append("s", vec![message("a")]).unwrap();

    // A match in the first bounded page of the first block carries its block
    // identity, its byte offset inside the entry and a snippet around it.
    let hits = Snapshot::search(&store, "s", "跨页", 5).unwrap();
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["entryId"], "a");
    assert_eq!(hits[0]["blockIndex"], 0);
    assert_eq!(hits[0]["kind"], "text");
    assert_eq!(hits[0]["byteOffset"], 0);
    assert!(hits[0]["snippet"].as_str().unwrap().starts_with("跨页"));

    // A hit in the tool arguments is reported as arguments, not as text.
    let hits = Snapshot::search(&store, "s", "正文", 5).unwrap();
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["kind"], "tool_call");
    assert_eq!(hits[0]["toolCallId"], "tool");
    assert_eq!(hits[0]["toolName"], "edit");

    // Three hits in one entry: a result budget of one stops at the second hit
    // instead of walking the rest of the entry.
    store
        .append(
            "s",
            vec![json!({"id":"b","type":"assistant","role":"assistant",
                "timestamp":"2026-10-26T00:00:02Z","meta":{"run_id":"r"},
                "content":[{"type":"text","text":"限位一"},
                    {"type":"text","text":"限位二"},
                    {"type":"text","text":"限位三"}]})],
        )
        .unwrap();
    assert_eq!(Snapshot::search(&store, "s", "限位", 1).unwrap().len(), 2);
    assert_eq!(Snapshot::search(&store, "s", "限位", 5).unwrap().len(), 3);
    // Nothing visible matches the opaque extension text.
    assert!(Snapshot::search(&store, "s", "opaque", 5)
        .unwrap()
        .is_empty());

    // The query and page budgets are refused rather than silently clamped.
    for query in ["".to_string(), "x".repeat(801)] {
        assert!(Snapshot::search(&store, "s", &query, 5).is_err());
    }
    assert!(Snapshot::search(&store, "s", "跨页", 0).is_err());
    assert!(Snapshot::search(&store, "s", "跨页", 21).is_err());

    // Recall pages one entry's visible bytes and names what it left out.
    let recalled = Snapshot::recall(&store, "s", "a", 0, 16).unwrap();
    assert_eq!(recalled["entryId"], "a");
    assert_eq!(recalled["offset"], 0);
    assert_eq!(recalled["chunks"][0]["blockIndex"], 0);
    assert_eq!(recalled["chunks"][0]["blockByteOffset"], 0);
    assert_eq!(recalled["chunks"][0]["text"], "跨页🙂跨页");
    assert_eq!(recalled["hasMore"], true);
    assert!(recalled["nextOffset"].as_i64().unwrap() > 0);
    assert_eq!(
        recalled["omittedKinds"],
        serde_json::json!(["future_block", "image_url", "reasoning"])
    );
    // Offsets are byte offsets into visible text: past the end, or inside a
    // multi-byte scalar, the read fails instead of guessing.
    let total = recalled["totalBytes"].as_i64().unwrap();
    assert!(Snapshot::recall(&store, "s", "a", total + 1, 16).is_err());
    assert!(Snapshot::recall(&store, "s", "a", 1, 4).is_err());
    // An unknown entry is named, not silently empty.
    assert!(Snapshot::recall(&store, "s", "missing", 0, 16).is_err());
}

/// A page of a small inline entry is a byte range of its text, so it must cut
/// on character boundaries in both directions: a page that ends inside a
/// character returns the characters it can serve, and an offset that starts
/// inside one is refused instead of producing replacement characters.
#[test]
fn an_inline_page_never_splits_a_character() {
    let (_dir, store) = store();
    store
        .append(
            "s",
            vec![json!({"id":"tiny","type":"user","role":"user",
                "timestamp":"2026-10-26T00:00:01Z","meta":{"run_id":"r"},
                "content":"跨页🙂"})],
        )
        .unwrap();

    // The text is small enough to stay inline in its message block.
    let recalled = Snapshot::recall(&store, "s", "tiny", 0, 4).unwrap();
    assert_eq!(recalled["chunks"][0]["kind"], "text");
    assert_eq!(recalled["totalBytes"], "跨页🙂".len() as i64);
    // Four bytes end inside the second character, so only the first is served.
    assert_eq!(recalled["chunks"][0]["text"], "跨");
    assert_eq!(recalled["hasMore"], true);
    assert_eq!(recalled["nextOffset"], 3);

    // The next page continues from the boundary the previous one stopped at.
    let next = Snapshot::recall(&store, "s", "tiny", 3, 4).unwrap();
    assert_eq!(next["chunks"][0]["text"], "页");

    // Starting inside a character is refused, never rounded.
    let error = Snapshot::recall(&store, "s", "tiny", 1, 4).unwrap_err();
    assert!(format!("{error:#}").contains("UTF-8 boundary"), "{error:#}");
    let error = Snapshot::recall(&store, "s", "tiny", 4, 4).unwrap_err();
    assert!(format!("{error:#}").contains("UTF-8 boundary"), "{error:#}");
}
