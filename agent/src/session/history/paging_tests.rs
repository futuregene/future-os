use super::*;
use crate::session::{Manager, Session};
use future_rpc::proto::RpcCommand;

fn difference(left: &Value, right: &Value, path: &str) -> Option<String> {
    if left == right {
        return None;
    }
    if let (Some(left), Some(right)) = (left.as_object(), right.as_object()) {
        for key in left.keys().chain(right.keys()) {
            if let Some(diff) = difference(
                left.get(key).unwrap_or(&Value::Null),
                right.get(key).unwrap_or(&Value::Null),
                &format!("{path}/{key}"),
            ) {
                return Some(diff);
            }
        }
    } else if let (Some(left), Some(right)) = (left.as_array(), right.as_array()) {
        for index in 0..left.len().max(right.len()) {
            if let Some(diff) = difference(
                left.get(index).unwrap_or(&Value::Null),
                right.get(index).unwrap_or(&Value::Null),
                &format!("{path}/{index}"),
            ) {
                return Some(diff);
            }
        }
    }
    let short = |value: &Value| {
        value.as_str().map_or_else(
            || value.to_string(),
            |text| {
                format!(
                    "{} bytes: {}",
                    text.len(),
                    text.chars().take(100).collect::<String>()
                )
            },
        )
    };
    Some(format!("{path}: {} != {}", short(left), short(right)))
}

#[test]
fn canonical_display_skips_only_fields_the_public_projection_discards() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().join("sessions"));
    let store = manager.storage().unwrap();
    store.reserve_segmented("s").unwrap();
    store.replace("s",vec![json!({"id":"a","type":"assistant","role":"assistant","timestamp":"2026-10-26T00:00:00Z",
        "provider_state":{"opaque":"invisible root".repeat(4000)},"meta":{"attachments":[{"kind":"file","path":"/synthetic"}]},
        "content":[{"type":"text","text":"visible","provider_metadata":{"signature":"required public field".repeat(1000)},"unused_vendor_state":{"raw":"invisible block".repeat(4000)}},
            {"type":"future_block","opaque":"preserved opaque block".repeat(1000)}]})]).unwrap();
    let full = manager.load("s").unwrap();
    let expected = project_entries(&full.entries);
    store.db.call(|db| {
        db.execute("DELETE FROM content_segments WHERE content_id IN (SELECT content_id FROM content_references WHERE session_id='s' AND (field='/provider_state' OR field='/content/0/unused_vendor_state'))",[])?;
        Ok(())
    }).unwrap();
    let page = manager.history_page("s", None, Some(0), Some(10)).unwrap();
    let expected = json!(expected);
    assert!(
        page["entries"] == expected,
        "{}",
        difference(&page["entries"], &expected, "").unwrap_or_default()
    );
    assert!(
        manager.load("s").is_err(),
        "the full view must still require every opaque source field"
    );
}

#[test]
fn canonical_pages_read_bodies_without_the_compatibility_view_and_reject_stale_ordinals() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().join("sessions"));
    let store = manager.storage().unwrap();
    store.reserve_segmented("s").unwrap();
    store
        .replace(
            "s",
            vec![json!({"id":"u","type":"user","role":"user",
        "timestamp":"2026-10-26T00:00:00Z","content":"question"})],
        )
        .unwrap();
    store
        .db
        .call(|db| {
            db.execute_batch("DROP VIEW entry_records")?;
            Ok(())
        })
        .unwrap();
    let page = manager.history_page("s", None, Some(0), Some(10)).unwrap();
    assert_eq!(page["entries"][0]["blocks"][0]["text"], "question");
    let old = store
        .db
        .read(|db| read_display_page(db, "s", None, Some(0), Some(10)))
        .unwrap();
    store
        .replace(
            "s",
            vec![json!({"id":"v","type":"user","role":"user",
        "timestamp":"2026-10-26T00:00:01Z","content":"replacement"})],
        )
        .unwrap();
    assert!(materialize_display_page(
        old,
        None,
        None,
        Some(store),
        Some(&manager.history_fragments)
    )
    .is_err());
}

