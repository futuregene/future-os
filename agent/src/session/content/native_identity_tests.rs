//! Actual UUID keys, rather than short string fixture aliases.
use super::*;
use crate::{
    session::sqlite_store::SqliteStore,
    types::{AgentMessage, ContentBlock},
};
use serde_json::{json, Value};

#[test]
fn native_message_keys_roundtrip_bodies_and_terminal_identity() {
    let root = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&root.path().join("agent.db")).unwrap();
    let session = uuid::Uuid::new_v4().to_string();
    let run = uuid::Uuid::new_v4().to_string();
    let info = uuid::Uuid::new_v4().to_string();
    let user = uuid::Uuid::new_v4().to_string();
    let start = uuid::Uuid::new_v4().to_string();
    let message = uuid::Uuid::new_v4().to_string();
    store.reserve_segmented(&session).unwrap();
    store.replace(&session,vec![json!({"id":info,"type":"session_info","timestamp":"2026-10-27T00:00:00Z","content":{"cwd":".","model":"synthetic"}}),json!({"id":user,"type":"user","role":"user","timestamp":"2026-10-27T00:00:00Z","content":"native user body"}),json!({"id":start,"type":"run_started","timestamp":"2026-10-27T00:00:00Z","content":{"run_id":run,"epoch":1}})]).unwrap();
    let key = format!("{message}:text:b");
    let event = |cursor, kind: &str, data: Value| json!({"session_id":session,"run_id":run,"epoch":1,"idx":cursor,"session_idx":-1,"event_type":kind,"timestamp":"2026-10-27T00:00:01Z","data":data.to_string()});
    journal::Writer::default().persist(&store,&session,&[
        event(0,"message_start",json!({"entry_id":message})),
        event(1,"text_start",json!({"entry_id":message,"content_key":key,"content_offset":0,"content_bytes":0})),
        event(2,"text_chunk",json!({"entry_id":message,"content_key":key,"content_offset":0,"content_bytes":6,"text":"native"})),
        event(3,"text_end",json!({"entry_id":message,"content_key":key,"content_offset":6,"content_bytes":0,"content_seal":true})),
        event(4,"message_end",json!({"entry_id":message})),
    ]).unwrap();
    let mut assistant = AgentMessage {
        role: "assistant".into(),
        content: vec![ContentBlock::text("native")],
        name: String::new(),
        tool_args: String::new(),
        metadata: None,
    };
    assistant.metadata = Some(serde_json::Map::from_iter([
        (AgentMessage::JOURNAL_ENTRY_ID_KEY.into(), json!(message)),
        (AgentMessage::STREAM_BLOCKS_KEY.into(), json!([key])),
    ]));
    let recipe = stream_entry::StreamEntry::from_assistant(&assistant, &run, 1).unwrap();
    store.append_stream_entry(&session, recipe).unwrap();
    let end = uuid::Uuid::new_v4().to_string();
    let terminal = json!({"id":end,"type":"run_terminal","timestamp":"2026-10-27T00:00:02Z","content":{"run_id":run,"state":"completed","run_tokens":0}});
    store
        .commit_terminal_entries(&session, vec![terminal.clone()])
        .unwrap();
    store
        .commit_terminal_entries(&session, vec![terminal])
        .unwrap();
    let entries = store.entries(&session).unwrap();
    assert_eq!(store.run_replay_watermark(&session, &run).unwrap(), Some(5));
    assert!(
        store
            .segmented_replay(&session, &run, -1, false, Some(32))
            .unwrap()
            .known
    );
    let identity = store.storage_identity().unwrap();
    let request = future_rpc::content_range::ContentRequest {
        database_id: identity.database_id,
        restore_generation: identity.restore_generation,
        epoch: 1,
        cursor: 2,
        field: "/data/text".into(),
        offset: 0,
        max_bytes: 1024,
    };
    let body = store
        .event_content_range(&session, &run, request.clone())
        .unwrap();
    assert_eq!(body["text"], "native");
    assert!(store
        .event_content_range(&uuid::Uuid::new_v4().to_string(), &run, request)
        .is_err());
    assert_eq!(
        entries.iter().find(|e| e["id"] == user).unwrap()["content"][0]["text"],
        "native user body"
    );
    assert_eq!(
        entries.iter().find(|e| e["id"] == message).unwrap()["content"][0]["text"],
        "native"
    );
    store.db.read(|db| {
        for (table,column) in [("sessions","id"),("entries","session_id"),("runs","session_id"),("stream_runs","session_id"),("message_blocks","session_id"),("content_references","session_id"),("session_formats","session_id"),("content_history_versions","session_id"),("entry_fingerprints","session_id"),("history_shapes","session_id"),("history_display","session_id"),("history_index_state","session_id")] {
            assert_eq!(db.query_row(&format!("SELECT count(*) FROM {table} WHERE typeof({column})!='blob' OR length({column})!=16"),[],|r|r.get::<_,i64>(0))?,0,"{table}.{column}");
        }
        let violations:i64=db.query_row("SELECT count(*) FROM pragma_foreign_key_check",[],|r|r.get(0))?;assert_eq!(violations,0);
        let invalid:i64=db.query_row("SELECT count(*) FROM entries WHERE typeof(entry_id)!='blob' OR length(entry_id)!=16",[],|r|r.get(0))?;assert_eq!(invalid,0);
        let invalid:i64=db.query_row("SELECT count(*) FROM entry_fingerprints WHERE typeof(entry_id)!='blob' OR length(entry_id)!=16",[],|r|r.get(0))?;assert_eq!(invalid,0);
        let invalid:i64=db.query_row("SELECT count(*) FROM content_references WHERE owner_kind='entry' AND (typeof(owner_id)!='blob' OR length(owner_id)!=16)",[],|r|r.get(0))?;assert_eq!(invalid,0);
        let invalid:i64=db.query_row("SELECT count(*) FROM entries WHERE run_id IS NOT NULL AND (typeof(run_id)!='blob' OR length(run_id)!=16)",[],|r|r.get(0))?;assert_eq!(invalid,0);
        let invalid:i64=db.query_row("SELECT count(*) FROM runs WHERE typeof(run_id)!='blob' OR length(run_id)!=16",[],|r|r.get(0))?;assert_eq!(invalid,0);
        let invalid:i64=db.query_row("SELECT count(*) FROM stream_runs WHERE typeof(run_id)!='blob' OR length(run_id)!=16",[],|r|r.get(0))?;assert_eq!(invalid,0);
        Ok(())
    }).unwrap();
}

