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

// ─── schema damage is reported, never swallowed ────────────────────────

const ENTRIES_DDL: &str = "CREATE TABLE entries(\
    session_id TEXT, position INTEGER, run_id TEXT, entry_type TEXT)";
const BLOCKS_DDL: &str = "CREATE TABLE message_blocks(\
    session_id TEXT, entry_position INTEGER, tool_call_id TEXT, kind TEXT)";
const SHAPES_DDL: &str = "CREATE TABLE history_shapes(\
    session_id TEXT, position INTEGER, payload TEXT)";

fn schema(statements: &[&str]) -> Connection {
    let db = Connection::open_in_memory().unwrap();
    for statement in statements {
        db.execute_batch(statement).unwrap();
    }
    db
}

fn shape_row(value: Value) -> String {
    let entry: SessionEntry = serde_json::from_value(value).expect("valid entry");
    serde_json::to_string(&entry).unwrap()
}

/// Every writer runs inside the session database. When the schema underneath it
/// is incomplete (a partially migrated or damaged file), the write must fail
/// loudly — a silently skipped projection would serve a page that no longer
/// matches the transcript.
#[test]
fn an_incomplete_schema_surfaces_instead_of_swallowing_the_write() {
    let value = json!({"id":"u1","type":"user","role":"user","timestamp":"2026-01-01T00:00:00Z",
        "content":[{"type":"text","text":"question"}]});
    let message = |error: anyhow::Error| format!("{error:#}");

    // Nothing at all: the shape insert and the metadata refresh both report the
    // missing tables instead of returning Ok.
    let empty = schema(&[]);
    assert!(
        message(insert_entry_shape(&empty, "s1", 0, &value).unwrap_err())
            .contains("history_shapes")
    );
    assert!(message(refresh_session_metadata(&empty, "s1").unwrap_err()).contains("history_shapes"));
    assert!(message(affected_history_start(&empty, "s1", 0).unwrap_err()).contains("entries"));

    // Only `entries` exists: the tool-dependency expansion cannot be answered,
    // so the caller must not treat the tail as already projected.
    let entries_only = schema(&[ENTRIES_DDL]);
    assert!(
        message(affected_history_start(&entries_only, "s1", 0).unwrap_err())
            .contains("message_blocks")
    );

    // `entries` without the requested column (a partially applied migration).
    let stale_entries = schema(&[
        "CREATE TABLE entries(session_id TEXT, position INTEGER, run_id TEXT)",
        BLOCKS_DDL,
    ]);
    let error = message(affected_history_start(&stale_entries, "s1", 0).unwrap_err());
    assert!(error.contains("entry_type"), "{error}");

    // A complete transcript side but no display table: the ordinal probe fails.
    let no_display = schema(&[ENTRIES_DDL, BLOCKS_DDL]);
    let error = message(update_display_index_from(&no_display, "s1", 0).unwrap_err());
    assert!(error.contains("history_shapes"), "{error}");

    // Shapes present but empty: the reconcile reaches the delete and confirms
    // the display table it is missing.
    let shapes_only = schema(&[ENTRIES_DDL, BLOCKS_DDL, SHAPES_DDL]);
    let error = message(update_display_index_from(&shapes_only, "s1", 0).unwrap_err());
    assert!(error.contains("history_display"), "{error}");

    // A stored shape to project: the upsert is what reports the missing table.
    let shape = shape_row(value.clone());
    let with_shape = schema(&[
        ENTRIES_DDL,
        BLOCKS_DDL,
        SHAPES_DDL,
        &format!("INSERT INTO history_shapes VALUES ('s1',0,'{shape}')"),
    ]);
    let error = message(update_display_index_from(&with_shape, "s1", 0).unwrap_err());
    assert!(error.contains("history_display"), "{error}");

    // The metadata refresh joins the record entry and then writes the display
    // row: with no display table the update is the failing statement.
    let info = shape_row(json!({"id":"info","type":"session_info","role":"system",
        "timestamp":"2026-01-01T00:00:00Z","content":{"session_name":"n"}}));
    let metadata = schema(&[
        ENTRIES_DDL,
        SHAPES_DDL,
        "INSERT INTO entries VALUES ('s1',0,'r','session_info')",
        &format!("INSERT INTO history_shapes VALUES ('s1',0,'{info}')"),
    ]);
    let error = message(refresh_session_metadata(&metadata, "s1").unwrap_err());
    assert!(error.contains("history_display"), "{error}");

    // A damaged index with a *non-zero* dirty start: the ordinal probe only runs
    // when the tail begins after position 0, and it must report the missing
    // table rather than silently restarting the index at zero.
    let probe = schema(&[
        ENTRIES_DDL,
        BLOCKS_DDL,
        SHAPES_DDL,
        "INSERT INTO entries VALUES ('s1',5,'r1','user')",
    ]);
    let error = message(update_display_index_from(&probe, "s1", 5).unwrap_err());
    assert!(error.contains("history_display"), "{error}");

    // The display table exists but refuses writes: both the upsert and the
    // trailing delete must surface the damage instead of leaving a half-written
    // index behind.
    const DISPLAY_DDL: &str = "CREATE TABLE history_display(session_id TEXT, ordinal INTEGER, \
        source_position INTEGER, is_user INTEGER, payload TEXT, PRIMARY KEY(session_id,ordinal))";
    for (timing, when, seed_stale_row) in [
        ("BEFORE INSERT", "inserts", false),
        ("BEFORE DELETE", "deletes", true),
    ] {
        let trigger = format!(
            "CREATE TRIGGER refuse_{when} {timing} ON history_display \
             BEGIN SELECT RAISE(ABORT, 'damaged display index'); END;"
        );
        // The trailing delete only touches rows past the new projection, so the
        // delete case needs one stale row for its trigger to have anything to
        // refuse.
        let stale_row = "INSERT INTO history_display VALUES ('s1',1,0,0,'stale')";
        let mut statements: Vec<&str> =
            vec![ENTRIES_DDL, BLOCKS_DDL, SHAPES_DDL, DISPLAY_DDL, &trigger];
        if seed_stale_row {
            statements.push(stale_row);
        }
        let shape_insert = format!("INSERT INTO history_shapes VALUES ('s1',0,'{shape}')");
        statements.push(&shape_insert);
        let db = schema(&statements);
        let error = message(update_display_index_from(&db, "s1", 0).unwrap_err());
        assert!(error.contains("damaged display index"), "{when}: {error}");
    }
}

