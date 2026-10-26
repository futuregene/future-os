use super::*;

#[test]
fn canonical_search_matches_legacy_semantics_across_segment_and_read_page_edges() {
    let root = tempfile::tempdir().unwrap();
    let manager = Manager::new(root.path().join("sessions"));
    let store = manager.storage().unwrap();
    store.reserve_segmented("s").unwrap();
    let prefix = "中🙂".repeat(crate::session::content::READ_BYTES / 7 - 1);
    let text = format!("{prefix}边界NeEdLe🙂\\n{}", "尾部".repeat(1000));
    store.replace("s",vec![
        json!({"id":"u","type":"user","role":"user","timestamp":"2026-10-26T00:00:00Z","content":"literal %_ and needle"}),
        json!({"id":"a","type":"assistant","role":"assistant","timestamp":"2026-10-26T00:00:01Z","meta":{"run_id":"r"},"content":[
            {"type":"text","text":text},
            {"type":"reasoning","text":"hidden-only","provider_metadata":{"signature":"signature".repeat(3000)}},
            {"type":"tool_call","id":"Exact-ID","name":"read","args":{"path":"/中文/Needle.swift","data":"参数".repeat(500)}}]}),
        json!({"id":"t","type":"tool","role":"tool","timestamp":"2026-10-26T00:00:02Z","meta":{"run_id":"r"},"content":[
            {"type":"tool_result","tool_call_id":"Exact-ID","content":"ab中文🙂\u{0000}tail needle","is_error":true},
            {"type":"tool_result","tool_call_id":"Empty-ID","content":""}]}),
    ]).unwrap();
    let queries = [
        "needle",
        "边界needle🙂",
        "Exact-ID",
        "Empty-ID",
        "%_",
        "hidden-only",
        "中文",
        "不存在",
    ];
    let mut expected = Vec::new();
    for query in queries {
        let query = query.to_owned();
        expected.push(
            store
                .db
                .read(move |db| search_session_matches(db, "s", &query, 20))
                .unwrap(),
        );
    }
    store.db.call(|db| {
        db.execute_batch("DROP VIEW entry_records; DROP VIEW resolved_message_blocks; DROP VIEW block_records")?;
        db.execute("DELETE FROM content_segments WHERE content_id IN (SELECT content_id FROM content_references WHERE session_id='s' AND field='/content/1/provider_metadata')",[])?;
        Ok(())
    }).unwrap();
    for (query, expected) in queries.into_iter().zip(expected) {
        let actual = manager.search_history("s", query, 20).unwrap();
        assert_eq!(actual["matches"], json!(expected), "query: {query}");
        for hit in actual["matches"].as_array().unwrap() {
            let fragment = manager
                .read_history_entry(
                    "s",
                    hit["entryId"].as_str().unwrap(),
                    hit["byteOffset"].as_i64().unwrap(),
                    1000,
                )
                .unwrap();
            assert!(
                fragment["totalBytes"].as_i64().unwrap() >= hit["byteOffset"].as_i64().unwrap()
            );
        }
    }
}