#[test]
fn native_session_public_reads_and_deleted_identity_survive_reopen() {
    use crate::session::Manager;
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("sessions");
    let manager = Manager::new(dir.clone());
    let store = manager.storage().unwrap();
    let session = uuid::Uuid::new_v4().to_string();
    let info = uuid::Uuid::new_v4().to_string();
    let user = uuid::Uuid::new_v4().to_string();
    // Exercise the production admission boundary, rather than test reservation.
    store.bind_events(&session).unwrap();
    assert!(!store.contains(&session).unwrap());
    let metadata = json!({"id":info,"type":"session_info","timestamp":"2026-10-27T00:00:00Z","content":{"cwd":".","model":"mock","session_name":"native sidebar"}});
    store.replace(&session,vec![metadata.clone(),json!({"id":user,"type":"user","role":"user","timestamp":"2026-10-27T00:00:01Z","content":"native visible input"})]).unwrap();
    store.bind_events(&session).unwrap();
    assert!(store.contains(&session).unwrap());
    assert!(store.revision(&session).unwrap().is_some());
    assert!(manager.list_ids().unwrap().contains(&session));
    let summaries = manager.list_all().unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].id, session);
    assert_eq!(manager.state_metadata(&session).unwrap().0, 1);
    assert_eq!(manager.parent_session_id(&session).unwrap(), None);
    assert_eq!(manager.load_metadata(&session).unwrap().id, session);
    manager
        .update_session_info(&session, "session_name", json!("renamed native sidebar"))
        .unwrap();
    assert_eq!(
        manager.load_metadata(&session).unwrap().name,
        "renamed native sidebar"
    );
    assert!(!manager
        .history_page(&session, None, None, Some(10))
        .unwrap()["entries"]
        .as_array()
        .unwrap()
        .is_empty());
    let search = manager.search_history_all("visible input", 10, 10).unwrap();
    assert_eq!(search["matches"][0]["sessionId"], session);
    let entries = store.entries(&session).unwrap();
    let run = entries.iter().find(|e| e["id"] == user).unwrap()["meta"]["run_id"]
        .as_str()
        .unwrap();
    journal::Writer::default().persist(store,&session,&[json!({"session_id":session,"run_id":run,"epoch":1,"idx":0,"session_idx":-1,"event_type":"usage","timestamp":"2026-10-27T00:00:02Z","data":"{\"input_tokens\":3}"})]).unwrap();
    assert_eq!(store.pricing_event_payloads(&session).unwrap().len(), 1);
    assert_eq!(store.event_cursors(&session).unwrap(), (0, 0));
    let expected = session.clone();
    store
        .db
        .read(move |db| {
            let actual: (String, String, i64) = db.query_row(
                "SELECT id,id_storage_type,id_bytes FROM sessions_inspect",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            assert_eq!(actual, (expected, "blob".into(), 16));
            Ok(())
        })
        .unwrap();
    assert_eq!(
        manager
            .session_presence(std::slice::from_ref(&session))
            .unwrap()["sessions"][0]["status"],
        "present"
    );
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join(format!("{session}.jsonl"));
    let bytes = format!(
        "{}\n",
        json!({"id":uuid::Uuid::new_v4().to_string(),"type":"user","role":"user","timestamp":"2020-01-01T00:00:00Z","content":"obsolete restored source"})
    );
    std::fs::write(&source, &bytes).unwrap();
    store.import_legacy(&dir, None).unwrap();
    assert_eq!(std::fs::read_to_string(&source).unwrap(), bytes);
    assert_eq!(store.import_records().unwrap().len(), 0);
    assert!(store
        .entries(&session)
        .unwrap()
        .iter()
        .any(|e| e["id"] == user));
    assert_eq!(
        manager
            .session_presence(std::slice::from_ref(&session))
            .unwrap()["sessions"][0]["status"],
        "present"
    );
    manager.delete(&session).unwrap();
    assert!(!store.contains(&session).unwrap());
    assert!(!manager.list_ids().unwrap().contains(&session));
    assert!(store.bind_events(&session).is_err());
    assert!(store.reserve_segmented(&session).is_err());
    assert!(store.replace(&session, vec![metadata]).is_err());
    store.import_legacy(&dir, None).unwrap();
    assert_eq!(
        manager
            .session_presence(std::slice::from_ref(&session))
            .unwrap()["sessions"][0]["status"],
        "deleted"
    );
    store
        .db
        .read(|db| {
            let key: (String, i64) = db.query_row(
                "SELECT typeof(session_id),length(session_id) FROM session_tombstones",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            assert_eq!(key, ("blob".into(), 16));
            Ok(())
        })
        .unwrap();
    drop(manager);
    let manager = Manager::new(dir);
    assert!(!manager.contains(&session).unwrap());
    assert!(manager.storage().unwrap().bind_events(&session).is_err());
}

#[test]
fn implicit_native_run_is_short_and_reused_by_retries_and_following_entries() {
    let root = tempfile::tempdir().unwrap();
    let manager = crate::session::Manager::new(root.path().join("sessions"));
    let store = manager.storage().unwrap();
    let session = uuid::Uuid::new_v4().to_string();
    let info = uuid::Uuid::new_v4().to_string();
    let user = uuid::Uuid::new_v4().to_string();
    let answer = uuid::Uuid::new_v4().to_string();
    store.reserve_segmented(&session).unwrap();
    store.replace(&session,vec![json!({"id":info,"type":"session_info","timestamp":"2026-10-27T00:00:00Z","content":{"model":"mock"}})]).unwrap();
    let input = json!({"id":user,"type":"user","role":"user","timestamp":"2026-10-27T00:00:01Z","content":"question"});
    store.append(&session, vec![input.clone()]).unwrap();
    let first = store.entries(&session).unwrap();
    let run = first.iter().find(|e| e["id"] == user).unwrap()["meta"]["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(uuid::Uuid::parse_str(&run).unwrap().to_string(), run);
    store.append(&session, vec![input]).unwrap();
    store.append(&session,vec![json!({"id":answer,"type":"assistant","role":"assistant","timestamp":"2026-10-27T00:00:02Z","content":"answer"})]).unwrap();
    let call = "provider_opaque:call/0001";
    let tool_message = uuid::Uuid::new_v4().to_string();
    let tool_result = uuid::Uuid::new_v4().to_string();
    store.append(&session,vec![
        json!({"id":tool_message,"type":"assistant","role":"assistant","timestamp":"2026-10-27T00:00:03Z","content":[{"type":"tool_call","id":call,"name":"read","args":{"path":"native.txt"}}]}),
        json!({"id":tool_result,"type":"tool","role":"tool","timestamp":"2026-10-27T00:00:04Z","content":[{"type":"tool_result","tool_call_id":call,"content":"native tool result","is_error":false}]}),
    ]).unwrap();
    assert_eq!(
        manager.tool_call_args(&session, &run, call).unwrap()["arguments"],
        json!({"path":"native.txt"})
    );
    assert_eq!(
        manager.tool_output(&session, &run, call).unwrap()["output"]["text"],
        "native tool result"
    );
    let tools = manager.tool_page(&session, &run, 0, 10).unwrap();
    assert_eq!(tools["tools"][0]["runId"], run);
    assert_eq!(tools["tools"][0]["toolCallId"], call);
    let hits = reader::Snapshot::search(store, &session, "answer", 10).unwrap();
    assert!(hits
        .iter()
        .any(|hit| hit["entryId"] == answer && hit["runId"] == run));
    let recalled = reader::Snapshot::recall(store, &session, &answer, 0, 1024).unwrap();
    assert_eq!(recalled["entryId"], answer);
    assert_eq!(recalled["runId"], run);
    let entries = store.entries(&session).unwrap();
    assert_eq!(entries.iter().filter(|e| e["id"] == user).count(), 1);
    for entry in entries.iter().filter(|e| e["type"] != "session_info") {
        assert_eq!(entry["meta"]["run_id"], run);
    }
    store
        .db
        .read(|db| {
            assert_eq!(
                db.query_row("SELECT count(*) FROM runs", [], |r| r.get::<_, i64>(0))?,
                1
            );
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM runs WHERE typeof(run_id)='blob' AND length(run_id)=16",
                    [],
                    |r| r.get::<_, i64>(0)
                )?,
                1
            );
            Ok(())
        })
        .unwrap();
}

