use super::*;
use crate::session::{Manager, Session};

/// The index shape keeps pairing information only: legacy `tool_calls` and
/// modern call blocks are both reduced to id/name, and a tool entry's result
/// id is paired from its blocks.
#[test]
fn shape_reduces_legacy_and_block_tool_calls_to_pairing_ids() {
    let legacy = entry_shape(&json!({
        "id":"a","type":"assistant","role":"assistant",
        "timestamp":"2026-01-01T00:00:00Z",
        "meta":{"run_id":"r"},
        "tool_calls":[{"id":"call","type":"function",
            "function":{"name":"read","arguments":{"path":"/synthetic"}}}]
    }));
    assert_eq!(legacy["tool_calls"][0]["id"], "call");
    assert_eq!(legacy["tool_calls"][0]["function"]["name"], "read");
    assert_eq!(
        legacy["tool_calls"][0]["function"]["arguments"],
        json!({}),
        "arguments are not duplicated into the index"
    );
    assert_eq!(legacy["meta"]["run_id"], "r");

    let paired = entry_shape(&json!({
        "id":"t","type":"tool","role":"tool",
        "timestamp":"2026-01-01T00:00:00Z",
        "content":[
            {"type":"text","text":"prefix"},
            {"type":"tool_result","tool_call_id":"call","content":"result"}
        ]
    }));
    assert_eq!(paired["tool_call_id"], "call");
}

#[test]
fn paged_projection_matches_full_history_and_reconciles_appends() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().join("sessions"));
    let mut session = Session::new("/synthetic", "mock");
    session.entries.push(SessionEntry::session_info(
        json!({"session_name":"old","tokens_in":0}),
        "mock".into(),
        String::new(),
    ));
    let mut user = SessionEntry::new_user(
        "user",
        json!([{"type":"text","text":"question"},{"type":"text","text":"hidden attachment context"}]),
    );
    user.meta = Some(json!({"run_id":"r","attachments":[{"name":"synthetic"}]}));
    session.entries.push(user);
    let assistant = SessionEntry::new_assistant(
        json!([{"type":"reasoning","text":"synthetic thought"},{"type":"text","text":"answer"},{"type":"tool_call","id":"tc","name":"read","args":{"path":"/synthetic"}}]),
        Vec::new(),
    );
    session.entries.push(assistant);
    session.entries.push(SessionEntry::session_info(
        json!({"session_name":"new","tokens_in":50,"tokens_cache_r":5}),
        "mock".into(),
        String::new(),
    ));
    session
        .entries
        .push(SessionEntry::run_terminal("r", "completed", 12, 34, None));
    manager.save(&session).unwrap();
    let assert_pages = || {
        let expected = project_entries(&manager.load(&session.id).unwrap().entries);
        let mut actual = Vec::new();
        let mut offset = 0;
        loop {
            let page = manager
                .history_page(&session.id, None, Some(offset), Some(1))
                .unwrap();
            actual.extend(page["entries"].as_array().unwrap().iter().cloned());
            if page["hasMore"] == false {
                break;
            }
            offset = page["nextOffset"].as_i64().unwrap();
        }
        assert_eq!(actual, expected);
    };
    assert_pages();
    // Real result replaces the synthetic lost-tool slot when inserted
    // inside its tool-response window, without persisting a placeholder.
    session
        .entries
        .insert(3, SessionEntry::new_tool("tc", "real result"));
    manager.save(&session).unwrap();
    assert_pages();
    manager
        .append_entries(
            &session.id,
            &[SessionEntry::new_user("user", json!("next"))],
        )
        .unwrap();
    let page = manager
        .history_page(&session.id, Some(i64::MAX), None, Some(1))
        .unwrap();
    assert_eq!(page["entries"].as_array().unwrap().len(), 1);
    assert_eq!(page["entries"][0]["blocks"][0]["text"], "next");
    assert_pages();
}