/// The projected display row for a session's compaction entry, read straight
/// from the index. A *page* is the wrong witness: with a body present it
/// re-projects from the entry, so the stored row's own payload is what has to
/// carry the checkpoint.
fn compaction_display_row(manager: &Manager, id: &str) -> serde_json::Value {
    let id = id.to_owned();
    let payload: String = manager
        .storage()
        .unwrap()
        .db
        .call(move |db| {
            Ok(db.query_row(
                "SELECT payload FROM history_display WHERE session_id=?1 AND json_extract(payload,'$.kind')='compaction'",
                rusqlite::params![id],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    serde_json::from_str(&payload).unwrap()
}

fn session_with_checkpoint() -> (Manager, Session) {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().join("sessions"));
    let mut session = Session::new("/synthetic", "mock");
    session
        .entries
        .push(SessionEntry::new_user("user", json!("question")));
    session.entries.push(SessionEntry {
        entry_type: "compaction".into(),
        ..SessionEntry::new_user(
            "",
            json!({
                "schema_version": 3, "checkpoint_id": "cp_1", "phase": "standalone",
                "tokens_before": 62203, "tokens_after": 33281, "trigger": "manual",
            }),
        )
    });
    session
        .entries
        .push(SessionEntry::new_assistant(json!("answer"), Vec::new()));
    manager.save(&session).unwrap();
    (manager, session)
}

/// The divider a client draws is built from `payload.checkpoint`: its identity
/// (`checkpointId`) folds the live copy against the durable one, and its counts
/// and `phase` decide the text and whether the marker stays inside the exchange.
/// A compaction shape must keep that object — it is the projection's input, not
/// message body text.
#[test]
fn a_compaction_shape_keeps_the_checkpoint_the_divider_prints() {
    let checkpoint = json!({
        "schema_version": 3,
        "checkpoint_id": "cp_20261019-170529",
        "cutoff_entry_id": "e9",
        "tokens_before": 62203,
        "tokens_after": 33281,
        "trigger": "manual",
        "phase": "standalone",
        "summary": [{"type": "text", "text": "covered history index"}],
    });
    let shape = entry_shape(&json!({
        "id": "c1", "type": "compaction", "role": "system",
        "timestamp": "2026-10-19T09:05:29Z",
        "content": checkpoint.clone(),
    }));
    assert_eq!(
        shape["content"], checkpoint,
        "the checkpoint is projected verbatim, summary included: a partial copy \
         would make an indexed page disagree with the direct projection"
    );
    let entry: SessionEntry = serde_json::from_value(shape).unwrap();
    let projected = project_entries(&[entry]).remove(0);
    assert_eq!(projected["kind"], "compaction");
    assert_eq!(
        projected["checkpoint"]["checkpointId"],
        "cp_20261019-170529"
    );
    assert_eq!(projected["checkpoint"]["tokensBefore"], 62203);
    assert_eq!(projected["checkpoint"]["tokensAfter"], 33281);
    assert_eq!(projected["checkpoint"]["trigger"], "manual");
    assert_eq!(projected["checkpoint"]["phase"], "standalone");
}

/// The same thing end to end: a saved compaction entry's indexed row carries the
/// counts, which is what the body-less fallback serves to a client.
#[test]
fn the_display_index_carries_the_checkpoint_counts_and_identity() {
    let (manager, session) = session_with_checkpoint();
    let payload = compaction_display_row(&manager, &session.id);
    assert_eq!(payload["checkpoint"]["checkpointId"], "cp_1");
    assert_eq!(payload["checkpoint"]["tokensBefore"], 62203);
    assert_eq!(payload["checkpoint"]["tokensAfter"], 33281);
    assert_eq!(payload["checkpoint"]["trigger"], "manual");
    assert_eq!(payload["checkpoint"]["phase"], "standalone");
}

/// A row the version-1 projection wrote — checkpoint blanked to `""` — is
/// rebuilt on the next read instead of being trusted forever: the version stamp
/// is what says whether a session's derived rows are current.
#[test]
fn a_version_one_row_is_rebuilt_on_its_next_read() {
    let (manager, session) = session_with_checkpoint();
    let id = session.id;
    // Exactly what the previous projection produced, plus its version stamp.
    manager.test_execute(&format!(
        "UPDATE history_display SET payload=json_set(payload,'$.checkpoint','')
           WHERE session_id='{id}' AND json_extract(payload,'$.kind')='compaction';
         INSERT OR REPLACE INTO history_index_state(session_id,projection_version)
           VALUES ('{id}',1);"
    ));
    assert_eq!(
        compaction_display_row(&manager, &id)["checkpoint"],
        json!(""),
        "the fixture must reproduce the version-1 row"
    );
    // Any read of the session rebuilds it, because the stamp is out of date.
    manager.history_page(&id, None, None, Some(100)).unwrap();
    let rebuilt = compaction_display_row(&manager, &id);
    assert_eq!(rebuilt["checkpoint"]["checkpointId"], "cp_1");
    assert_eq!(rebuilt["checkpoint"]["tokensBefore"], 62203);
    assert_eq!(rebuilt["checkpoint"]["tokensAfter"], 33281);
    // The rebuild must not have moved or dropped anything else.
    assert_eq!(rebuilt["kind"], "compaction");
}

/// A row the old projection already wrote is still a marker when it is read as
/// data: the client draws a divider without counts. Losing the marker entirely
/// would be the worse failure — the transcript would claim the conversation
/// continued unbroken.
#[test]
fn a_blank_checkpoint_still_projects_a_compaction_marker() {
    let entry = SessionEntry {
        entry_type: "compaction".into(),
        ..SessionEntry::new_user("", json!(""))
    };
    let projected = project_entries(&[entry]).remove(0);
    assert_eq!(projected["kind"], "compaction");
    assert_eq!(projected["checkpoint"], json!(""));
}