#[test]
fn uuid_looking_legacy_keys_keep_their_text_storage_when_continued() {
    let root = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&root.path().join("agent.db")).unwrap();
    let session = uuid::Uuid::new_v4().to_string();
    let run = uuid::Uuid::new_v4().to_string();
    let info = uuid::Uuid::new_v4().to_string();
    let user = uuid::Uuid::new_v4().to_string();
    let start = uuid::Uuid::new_v4().to_string();
    // The compatibility writer creates format 1, as a released importer does.
    store.replace(&session,vec![
        json!({"id":info,"type":"session_info","timestamp":"2026-10-27T00:00:00Z","content":{"model":"mock"}}),
        json!({"id":user,"type":"user","role":"user","timestamp":"2026-10-27T00:00:01Z","meta":{"run_id":run},"content":"historical input"}),
        json!({"id":start,"type":"run_started","timestamp":"2026-10-27T00:00:02Z","content":{"run_id":run,"epoch":1}}),
    ]).unwrap();
    assert_eq!(store.storage_format(&session).unwrap(), 1);
    let answer = uuid::Uuid::new_v4().to_string();
    store.append(&session,vec![json!({"id":answer,"type":"assistant","role":"assistant","timestamp":"2026-10-27T00:00:03Z","meta":{"run_id":run},"content":"continued history"})]).unwrap();
    let entries = store.entries(&session).unwrap();
    assert!(entries.iter().any(|e| e["id"] == user));
    assert_eq!(
        entries.iter().find(|e| e["id"] == answer).unwrap()["meta"]["run_id"],
        run
    );
    store.db.read(|db| {
        assert_eq!(db.query_row("SELECT count(*) FROM sessions WHERE typeof(id)!='text'",[],|r|r.get::<_,i64>(0))?,0);
        assert_eq!(db.query_row("SELECT count(*) FROM entries WHERE typeof(entry_id)!='text' OR (run_id IS NOT NULL AND typeof(run_id)!='text')",[],|r|r.get::<_,i64>(0))?,0);
        assert_eq!(db.query_row("SELECT count(*) FROM runs WHERE typeof(run_id)!='text'",[],|r|r.get::<_,i64>(0))?,0);
        Ok(())
    }).unwrap();
}