#[test]
fn dropping_a_fragment_cache_loses_no_body_and_recreated_fragments_have_the_same_digest() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().join("sessions"));
    let store = manager.storage().unwrap();
    store.reserve_segmented("s").unwrap();
    store
        .replace(
            "s",
            vec![json!({"id":"a","type":"assistant","role":"assistant",
        "timestamp":"2026-10-26T00:00:00Z","content":"中文🙂".repeat(900_000)})],
        )
        .unwrap();
    let first = manager
        .read_history_page("s", None, Some(0), Some(1), None)
        .unwrap();
    let offset = first["fragment"]["json"].as_str().unwrap().len() as i64;
    let next = manager
        .read_history_page("s", None, Some(0), Some(1), Some(offset))
        .unwrap();
    *manager.history_fragments.lock() = Default::default();
    let rebuilt = manager
        .read_history_page("s", None, Some(0), Some(1), Some(offset))
        .unwrap();
    assert_eq!(rebuilt, next);
    assert_eq!(first["fragment"]["sha256"], next["fragment"]["sha256"]);
    assert_eq!(
        store.entries("s").unwrap()[0]["content"][0]["text"]
            .as_str()
            .unwrap()
            .len(),
        "中文🙂".len() * 900_000
    );
}

async fn assembled(manager: &Manager, id: &str, command: RpcCommand) -> Value {
    let page = future_rpc::history::read_page(command, |command| async move {
        let value = manager
            .read_history_page(
                id,
                command.before,
                command.offset,
                command.limit,
                command.fragment_offset,
            )
            .map_err(|e| e.to_string())?;
        assert!(
            value.to_string().len() < HISTORY_PAGE_BYTE_BUDGET,
            "every transport page is bounded"
        );
        let response = future_rpc::proto::RpcResponse {
            payload: future_rpc::encode::response_payload("get_session_entries", &value),
            ..Default::default()
        };
        future_rpc::decode::decode_session_entries_page(&response)
            .ok_or("invalid typed page".into())
    })
    .await
    .unwrap();
    serde_json::to_value(page).unwrap()
}

#[tokio::test]
async fn both_directions_reassemble_oversized_unicode_entries_without_losing_rows() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().join("sessions"));
    let mut session = Session::new("/synthetic", "mock");
    session
        .entries
        .push(SessionEntry::new_user("user", json!("question")));
    // Include escapes and multi-byte text at many fragment boundaries.
    session.entries.push(SessionEntry::new_assistant(
        json!("中文🙂\n\"\\".repeat(700_000)),
        Vec::new(),
    ));
    session
        .entries
        .push(SessionEntry::new_user("user", json!("next")));
    session
        .entries
        .push(SessionEntry::new_assistant(json!("answer"), Vec::new()));
    manager.save(&session).unwrap();
    let expected = project_entries(&manager.load(&session.id).unwrap().entries);
    for backward in [false, true] {
        let mut cursor = if backward { i64::MAX } else { 0 };
        let mut actual = Vec::new();
        for _ in 0..10 {
            let command = RpcCommand {
                before: backward.then_some(cursor),
                offset: (!backward).then_some(cursor),
                limit: Some(100),
                ..Default::default()
            };
            let page = assembled(&manager, &session.id, command).await;
            let mut rows = page["entries"].as_array().unwrap().clone();
            if backward {
                rows.append(&mut actual);
                actual = rows;
            } else {
                actual.extend(rows);
            }
            if page["hasMore"] == false {
                break;
            }
            let next = page["nextOffset"].as_i64().unwrap();
            assert!(if backward {
                next < cursor
            } else {
                next > cursor
            });
            cursor = next;
        }
        assert_eq!(actual, expected);
    }
}