#[test]
fn mixed_format_cross_session_search_preserves_order_limits_and_exact_scope() {
    let root = tempfile::tempdir().unwrap();
    let manager = Manager::new(root.path().join("sessions"));
    let store = manager.storage().unwrap();
    for (session, canonical, day) in [("legacy", false, 25), ("canonical", true, 26)] {
        if canonical {
            store.reserve_segmented(session).unwrap();
        }
        let entries=(0..4).map(|i|json!({"id":format!("{session}-{i}"),"type":"user","role":"user","timestamp":format!("2026-10-{day}T00:00:0{i}Z"),"content":format!("needle {session} {i}")})).collect();
        store.replace(session, entries).unwrap();
    }
    let hits = manager.search_history_all("needle", 2, 2).unwrap();
    assert_eq!(hits["scannedSessions"], 2);
    assert_eq!(hits["hasMore"], true);
    assert_eq!(hits["truncated"], false);
    assert_eq!(hits["matches"][0]["entryId"], "canonical-3");
    assert_eq!(hits["matches"][1]["entryId"], "canonical-2");
    let recent = manager.search_history_all("needle", 20, 1).unwrap();
    assert_eq!(recent["scannedSessions"], 1);
    assert_eq!(recent["truncated"], true);
    assert!(recent["matches"]
        .as_array()
        .unwrap()
        .iter()
        .all(|hit| hit["sessionId"] == "canonical"));
    assert!(
        manager.search_history("canonical", "legacy", 20).unwrap()["matches"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn canonical_original_ranges_cross_blocks_without_loading_unrelated_protocol_bodies() {
    let root = tempfile::tempdir().unwrap();
    let manager = Manager::new(root.path().join("sessions"));
    let store = manager.storage().unwrap();
    store.reserve_segmented("s").unwrap();
    let first = "正文🙂".repeat(10_000);
    let args = json!({"path":"/tmp/中文","body":"参数\"🙂".repeat(500)});
    let last = "尾部";
    store.replace("s",vec![json!({"id":"a","type":"assistant","role":"assistant",
        "timestamp":"2026-10-26T00:00:00Z","meta":{"run_id":"r"},"content":[
            {"type":"text","text":first},
            {"type":"reasoning","text":"hidden","provider_metadata":{"signature":"signature".repeat(5000)}},
            {"type":"tool_call","id":"call","name":"edit","args":args},
            {"type":"text","text":last}]})]).unwrap();
    store.db.call(|db| {
        db.execute_batch("DROP VIEW entry_records; DROP VIEW resolved_message_blocks")?;
        db.execute("DELETE FROM content_segments WHERE content_id IN (SELECT content_id FROM content_references WHERE session_id='s' AND owner_id='a' AND field='/content/1/provider_metadata')",[])?;
        Ok(())
    }).unwrap();
    let expected = format!("{first}{}{last}", args);
    let mut actual = String::new();
    let mut offset = 0;
    loop {
        let page = manager.read_history_entry("s", "a", offset, 8191).unwrap();
        assert_eq!(page["totalBytes"], expected.len());
        assert_eq!(page["omittedKinds"], json!(["reasoning"]));
        let before = actual.len();
        for chunk in page["chunks"].as_array().unwrap() {
            actual.push_str(chunk["text"].as_str().unwrap());
            assert!(matches!(chunk["blockIndex"].as_i64(), Some(0 | 2 | 3)));
        }
        assert!(actual.len() > before);
        if page["hasMore"] == false {
            break;
        }
        offset = page["nextOffset"].as_i64().unwrap();
        assert_eq!(offset as usize, actual.len());
    }
    assert_eq!(actual, expected);
    assert!(manager.read_history_entry("s", "a", 1, 40).is_err());
    assert!(manager
        .read_history_entry("s", "a", expected.len() as i64 + 1, 40)
        .is_err());
    assert!(manager.read_history_entry("s", "foreign", 0, 40).is_err());
}
fn setup() -> (tempfile::TempDir, Manager) {
    let temp = tempfile::tempdir().unwrap();
    let m = Manager::new(temp.path().to_owned());
    m.storage().unwrap().replace("s",vec![
        json!({"id":"u","type":"user","role":"user","timestamp":"2026-01-01T00:00:00Z","content":[{"type":"text","text":"中文 ExpoSharing literal %_"}]}),
        json!({"id":"a","type":"assistant","role":"assistant","timestamp":"2026-01-01T00:00:01Z","content":[{"type":"reasoning","text":"hidden-only"},{"type":"text","text":"abc"},{"type":"tool_call","id":"call-1","name":"read","args":{"path":"ShareInbox.swift"}}]}),
        json!({"id":"t","type":"tool","role":"tool","timestamp":"2026-01-01T00:00:02Z","content":[{"type":"tool_result","tool_call_id":"call-1","content":"ab中文🙂\u{0000}tail ExpoSharing","is_error":true}]}),
    ]).unwrap();
    m.storage().unwrap().replace("other",vec![json!({"id":"foreign","type":"user","role":"user","timestamp":"2026-01-01T00:00:00Z","content":"OTHER ExpoSharing"})]).unwrap();
    (temp, m)
}
#[test]
fn search_is_scoped_literal_bounded_and_excludes_reasoning() {
    let (_t, m) = setup();
    let revision = m.session_revision("s").unwrap();
    let r = m.search_history("s", "exposharing", 1).unwrap();
    assert_eq!(r["matches"].as_array().unwrap().len(), 1);
    assert_eq!(r["hasMore"], true);
    assert_eq!(r["matches"][0]["entryId"], "t");
    assert!(m.search_history("s", "hidden-only", 5).unwrap()["matches"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        m.search_history("s", "%_", 5).unwrap()["matches"][0]["entryId"],
        "u"
    );
    assert_eq!(
        m.search_history("s", "ShareInbox", 5).unwrap()["matches"][0]["kind"],
        "tool_call"
    );
    assert!(m.search_history("missing", "x", 5).is_err());
    assert!(m.search_history("s", " ", 5).is_err());
    assert!(m.search_history("s", "x", 21).is_err());
    assert!(revision == m.session_revision("s").unwrap());
}
#[test]
fn indexed_read_preserves_unicode_nul_and_pages_without_loss() {
    let (_t, m) = setup();
    let mut offset = 0;
    let mut all = String::new();
    loop {
        let r = m.read_history_entry("s", "t", offset, 4).unwrap();
        for c in r["chunks"].as_array().unwrap() {
            all.push_str(c["text"].as_str().unwrap());
            assert_eq!(c["toolCallId"], "call-1");
            assert_eq!(c["isError"], true);
        }
        if !r["hasMore"].as_bool().unwrap() {
            break;
        }
        let next = r["nextOffset"].as_i64().unwrap();
        assert!(next > offset);
        offset = next;
    }
    assert_eq!(all, "ab中文🙂\u{0000}tail ExpoSharing");
    assert!(m.read_history_entry("s", "t", 3, 4).is_err());
    assert!(m.read_history_entry("s", "foreign", 0, 100).is_err());
    assert!(m.read_history_entry("s", "t", 999, 100).is_err());
    assert!(m.read_history_entry("s", "t", -1, 100).is_err());
    assert!(m.read_history_entry("s", "t", 0, 3).is_err());
}
#[test]
fn read_returns_block_ids_arguments_and_omissions() {
    let (_t, m) = setup();
    let r = m.read_history_entry("s", "a", 0, 8192).unwrap();
    let c = r["chunks"].as_array().unwrap();
    assert_eq!(c.len(), 2);
    assert_eq!(c[0]["blockIndex"], 1);
    assert_eq!(c[1]["blockIndex"], 2);
    assert_eq!(c[1]["toolName"], "read");
    assert!(c[1]["text"].as_str().unwrap().contains("ShareInbox.swift"));
    assert_eq!(r["omittedKinds"], json!(["reasoning"]));
    let start = m.search_history("s", "ShareInbox", 5).unwrap()["matches"][0]["byteOffset"]
        .as_i64()
        .unwrap();
    assert!(
        m.read_history_entry("s", "a", start, 8192).unwrap()["chunks"][0]["text"]
            .as_str()
            .unwrap()
            .contains("ShareInbox")
    );
}
#[test]
fn ids_are_searchable_and_entry_lookup_uses_the_identity_index() {
    let (_t, m) = setup();
    let hits = m.search_history("s", "call-1", 5).unwrap();
    assert_eq!(hits["matches"].as_array().unwrap().len(), 2);
    assert_eq!(hits["matches"][0]["entryId"], "t");
    assert!(m.search_history("s", "' OR 1=1 --", 5).unwrap()["matches"]
        .as_array()
        .unwrap()
        .is_empty());
    let plan = m.storage().unwrap().db.call(|db| {
        let mut stmt = db.prepare("EXPLAIN QUERY PLAN SELECT position FROM entries WHERE session_id=?1 AND entry_id=?2")?;
        let rows = stmt.query_map(params!["s", "t"], |row| row.get::<_, String>(3))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.join("\n"))
    }).unwrap();
    assert!(plan.contains("entries_identity_unique"), "{plan}");
}

/// The snippet window is a *byte* slice of concatenated UTF-8, so its edges
/// land inside a multi-byte character whenever the match sits at a residue
/// other than 0 mod (char width). Decoding such a window lossily turns the
/// halves into U+FFFD, which a terminal draws as a box — the reported bug.
/// Sweep the match across 100 offsets so every residue is exercised, and
/// only trim: the query must survive and `byteOffset` must still point at it.
#[test]
fn snippets_keep_character_boundaries_at_every_match_offset() {
    let temp = tempfile::tempdir().unwrap();
    let m = Manager::new(temp.path().to_owned());
    let needle = "ExpoSharing";
    for pad in 0..100 {
        // A mix of 3-byte CJK and 4-byte emoji before the match keeps
        // `hit-120` inside a character for most residues (a pure 3-byte
        // prefix would stay aligned, since 120 is divisible by 3), and a
        // long emoji tail pushes the trailing edge past 480 bytes without
        // running off the end of the body.
        let body = format!(
            "{}{}{needle}{}",
            "汉".repeat(90 + pad),
            "🙂".repeat(pad % 7),
            "🙂".repeat(150)
        );
        m.storage()
            .unwrap()
            .replace(
                "s",
                vec![json!({"id":"u","type":"user","role":"user","timestamp":"2026-01-01T00:00:00Z","content": body})],
            )
            .unwrap();
        let r = m.search_history("s", needle, 1).unwrap();
        let hit = &r["matches"][0];
        let snippet = hit["snippet"].as_str().unwrap();
        assert!(
            snippet.contains(needle),
            "pad {pad}: snippet lost the query: {snippet:?}"
        );
        assert!(
            !snippet.contains('\u{FFFD}'),
            "pad {pad}: replacement character in {snippet:?}"
        );
        assert!(
            body.contains(snippet),
            "pad {pad}: snippet is not a contiguous slice of the body"
        );
        let offset = hit["byteOffset"].as_u64().unwrap() as usize;
        assert!(
            body[offset..].starts_with(needle),
            "pad {pad}: byteOffset {offset} no longer points at the match"
        );
    }
}

/// `trim_partial_characters` may only ever drop an *incomplete* character;
/// a window that happens to end on a boundary stays byte-identical.
#[test]
fn snippet_edges_drop_only_incomplete_characters() {
    let cjk = [0xE4, 0xB8, 0xAD]; // 中 (3 bytes)
    let emoji = [0xF0, 0x9F, 0x99, 0x82]; // 🙂 (4 bytes)

    // Leading continuation bytes belong to the character before the window.
    let mut leading = vec![0xAD];
    leading.extend_from_slice(&cjk);
    assert_eq!(trim_partial_characters(&leading), cjk);
    // Incomplete characters on either edge disappear entirely.
    assert_eq!(trim_partial_characters(&cjk[..2]), &[] as &[u8]);
    assert_eq!(trim_partial_characters(&emoji[..3]), &[] as &[u8]);
    assert_eq!(trim_partial_characters(&[emoji[0], emoji[1]]), &[] as &[u8]);
    // Whole characters and ASCII are never touched.
    assert_eq!(trim_partial_characters(&cjk), cjk);
    assert_eq!(trim_partial_characters(&emoji), emoji);
    let mut mixed = b"abc".to_vec();
    mixed.extend_from_slice(&cjk);
    mixed.extend_from_slice(b"def");
    mixed.extend_from_slice(&emoji);
    assert_eq!(trim_partial_characters(&mixed), mixed.as_slice());
    let mut trailing = b"abc".to_vec();
    trailing.extend_from_slice(&emoji[..3]);
    assert_eq!(trim_partial_characters(&trailing), b"abc");
    assert_eq!(trim_partial_characters(b""), b"");
}

#[test]
fn very_large_single_line_is_bounded() {
    let (_t, m) = setup();
    m.storage().unwrap().append("s",vec![json!({"id":"large","type":"user","role":"user","timestamp":"2026-01-01T00:00:03Z","content":"z".repeat(2_000_000)})]).unwrap();
    let r = m.read_history_entry("s", "large", 0, 8192).unwrap();
    assert_eq!(r["totalBytes"], 2_000_000);
    assert_eq!(r["nextOffset"], 8192);
    assert!(r.to_string().len() < 10_000);
}

/// Pin a session's position in the cross-session scan order. `replace`
/// stamps `updated_at_ms` with the wall clock, so two sessions created in
/// the same millisecond would otherwise be ordered by the id tiebreaker.
fn pin_session(m: &Manager, session: &str, updated_at_ms: i64, name: Option<&str>) {
    let session = session.to_owned();
    let metadata = name.map(|name| json!({"session_name": name}).to_string());
    m.storage()
        .unwrap()
        .db
        .call(move |db| {
            db.execute(
                "UPDATE sessions SET updated_at_ms=?1, current_metadata_json=coalesce(?2,current_metadata_json) WHERE id=?3",
                params![updated_at_ms, metadata, session],
            )?;
            Ok(())
        })
        .unwrap();
}

#[test]
fn cross_session_search_labels_each_match_and_orders_by_timestamp() {
    let (_t, m) = setup();
    pin_session(&m, "s", 2_000, Some("Alpha"));
    pin_session(&m, "other", 1_000, None);
    let r = m.search_history_all("exposharing", 20, 50).unwrap();
    let matches = r["matches"].as_array().unwrap();
    assert_eq!(r["scannedSessions"], 2);
    assert_eq!(r["truncated"], false);
    assert_eq!(r["hasMore"], false);
    // Newest first: the tool result (00:02) precedes the two 00:00 hits.
    assert_eq!(matches.len(), 3);
    assert_eq!(matches[0]["entryId"], "t");
    assert_eq!(matches[0]["sessionId"], "s");
    assert_eq!(matches[0]["sessionName"], "Alpha");
    assert_eq!(matches[1]["entryId"], "u");
    assert_eq!(matches[1]["sessionId"], "s");
    // A session with no title simply omits the field.
    assert_eq!(matches[2]["entryId"], "foreign");
    assert_eq!(matches[2]["sessionId"], "other");
    assert!(matches[2].get("sessionName").is_none());
    // The per-session payload is unchanged apart from the two added keys.
    assert_eq!(matches[0]["kind"], "tool_result");
    assert!(matches[0]["snippet"]
        .as_str()
        .unwrap()
        .contains("ExpoSharing"));
}

#[test]
fn cross_session_search_bounds_matches_and_the_session_scan() {
    let (_t, m) = setup();
    pin_session(&m, "s", 2_000, None);
    pin_session(&m, "other", 1_000, None);
    let r = m.search_history_all("exposharing", 1, 50).unwrap();
    assert_eq!(r["matches"].as_array().unwrap().len(), 1);
    assert_eq!(r["hasMore"], true);

    // max_sessions cuts the scan to the most recently updated sessions and
    // says so, instead of quietly reporting an incomplete history.
    let r = m.search_history_all("exposharing", 20, 1).unwrap();
    assert_eq!(r["scannedSessions"], 1);
    assert_eq!(r["truncated"], true);
    assert!(r["matches"]
        .as_array()
        .unwrap()
        .iter()
        .all(|m| m["sessionId"] == "s"));
}

#[test]
fn cross_session_search_validates_bounds_before_touching_storage() {
    let (_temp, m) = setup();
    assert!(m.search_history_all("   ", 5, 50).is_err());
    assert!(m.search_history_all(&"q".repeat(200), 5, 50).is_ok());
    assert!(m.search_history_all(&"q".repeat(201), 5, 50).is_err());
    assert!(m.search_history_all("q", 0, 50).is_err());
    assert!(m.search_history_all("q", 21, 50).is_err());
    assert!(m.search_history_all("q", 5, 0).is_err());
    assert!(m.search_history_all("q", 5, 501).is_err());
    assert!(m.search_history_all("q", 5, 500).is_ok());
}

/// The cross-session hit SQL is the single-session one, so a query with no
/// matches must come back empty rather than falling back to any session.
#[test]
fn cross_session_search_returns_no_matches_for_an_absent_literal() {
    let (_t, m) = setup();
    let r = m
        .search_history_all("nothing-like-this-anywhere", 20, 50)
        .unwrap();
    assert!(r["matches"].as_array().unwrap().is_empty());
    assert_eq!(r["hasMore"], false);
    // Both fixture sessions are searched even though neither matches.
    assert_eq!(r["scannedSessions"], 2);
}

#[test]
fn out_of_range_search_and_read_requests_are_rejected_before_touching_storage() {
    let (_temp, m) = setup();
    assert!(m.search_history("", "query", 5).is_err());
    assert!(m.search_history("s", "   ", 5).is_err());
    // The query bound is on characters, not bytes.
    assert!(m.search_history("s", &"q".repeat(200), 20).is_ok());
    assert!(m.search_history("s", &"q".repeat(201), 5).is_err());
    assert!(m.search_history("s", "q", 0).is_err());
    assert!(m.search_history("s", "q", 21).is_err());

    assert!(m.read_history_entry("s", "", 0, 8).is_err());
    assert!(m.read_history_entry("s", "u", -1, 8).is_err());
    assert!(m.read_history_entry("s", "u", 0, 3).is_err());
    assert!(m.read_history_entry("s", "u", 0, 40_000).is_err());
    assert!(m.read_history_entry("s", "missing-entry", 0, 8).is_err());
    let error = m
        .read_history_entry("s", "u", 1_000, 8)
        .unwrap_err()
        .to_string();
    assert!(error.contains("readable bytes"), "{error}");
}

/// The session gate every history read goes through. A damaged `sessions` table
/// and a damaged `legacy_imports` table must both surface their own cause: the
/// gate exists so a reader never treats "I cannot tell" as "this session is
/// empty", and a skipped import must never look like readable history.
#[test]
fn the_session_gate_reports_each_damaged_table_it_needs() {
    use rusqlite::Connection;

    let db = Connection::open_in_memory().unwrap();
    let error = require_session(&db, "s").unwrap_err();
    assert!(format!("{error:#}").contains("sessions"), "{error:#}");

    // `sessions` is there, the import bookkeeping is not: still an error, and
    // it names the table it could not consult.
    db.execute_batch("CREATE TABLE sessions(id TEXT, revision INTEGER)")
        .unwrap();
    let error = require_session(&db, "s").unwrap_err();
    assert!(format!("{error:#}").contains("legacy_imports"), "{error:#}");

    // With both tables intact the gate answers for a real session and refuses
    // for an unknown one.
    db.execute_batch(
        "CREATE TABLE legacy_imports(session_id TEXT, status TEXT);
         INSERT INTO sessions VALUES ('known', 1);",
    )
    .unwrap();
    require_session(&db, "known").unwrap();
    let error = require_session(&db, "missing").unwrap_err();
    assert!(error.to_string().contains("session not found"), "{error:#}");
}