#[test]
fn native_message_fork_and_reset_keep_shared_body_ranges_across_reopen() {
    use crate::session::{
        reset::{DropKind, ResetSelectors},
        ForkPoint, ForkRequest, Manager,
    };
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("sessions");
    let manager = Manager::new(dir.clone());
    let store = manager.storage().unwrap();
    let session = uuid::Uuid::new_v4().to_string();
    let user = uuid::Uuid::new_v4().to_string();
    let answer = uuid::Uuid::new_v4().to_string();
    let info = uuid::Uuid::new_v4().to_string();
    let text = "retained answer🙂".repeat(4000);
    store.reserve_segmented(&session).unwrap();
    store.replace(&session,vec![
        json!({"id":info,"type":"session_info","timestamp":"2026-10-27T00:00:00Z","content":{"model":"mock","cwd":"."}}),
        json!({"id":user,"type":"user","role":"user","timestamp":"2026-10-27T00:00:01Z","content":"question"}),
        json!({"id":answer,"type":"assistant","role":"assistant","timestamp":"2026-10-27T00:00:02Z","content":[{"type":"reasoning","text":"private thought".repeat(4000)},{"type":"text","text":text}]}),
    ]).unwrap();
    let range = |store: &SqliteStore, session: &str, entry: &str, field: &str| {
        let (s, e, f) = (session.to_owned(), entry.to_owned(), field.to_owned());
        store
            .db
            .read(move |db| resolve(db, &s, "entry", &e, &f))
            .unwrap()
            .unwrap()
    };
    let original = range(store, &session, &answer, "/content/1/text");
    let request = ForkRequest {
        request_id: uuid::Uuid::new_v4().to_string(),
        parent_session_id: session.clone(),
        point: ForkPoint::ThroughEntry(answer.clone()),
        created_by: "test".into(),
        creator_id: "device".into(),
    };
    let fork = manager.create_fork_identity(request.clone()).unwrap();
    assert_eq!(
        manager.create_fork_identity(request).unwrap().session_id,
        fork.session_id
    );
    let child = store.entries(&fork.session_id).unwrap();
    let child_answer = child
        .iter()
        .find(|e| e["meta"]["fork_origin"]["entry_id"] == answer)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        range(store, &fork.session_id, &child_answer, "/content/1/text"),
        original
    );
    let reset = manager
        .reset_history(
            &fork.session_id,
            &ResetSelectors {
                drop: vec![DropKind::Thinking],
                ..Default::default()
            },
            true,
        )
        .unwrap();
    assert!(reset.applied);
    assert_eq!(
        range(store, &fork.session_id, &child_answer, "/content/0/text"),
        original
    );
    assert_eq!(
        store
            .entries(&fork.session_id)
            .unwrap()
            .iter()
            .find(|e| e["id"] == child_answer)
            .unwrap()["content"][0]["text"],
        text
    );
    manager.delete(&session).unwrap();
    drop(manager);
    let manager = Manager::new(dir);
    let store = manager.storage().unwrap();
    assert_eq!(
        range(store, &fork.session_id, &child_answer, "/content/0/text"),
        original
    );
    assert_eq!(
        store
            .entries(&fork.session_id)
            .unwrap()
            .iter()
            .find(|e| e["id"] == child_answer)
            .unwrap()["content"][0]["text"],
        text
    );
}