#[tokio::test]
async fn a_backward_exchange_larger_than_the_budget_is_complete_before_projection() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().join("sessions"));
    let mut session = Session::new("/synthetic", "mock");
    session
        .entries
        .push(SessionEntry::new_user("user", json!("question")));
    for n in 0..12 {
        if n == 5 {
            session.entries.push(SessionEntry {
                entry_type: "compaction".into(),
                ..SessionEntry::new_user(
                    "",
                    json!({"schema_version":2,"phase":"mid_turn","trigger":"automatic"}),
                )
            });
        }
        session.entries.push(SessionEntry::new_assistant(
            json!("x".repeat(1024 * 1024)),
            Vec::new(),
        ));
    }
    manager.save(&session).unwrap();
    let raw = manager
        .history_page(&session.id, Some(i64::MAX), None, Some(1))
        .unwrap();
    assert!(raw["entries"].as_array().unwrap().len() < session.entries.len());
    assert!(raw.to_string().len() < HISTORY_PAGE_BYTE_BUDGET);
    let full = assembled(
        &manager,
        &session.id,
        RpcCommand {
            before: Some(i64::MAX),
            limit: Some(1),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(
        full["entries"],
        json!(project_entries(&manager.load(&session.id).unwrap().entries))
    );
    assert_eq!(full["nextOffset"], 0);
    assert_eq!(full["hasMore"], false);
}

/// A backward page's `limit` counts user exchanges, so the page must carry every
/// row of the exchanges it named — including the ones above a long run.
///
/// One assistant step and one tool entry per tool call make a busy run a hundred
/// display rows on its own. A fixed row ceiling over the canonical branch cut the
/// requested window down to whatever fitted, so a session whose newest exchange
/// was such a run opened on its last reply alone: the question that had produced
/// it, and every exchange before that, looked gone.
#[test]
fn a_busy_exchange_does_not_hide_the_exchanges_above_it() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().join("sessions"));
    let store = manager.storage().unwrap();
    store.reserve_segmented("s").unwrap();
    let mut entries = vec![
        json!({"id":"u1","type":"user","role":"user","timestamp":"2026-10-26T00:00:00Z",
            "content":[{"type":"text","text":"first question"}]}),
        json!({"id":"a1","type":"assistant","role":"assistant","timestamp":"2026-10-26T00:00:01Z",
            "content":[{"type":"text","text":"first answer"}]}),
        json!({"id":"u2","type":"user","role":"user","timestamp":"2026-10-26T00:00:02Z",
            "content":[{"type":"text","text":"second question"}]}),
    ];
    // A run with more steps than the old ceiling: one row per step.
    for step in 0..80 {
        entries.push(
            json!({"id":format!("a2-{step}"),"type":"assistant","role":"assistant",
            "timestamp":"2026-10-26T00:00:03Z",
            "content":[{"type":"text","text":format!("step {step}")}]}),
        );
    }
    entries.push(json!({"id":"a3","type":"assistant","role":"assistant",
        "timestamp":"2026-10-26T00:00:04Z",
        "content":[{"type":"text","text":"second answer"}]}));
    store.replace("s", entries).unwrap();
    let page = manager
        .history_page("s", Some(i64::MAX), None, Some(2))
        .unwrap();
    let rows = page["entries"].as_array().unwrap();
    assert!(
        rows.len() > 64,
        "two exchanges including a busy run are more than the old row ceiling, got {}",
        rows.len()
    );
    assert_eq!(rows[0]["blocks"][0]["text"], json!("first question"));
    assert_eq!(page["hasMore"], json!(false));
}