#[test]
fn cold_page_reads_only_selected_bodies() {
    let dir = tempfile::tempdir().unwrap();
    let sessions = dir.path().join("sessions");
    let manager = Manager::new(sessions.clone());
    let mut session = Session::new("/synthetic", "mock");
    for index in 0..200 {
        let mut user = SessionEntry::new_user("user", json!(format!("question-{index}")));
        user.id = format!("u-{index}");
        session.entries.push(user);
        let mut reply = SessionEntry::new_assistant(json!("x".repeat(16384)), Vec::new());
        reply.id = format!("a-{index}");
        session.entries.push(reply);
    }
    manager.save(&session).unwrap();
    manager.initialize().unwrap();
    // Break an off-page body AFTER indexing. A full load would fail, but
    // a cold tail page must never decode or return that body's payload.
    manager.test_execute("PRAGMA ignore_check_constraints=ON; UPDATE entries SET content_json='invalid' WHERE position=1; PRAGMA ignore_check_constraints=OFF;");
    drop(manager);
    let manager = Manager::new(sessions);
    let page = manager
        .history_page(&session.id, Some(i64::MAX), None, Some(1))
        .unwrap();
    assert_eq!(page["entries"].as_array().unwrap().len(), 2);
    assert_eq!(page["entries"][0]["id"], "u-199");
    assert!(manager.load(&session.id).is_err());
    let id = session.id.clone();
    let count = manager
        .storage()
        .unwrap()
        .db
        .call(move |db| {
            Ok(crate::session::history::paging::read_display_page(
                db,
                &id,
                Some(i64::MAX),
                None,
                Some(1),
            )?
            .rows
            .len())
        })
        .unwrap();
    assert_eq!(count, 2);
    manager.delete(&session.id).unwrap();
    let count: i64 = manager
        .storage()
        .unwrap()
        .db
        .call(|db| Ok(db.query_row("SELECT count(*) FROM history_display", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn a_forward_page_stops_at_the_eight_megabyte_budget() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().join("sessions"));
    let mut session = Session::new("/synthetic", "mock");
    for index in 0..600 {
        let mut entry =
            SessionEntry::new_assistant(serde_json::json!("x".repeat(16 * 1024)), Vec::new());
        entry.id = format!("a-{index}");
        session.entries.push(entry);
    }
    manager.save(&session).unwrap();
    let page = manager.history_page(&session.id, None, None, None).unwrap();
    assert_eq!(page["hasMore"], true, "the page is cut short");
    assert!(
        page["entries"].as_array().unwrap().len() < 600,
        "only part of the 9.6 MiB history is materialized in one page"
    );
    assert!(page["nextOffset"].as_i64().unwrap() > 0);
}

/// The second, payload-level bound: a forward page stops *before* a row that
/// would push the materialized JSON past the budget, even though the row
/// itself was read. The row is deferred to the next page (`nextOffset`
/// points at it), so a single oversized message cannot make one response
/// unbounded.
#[test]
fn a_forward_page_is_cut_before_a_row_that_exceeds_the_payload_budget() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().join("sessions"));
    let mut session = Session::new("/synthetic", "mock");
    for index in 0..8 {
        let mut entry =
            SessionEntry::new_user("user", serde_json::json!(format!("question-{index}")));
        entry.id = format!("u-{index}");
        session.entries.push(entry);
    }
    // One message far larger than the whole page budget, past the first row.
    let mut huge = SessionEntry::new_user("user", serde_json::json!("H".repeat(9 * 1024 * 1024)));
    huge.id = "u-huge".into();
    session.entries.push(huge);
    manager.save(&session).unwrap();
    let id = session.id.clone();

    let page = manager.history_page(&id, None, None, Some(1000)).unwrap();
    assert_eq!(page["hasMore"], true, "the page is cut short");
    assert_eq!(
        page["nextOffset"], 8,
        "the cursor points at the oversized row, not past it"
    );
    let entries = page["entries"].as_array().unwrap();
    assert_eq!(
        entries.len(),
        8,
        "only the rows that fit the budget are materialized"
    );
    assert_eq!(entries[0]["id"], "u-0");
    assert!(
        !page.to_string().contains("HHHH"),
        "the oversized row is deferred, not serialized into this page"
    );

    // And it is reachable on the next page, so nothing is lost.
    let next = manager
        .history_page(
            &id,
            None,
            Some(page["nextOffset"].as_i64().unwrap()),
            Some(1000),
        )
        .unwrap();
    assert_eq!(next["fragment"]["entryId"], "u-huge");
    assert!(next.to_string().len() < crate::session::history::paging::HISTORY_PAGE_BYTE_BUDGET);
}

/// The bytes a forward page materializes are bounded by the bytes it read:
/// the projection of a row is never larger than the row's stored body plus
/// its index overlay. This is what keeps the payload bound above from firing
/// on ordinary pages — if a projection ever started duplicating its input,
/// this assertion would fail.
#[test]
fn the_projected_page_is_never_larger_than_the_bytes_that_were_read() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().join("sessions"));
    let mut session = Session::new("/synthetic", "mock");
    session.entries.push(SessionEntry::session_info(
        serde_json::json!({"session_name":"synthetic","tokens_in":10}),
        "mock".into(),
        String::new(),
    ));
    let mut cjk = SessionEntry::new_user("user", serde_json::json!("中文问题🙂 with ASCII"));
    cjk.id = "u-cjk".into();
    session.entries.push(cjk);
    let mut long = SessionEntry::new_assistant(
        serde_json::json!(format!("x {}", "y".repeat(64 * 1024))),
        Vec::new(),
    );
    long.id = "a-long".into();
    session.entries.push(long);
    let mut tool = SessionEntry::new_tool("call-1", "tool output");
    tool.id = "t-1".into();
    tool.meta = Some(serde_json::json!({"run_id":"r","attachments":[{"name":"a.txt"}]}));
    session.entries.push(tool);
    session
        .entries
        .push(SessionEntry::run_started_with_sequence("r", 1, Some(1)));
    session
        .entries
        .push(SessionEntry::run_terminal("r", "completed", 5, 7, None));
    session.entries.push(SessionEntry::new_assistant(
        serde_json::json!([]),
        Vec::new(),
    ));
    manager.save(&session).unwrap();
    let id = session.id.clone();

    let page = manager.history_page(&id, None, None, Some(1000)).unwrap();
    let entries = page["entries"].as_array().unwrap();
    let (rows, stored) = manager
        .storage()
        .unwrap()
        .db
        .call(move |db| {
            let rows: i64 = db
                .query_row(
                    "SELECT count(*) FROM history_display WHERE session_id=?1",
                    [&id],
                    |r| r.get(0),
                )
                .unwrap();
            let stored: i64 = db
                .query_row(
                    "SELECT coalesce(sum(length(d.payload) + coalesce(length(e.payload),0)),0)
                 FROM history_display d LEFT JOIN entry_records e
                   ON e.session_id=d.session_id AND e.position=d.source_position
                 WHERE d.session_id=?1",
                    [&id],
                    |r| r.get(0),
                )
                .unwrap();
            Ok((rows, stored))
        })
        .unwrap();
    let projected: usize = entries
        .iter()
        .map(|entry| serde_json::to_vec(entry).unwrap().len())
        .sum();
    assert_eq!(
        entries.len() as i64,
        rows,
        "the whole (small) history fits in one page, so the two sums cover the same rows"
    );
    assert!(
        projected as i64 <= stored,
        "the projection inflated a page: {projected} projected > {stored} read"
    );
}