#[test]
fn native_unpublished_message_cleanup_preserves_published_and_queued_owners() {
    use crate::session::{Manager, SessionEntry};
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("sessions");
    let manager = Manager::new(dir.clone());
    let store = manager.storage().unwrap();
    let session = uuid::Uuid::new_v4().to_string();
    let info = uuid::Uuid::new_v4().to_string();
    let run = uuid::Uuid::new_v4().to_string();
    store.reserve_segmented(&session).unwrap();
    store.replace(&session,vec![json!({"id":info,"type":"session_info","timestamp":"2026-10-27T00:00:00Z","content":{"model":"mock"}})]).unwrap();
    let input = |id: &str| json!({"id":id,"type":"user","role":"user","timestamp":"2026-10-27T00:00:01Z","meta":{"run_id":run},"content":"durable input🙂".repeat(5000)});
    let owners = |store: &SqliteStore, id: &str| {
        let (s, e) = (session.clone(), id.to_owned());
        store.db.read(move|db| {
            let owner=super::owner_key(db,&s,"entry",&e)?;
            let s=crate::session::identity::session_key(db,&s)?;
            Ok(db.query_row("SELECT count(*) FROM content_references WHERE session_id=?1 AND owner_kind='entry' AND owner_id=?2",rusqlite::params![s,owner],|r|r.get::<_,i64>(0))?)
        }).unwrap()
    };
    let abandoned = uuid::Uuid::new_v4().to_string();
    let staged =
        entry_stages::PreparedEntries::stage(store, &session, vec![input(&abandoned)]).unwrap();
    assert!(owners(store, &abandoned) > 0);
    drop(staged);
    assert_eq!(owners(store, &abandoned), 0);
    let published = uuid::Uuid::new_v4().to_string();
    store.append(&session, vec![input(&published)]).unwrap();
    let staged =
        entry_stages::PreparedEntries::stage(store, &session, vec![input(&published)]).unwrap();
    drop(staged);
    assert!(owners(store, &published) > 0);
    let queued = uuid::Uuid::new_v4().to_string();
    let user: SessionEntry = serde_json::from_value(input(&queued)).unwrap();
    let ack = crate::runtime::RunAck::queued(run.clone(), 1, 1);
    manager
        .record_queued_receipt(&session, "request", &ack, "fingerprint", &json!({}), &user)
        .unwrap();
    assert!(owners(store, &queued) > 0);
    let merged_run = uuid::Uuid::new_v4().to_string();
    let merged = uuid::Uuid::new_v4().to_string();
    let mut merged_input = input(&merged);
    merged_input["meta"]["run_id"] = json!(merged_run);
    let merged_user: SessionEntry = serde_json::from_value(merged_input).unwrap();
    manager
        .record_queued_receipt(
            &session,
            "merged-request",
            &crate::runtime::RunAck::queued(merged_run.clone(), 2, 2),
            "merged-fingerprint",
            &json!({}),
            &merged_user,
        )
        .unwrap();
    assert!(owners(store, &merged) > 0);
    let mut started_input = input(&queued);
    started_input["meta"]["coalesced_run_ids"] = json!([merged_run]);
    manager
        .append_run_start(
            &session,
            serde_json::from_value(started_input).unwrap(),
            SessionEntry::run_started(&run, 1),
        )
        .unwrap();
    assert_eq!(
        owners(store, &merged),
        0,
        "coalesced receipt must release its binary unpublished owner"
    );
    assert!(
        owners(store, &queued) > 0,
        "started input remains owned by its committed entry"
    );
    drop(manager);
    let manager = Manager::new(dir);
    let store = manager.storage().unwrap();
    assert_eq!(
        store
            .entries(&session)
            .unwrap()
            .iter()
            .find(|e| e["id"] == queued)
            .unwrap()["content"][0]["text"],
        input(&queued)["content"]
    );
}