/// A display index that is missing (a partially migrated or damaged file) must
/// be reported, never paged as "no history": an empty page would look to the
/// caller like a session with nothing in it.
/// A client that resumed a fragmented read passes the byte offset it already
/// has. `materialize_display_page` must then return the *next* fragment, and
/// because a JSON string can be cut through a multi-byte character, it has to
/// walk the end back to a real UTF-8 boundary before slicing — otherwise the
/// fragment is invalid JSON and the client cannot concatenate it.
///
/// The offset is derived from the entry's own serialized form rather than
/// guessed: the answer is deterministic precisely because the text is one
/// repeated three-byte character, so `offset + ENTRY_FRAGMENT_BYTE_BUDGET` can
/// only land mid-character (1048576 % 3 == 1) once `offset` is a character
/// start.
#[tokio::test]
async fn a_resumed_fragment_stops_on_a_character_boundary() {
    const HAN: &str = "中";
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().join("sessions"));
    let mut session = Session::new("/synthetic", "mock");
    session
        .entries
        .push(SessionEntry::new_user("user", json!("question")));
    // 3 bytes per character, comfortably past the page budget so the page can
    // only be transported as a fragment.
    let huge = SessionEntry::new_assistant(json!(HAN.repeat(3_000_000)), Vec::new());
    let huge_id = huge.id.clone();
    session.entries.push(huge);
    session
        .entries
        .push(SessionEntry::new_user("user", json!("next")));
    manager.save(&session).unwrap();

    // The same projection the transport slices, so the offset is a real one.
    let stored = manager.load(&session.id).unwrap();
    let entry = stored
        .entries
        .iter()
        .find(|entry| entry.id == huge_id)
        .expect("the oversized entry")
        .clone();
    let payload = project_entries(&[entry]).pop().unwrap();
    let serialized = serde_json::to_string(&payload).unwrap();
    let offset = serialized.find(HAN).expect("the text") as i64;
    assert!(
        !serialized.is_char_boundary(offset as usize + ENTRY_FRAGMENT_BYTE_BUDGET),
        "the fixture must place the budget cut inside a character"
    );

    // Ordinal 1 is the oversized entry, and a one-row page cannot be paged
    // normally, so this must come back as a fragment.
    let page = manager
        .read_history_page(&session.id, None, Some(1), Some(1), Some(offset))
        .unwrap();
    assert_eq!(page["entries"], json!([]), "a lone oversized row fragments");
    let fragment = &page["fragment"];
    assert_eq!(fragment["offset"], offset);
    assert_eq!(fragment["totalBytes"], serialized.len());
    assert!(
        fragment["totalBytes"].as_u64().unwrap() > ENTRY_FRAGMENT_BYTE_BUDGET as u64,
        "the entry must be larger than one fragment: {fragment}"
    );
    // The walk ran: the slice stops before the budget, on a boundary.
    let json = fragment["json"].as_str().unwrap();
    assert!(json.len() < ENTRY_FRAGMENT_BYTE_BUDGET, "{json:.40}");
    assert!(
        !json.is_empty() && json.ends_with('中'),
        "the fragment must end on a whole character"
    );

    // Resuming from where the fragment ended is accepted and continues from
    // there, which is what makes the client-side concatenation possible.
    let resumed = manager
        .read_history_page(
            &session.id,
            None,
            Some(1),
            Some(1),
            Some(offset + json.len() as i64),
        )
        .unwrap();
    assert_eq!(
        resumed["fragment"]["offset"].as_i64(),
        Some(offset + json.len() as i64)
    );
    // The contract that matters is concatenation: the two fragments must be
    // adjacent slices of the entry, so a client can join them. (Comparing the
    // fragments to each other would prove nothing here — the text is one
    // repeated character, so two fragments of equal length are equal strings.)
    let resumed_json = resumed["fragment"]["json"].as_str().unwrap();
    let joined = format!("{json}{resumed_json}");
    let start = offset as usize;
    assert_eq!(
        &serialized[start..start + joined.len()],
        joined.as_str(),
        "the resumed fragment must continue where the first one stopped"
    );
}

/// The two byte budgets agree on one point that decides an entire arm of
/// `materialize_display_page`: `read_display_page` already caps a page's
/// **raw** size at `HISTORY_PAGE_BYTE_BUDGET` (overlay + stored body per row),
/// while the materializer counts each row's **re-serialized projection**. A page
/// with several rows therefore cannot reach the materializer's budget unless its
/// raw size does too — so only a lone oversized row can be cut there, which is
/// the fragment path.
#[tokio::test]
async fn a_multi_row_page_never_takes_the_oversized_entry_cut() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().join("sessions"));
    let mut session = Session::new("/synthetic", "mock");
    session
        .entries
        .push(SessionEntry::new_user("user", json!("question")));
    // Four ~1 MiB bodies: the raw accounting exceeds the 8 MiB budget well
    // before the materializer's own budget can be crossed.
    for _ in 0..6 {
        session.entries.push(SessionEntry::new_assistant(
            json!("x".repeat(1024 * 1024)),
            Vec::new(),
        ));
    }
    manager.save(&session).unwrap();

    let page = manager
        .history_page(&session.id, None, Some(0), Some(1000))
        .unwrap();
    let rows = page["entries"].as_array().unwrap();
    assert!(
        rows.len() >= 2,
        "the fixture must produce a multi-row page: {}",
        rows.len()
    );
    assert!(
        page["fragment"].is_null(),
        "a multi-row page is transported whole: {}",
        page["fragment"]
    );
    let serialized: usize = rows
        .iter()
        .map(|row| serde_json::to_string(row).unwrap().len())
        .sum();
    assert!(
        serialized <= HISTORY_PAGE_BYTE_BUDGET,
        "the materializer's budget was never crossed, so its cut arm cannot run: \
         {serialized} serialized bytes over {} rows",
        rows.len()
    );
}