#[test]
fn native_checkpoint_receipt_resolves_cutoff_and_cached_entry_after_reopen() {
    use crate::session::{checkpoint_to_entry, compaction_ops::CompactionClaim, Manager};
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("sessions");
    let manager = Manager::new(dir.clone());
    let store = manager.storage().unwrap();
    let session = uuid::Uuid::new_v4().to_string();
    let info = uuid::Uuid::new_v4().to_string();
    let user = uuid::Uuid::new_v4().to_string();
    let answer = uuid::Uuid::new_v4().to_string();
    store.reserve_segmented(&session).unwrap();
    store.replace(&session,vec![
        json!({"id":info,"type":"session_info","timestamp":"2026-10-27T00:00:00Z","content":{"model":"mock"}}),
        json!({"id":user,"type":"user","role":"user","timestamp":"2026-10-27T00:00:01Z","content":"requirement"}),
        json!({"id":answer,"type":"assistant","role":"assistant","timestamp":"2026-10-27T00:00:02Z","content":"answer"}),
    ]).unwrap();
    let checkpoint:crate::compaction::ContextCheckpoint=serde_json::from_value(json!({"entry_id":uuid::Uuid::new_v4().to_string(),"checkpoint_id":uuid::Uuid::new_v4().to_string(),"covered_from_entry_id":user,"cutoff_entry_id":answer,
        "summary":[{"type":"text","text":"existing context feature"}],"protected_entry_ids":[user],"tokens_before":100000,"tokens_after":10000,
        "trigger":"manual","algorithm_version":"deterministic-evidence-v1","model":"mock","context_window":128000,"created_at":"2026-10-27T00:00:03Z"})).unwrap();
    let policy = json!({"model":"mock","policy":1});
    let (key, input) = match manager
        .claim_compaction(&session, policy.clone(), "original-operation")
        .unwrap()
    {
        CompactionClaim::New { key, input } => (key, input),
        _ => panic!("expected new checkpoint receipt"),
    };
    let receipt = json!({"checkpoint":null,"result":{"checkpointId":checkpoint.checkpoint_id,"tokensAfter":10000}});
    manager
        .finish_compaction(
            &session,
            &key,
            &input,
            Some(checkpoint_to_entry(&checkpoint)),
            receipt.clone(),
        )
        .unwrap();
    manager
        .finish_compaction(
            &session,
            &key,
            &input,
            Some(checkpoint_to_entry(&checkpoint)),
            receipt,
        )
        .unwrap();
    drop(manager);
    let manager = Manager::new(dir);
    match manager.claim_compaction(&session, policy, "retry").unwrap() {
        CompactionClaim::Cached {
            value,
            operation_id,
        } => {
            assert_eq!(operation_id, "original-operation");
            assert_eq!(value["checkpoint"], json!(checkpoint));
        }
        _ => panic!("expected durable cached receipt"),
    }
}