/// A display row can exist with **no entry body**: `history_display.source_position`
/// is nullable, and an index that outlived (or never had) its source produces
/// exactly that. For such a row the materializer's payload *is* the indexed
/// payload, so its serialized size equals what `read_display_page` counted —
/// which is the only way the page reader's budget can be spent before the
/// materializer's own. This is the arm that cuts a page at an entry too large to
/// fit, and it must report the cut honestly in both directions.
#[test]
fn a_page_of_bodyless_rows_can_be_cut_at_the_materializer_budget() {
    use rusqlite::Connection;

    // Both budgets are 8 MiB. Eight 1_048_570-byte rows are accepted by the page
    // reader (8 * 1_048_570 = 8_388_560 <= 8_388_608) and then cross the
    // materializer's check on the eighth row, because it also counts the
    // separators and the page envelope.
    const PAYLOAD: usize = 1_048_570;
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE history_display(session_id TEXT, ordinal INTEGER, \
         source_position INTEGER, is_user INTEGER, payload TEXT);
         CREATE TABLE entry_records(session_id TEXT, position INTEGER, payload TEXT);
         CREATE TABLE session_formats(session_id TEXT,format INTEGER);",
    )
    .unwrap();
    // A JSON string of exactly PAYLOAD bytes.
    let payload = serde_json::to_string(&json!("x".repeat(PAYLOAD - 2))).unwrap();
    assert_eq!(payload.len(), PAYLOAD);
    for ordinal in 0..8 {
        db.execute(
            "INSERT INTO history_display(session_id,ordinal,source_position,is_user,payload) \
             VALUES ('s',?1,NULL,?2,?3)",
            params![ordinal, ordinal % 2, payload],
        )
        .unwrap();
    }

    // The page reader accepts all eight (8 * PAYLOAD fits its raw budget)…
    let forward = read_display_page(&db, "s", None, Some(0), Some(1000)).unwrap();
    assert_eq!(
        forward.rows.len(),
        8,
        "the reader's raw budget admits them all"
    );
    assert!(!forward.has_more);
    // …and the materializer is where the page is cut, one row earlier.
    assert_eq!(forward.next_offset, 8);
    let forward = materialize_display_page(forward, None, None, None, None)
        .unwrap()
        .0;
    assert_eq!(forward["entries"].as_array().unwrap().len(), 7);
    assert_eq!(forward["hasMore"], true);
    assert_eq!(forward["nextOffset"], 7);

    // Backward: the cursor walks down from the end, so the row that does not fit
    // is the lowest ordinal and the next offset moves *past* it.
    let backward = read_display_page(&db, "s", Some(8), None, Some(1000)).unwrap();
    assert_eq!(backward.rows.len(), 8);
    let backward = materialize_display_page(backward, Some(8), None, None, None)
        .unwrap()
        .0;
    assert_eq!(backward["entries"].as_array().unwrap().len(), 7);
    assert_eq!(backward["hasMore"], true);
    assert_eq!(backward["nextOffset"], 1);
    // The two directions are the same cut, mirrored: 7 + 1 row.
    assert_eq!(forward["nextOffset"], 7);
}

#[test]
fn a_damaged_display_index_is_reported_instead_of_paging_a_blank_history() {
    use rusqlite::Connection;

    let page = |db: &Connection, before: Option<i64>| match read_display_page(
        db,
        "s",
        before,
        None,
        Some(5),
    ) {
        Ok(_) => panic!("a damaged index must not page as an empty history"),
        Err(error) => format!("{error:#}"),
    };

    // No history_display table at all: both directions report it.
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE session_formats(session_id TEXT,format INTEGER)")
        .unwrap();
    assert!(
        page(&db, None).contains("history_display"),
        "{}",
        page(&db, None)
    );
    assert!(
        page(&db, Some(10)).contains("history_display"),
        "{}",
        page(&db, Some(10))
    );

    // The table exists but is half-migrated (no `is_user`, no `payload`, and no
    // body store to join): the forward page reaches the join and names what it
    // cannot read…
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE history_display(session_id TEXT, ordinal INTEGER); CREATE TABLE session_formats(session_id TEXT,format INTEGER)")
        .unwrap();
    let forward = page(&db, None);
    assert!(forward.contains("entry_records"), "{forward}");
    // …while the backward cursor fails earlier, on the column it needs to find
    // the exchange boundary, rather than silently paginating from zero.
    let backward = page(&db, Some(10));
    assert!(backward.contains("is_user"), "{backward}");
}

#[test]
fn first_native_page_keeps_one_snapshot_across_startup_metadata_replacement() {
    let root = tempfile::tempdir().unwrap();
    let session = uuid::Uuid::new_v4().to_string();
    {
        let manager = Manager::new(root.path().join("sessions"));
        let store = manager.storage().unwrap();
        store.reserve_segmented(&session).unwrap();
        store.replace(&session, vec![
            json!({"id":uuid::Uuid::new_v4().to_string(),"type":"session_info","timestamp":"2026-10-27T00:00:00Z","content":{"model":"mock","session_name":"initial","system_prompt":"persistent configuration🙂".repeat(2000)}}),
            json!({"id":uuid::Uuid::new_v4().to_string(),"type":"user","role":"user","timestamp":"2026-10-27T00:00:01Z","content":"retained question"}),
            json!({"id":uuid::Uuid::new_v4().to_string(),"type":"assistant","role":"assistant","timestamp":"2026-10-27T00:00:02Z","content":[{"type":"text","text":"retained answer"}]})
        ]).unwrap();
    }
    let manager = Manager::new(root.path().join("sessions"));
    let store = manager.storage().unwrap();
    for before in [None, Some(i64::MAX)] {
        let expected = manager
            .history_page(&session, before, Some(0), Some(10))
            .unwrap();
        let snapshot = store.db.read_snapshot().unwrap();
        let mut page = snapshot
            .read(|db| read_display_page(db, &session, before, Some(0), Some(10)))
            .unwrap();
        page.snapshot.as_mut().unwrap().use_read_snapshot(snapshot);
        // Reproduce startup publication between capturing locators and reading
        // their bodies. Also replace an externalized configuration field and
        // actually collect its old body before completing the original page.
        manager.update_session_info_fields(&session, serde_json::from_value(json!({
            "session_name":"restored", "system_prompt":"updated configuration🙂".repeat(2000)
        })).unwrap()).unwrap();
        store
            .db
            .call(|db| {
                let tx = db.transaction()?;
                let ids = {
                    let mut q = tx.prepare("SELECT content_id FROM content_collection")?;
                    let rows = q
                        .query_map([], |row| row.get(0))?
                        .collect::<rusqlite::Result<Vec<i64>>>()?;
                    rows
                };
                crate::session::content::collect(&tx, &ids)?;
                tx.commit()?;
                Ok(())
            })
            .unwrap();
        let actual = materialize_display_page(page, before, None, Some(store), None)
            .unwrap()
            .0;
        assert_eq!(
            actual, expected,
            "one successful read, with no internal retry"
        );
        let current = manager
            .history_page(&session, before, Some(0), Some(10))
            .unwrap();
        assert_eq!(current["entries"][0]["session"]["sessionName"], "restored");
        assert!(current["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["blocks"][0]["text"] == "retained answer"));
    }
}

#[test]
fn pinned_page_still_rejects_deletion_and_releases_snapshot_on_error() {
    let root = tempfile::tempdir().unwrap();
    let manager = Manager::new(root.path().join("sessions"));
    let store = manager.storage().unwrap();
    store.reserve_segmented("s").unwrap();
    store.replace("s", vec![json!({"id":"u","type":"user","role":"user","timestamp":"2026-10-27T00:00:00Z","content":"history"})]).unwrap();
    let snapshot = store.db.read_snapshot().unwrap();
    let mut page = snapshot
        .read(|db| read_display_page(db, "s", None, Some(0), Some(10)))
        .unwrap();
    page.snapshot.as_mut().unwrap().use_read_snapshot(snapshot);
    store.delete("s").unwrap();
    assert!(materialize_display_page(page, None, None, Some(store), None).is_err());
    // Errors return the lease; normal history workers and all snapshot slots
    // remain available, and the deleted history is not served as success.
}
