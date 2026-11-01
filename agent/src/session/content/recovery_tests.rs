use super::*;
use crate::session::content::ContentDigest;
use crate::session::{content::journal::Writer, identity::BoundIdentity, Manager};

fn seed(manager: &Manager, malformed: bool) -> (String, String) {
    let store = manager.storage().unwrap();
    let session = uuid::Uuid::new_v4().to_string();
    let run = uuid::Uuid::new_v4().to_string();
    store.bind_events(&session).unwrap();
    store.replace(&session, vec![
        json!({"id":uuid::Uuid::new_v4().to_string(),"type":"session_info","timestamp":"2026-10-28T00:00:00Z","content":{"cwd":".","model":"mock"}}),
        json!({"id":uuid::Uuid::new_v4().to_string(),"type":"run_started","timestamp":"2026-10-28T00:00:00Z","content":{"run_id":run,"epoch":1,"run_sequence":1}}),
    ]).unwrap();
    if malformed {
        // Valid SQLite/stream rows, with one semantically invalid run fact.
        // Other conversations and authoritative database structure are intact.
        Writer::default()
            .persist(
                store,
                &session,
                &[json!({
                    "session_id":session,"run_id":run,"epoch":1,"idx":0,"session_idx":-1,
                    "timestamp":"2026-10-28T00:00:01Z","event_type":"message_start",
                    "data":"{\"source_marker\":\"preserve original invalid fact\"}"
                })],
            )
            .unwrap();
    }
    (session, run)
}

fn status(manager: &Manager, session: &str, run: &str) -> String {
    let (session, run) = (session.to_owned(), run.to_owned());
    manager
        .storage()
        .unwrap()
        .db
        .read(move |db| {
            let run = BoundIdentity::run(db, &session, &run)?;
            let session = BoundIdentity::session(db, &session)?;
            Ok(db.query_row(
                "SELECT status FROM runs WHERE session_id=?1 AND run_id=?2",
                params![session, run],
                |row| row.get(0),
            )?)
        })
        .unwrap()
}

fn run_error(manager: &Manager, session: &str, run: &str) -> String {
    let (s, r) = (session.to_owned(), run.to_owned());
    manager
        .storage()
        .unwrap()
        .db
        .read(move |db| {
            let r = BoundIdentity::run(db, &s, &r)?;
            let s = BoundIdentity::session(db, &s)?;
            Ok(db.query_row(
                "SELECT coalesce(error,'') FROM runs WHERE session_id=?1 AND run_id=?2",
                params![s, r],
                |row| row.get(0),
            )?)
        })
        .unwrap()
}

fn stream_bytes(manager: &Manager, session: &str) -> Vec<String> {
    let session = session.to_owned();
    manager.storage().unwrap().db.read(move |db| {
        let session = BoundIdentity::session(db, &session)?;
        let mut query = db.prepare("SELECT c.metadata_json||x.descriptor_json FROM stream_changes c JOIN stream_contexts x ON x.id=c.context_id JOIN stream_runs r ON r.id=c.stream_id WHERE r.session_id=?1 ORDER BY c.id")?;
        let rows = query.query_map([session], |row| row.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }).unwrap()
}

#[test]
fn one_invalid_native_run_does_not_block_startup_or_erase_its_original_facts() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("sessions");
    let manager = Manager::new(directory.clone());
    let (broken, broken_run) = seed(&manager, true);
    let (healthy, healthy_run) = seed(&manager, false);
    let original = stream_bytes(&manager, &broken);
    let history = manager.storage().unwrap().entries(&broken).unwrap();
    drop(manager);

    let manager = Manager::new(directory.clone());
    manager.initialize().unwrap();
    assert_eq!(status(&manager, &broken, &broken_run), "recovery_failed");
    assert_eq!(status(&manager, &healthy, &healthy_run), "interrupted");
    assert_eq!(stream_bytes(&manager, &broken), original);
    assert_eq!(
        manager.storage().unwrap().entries(&broken).unwrap(),
        history
    );
    assert!(manager.ensure_execution_allowed(&broken).is_err());
    assert!(manager
        .settle_run_if_unfinished(
            &broken,
            &broken_run,
            crate::session::RUN_STATE_CANCELLED,
            "watchdog fallback"
        )
        .is_err());
    assert_eq!(status(&manager, &broken, &broken_run), "recovery_failed");
    manager.ensure_execution_allowed(&healthy).unwrap();
    assert_eq!(manager.storage_health().unwrap()["failedRunRecoveries"], 1);
    // Recovery is stable across restarts, and cannot invent a successful retry.
    drop(manager);
    let manager = Manager::new(directory);
    manager.initialize().unwrap();
    assert_eq!(stream_bytes(&manager, &broken), original);
    assert!(manager.retry_session_recovery(&broken).is_err());
    assert_eq!(status(&manager, &broken, &broken_run), "recovery_failed");
    assert_eq!(stream_bytes(&manager, &broken), original);
    let backups = std::fs::read_dir(root.path().join("backups"))
        .unwrap()
        .count();
    assert_eq!(backups, 1, "explicit retry preserves a pre-retry snapshot");
}

/// A run interrupted between `tool_input_start` and `message_end` leaves the
/// tool call's arguments staged but unpublished. Recovery must publish the call
/// itself — id, name, and a provider-metadata slot — and attach the staged
/// arguments through the body link, so the reader materializes exactly the bytes
/// that were committed and nothing is replayed from the model.
#[test]
fn a_recovered_tool_input_end_publishes_the_call_without_replaying_its_arguments() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("sessions");
    let manager = Manager::new(directory.clone());
    let (session, run) = seed(&manager, false);

    let identity = StreamIdentity {
        session_id: session.clone(),
        run_id: run.clone(),
        epoch: 1,
    };
    let arguments = r#"{"cmd":"ls"}"#;
    let mut digest = ContentDigest::default();
    digest.push(arguments).unwrap();
    let append = ContentAppend {
        key: "args".into(),
        offset: 0,
        text: arguments.into(),
        seal: Some(digest),
    };
    let metadata_body = "{}";
    let mut metadata_digest = ContentDigest::default();
    metadata_digest.push(metadata_body).unwrap();
    let append_metadata = ContentAppend {
        key: "provider".into(),
        offset: 0,
        text: metadata_body.into(),
        seal: Some(metadata_digest),
    };
    let cursor = i64::try_from(arguments.len()).unwrap();
    // Each change owns a distinct cursor window: the table keys a run's changes
    // by cursor, and a shared window reads as a conflicting retry.
    let change = move |(first, last): (i64, i64),
                       event: &str,
                       data: serde_json::Value,
                       bodies: Vec<BodyLink>| Change {
        first_cursor: first,
        last_cursor: last,
        event_type: event.into(),
        metadata: json!({"data": data, "timestamp": "2026-11-01T00:00:00Z"}),
        bodies,
    };
    let args_body = move || {
        vec![
            BodyLink {
                field: "/data/tool_args".into(),
                encoding: Encoding::Json,
                key: "args".into(),
                offset: 0,
                length: cursor,
            },
            // The provider's metadata travels on its own channel; when it is
            // present the call carries an empty object rather than the event's
            // copied value.
            BodyLink {
                field: "/data/provider_metadata".into(),
                encoding: Encoding::Json,
                key: "provider".into(),
                offset: 0,
                length: 2,
            },
        ]
    };
    manager
        .storage()
        .unwrap()
        .db
        .call(move |db| {
            commit_batch(
                db,
                &identity,
                vec![append, append_metadata],
                vec![
                    change(
                        (0, 1),
                        "message_start",
                        json!({"entry_id": "e1"}),
                        Vec::new(),
                    ),
                    change(
                        (2, 3),
                        "tool_start",
                        json!({
                            "entry_id": "e1",
                            "phase": "input",
                            "tc_index": 0,
                            "tool_id": "t1",
                            "tool_name": "shell",
                            "content_key": "call-1"
                        }),
                        Vec::new(),
                    ),
                    // A `thinking_end` with no provider-metadata body: the call
                    // keeps the provider snapshot the event itself carries.
                    change(
                        (4, 5),
                        "thinking_end",
                        json!({
                            "entry_id": "e1",
                            "content_key": "call-1",
                            "provider_metadata": {"openai": {"reasoning_id": "rs_1"}}
                        }),
                        Vec::new(),
                    ),
                    change(
                        (6, 7),
                        "tool_input_end",
                        // The link names a metadata field, so the field must exist.
                        json!({
                            "entry_id": "e1",
                            "tc_index": 0,
                            "tool_id": "t1",
                            "tool_name": "shell",
                            "content_key": "call-1",
                            "tool_args": "",
                            "provider_metadata": {}
                        }),
                        args_body(),
                    ),
                    change(
                        (8, cursor),
                        "message_end",
                        json!({"entry_id": "e1"}),
                        Vec::new(),
                    ),
                ],
            )?;
            Ok(())
        })
        .unwrap();
    assert_eq!(
        status(&manager, &session, &run),
        "running",
        "the fixture's run must be live for recovery to claim it"
    );
    drop(manager);

    let manager = Manager::new(directory);
    manager.initialize().unwrap();
    assert_eq!(
        status(&manager, &session, &run),
        "interrupted",
        "recovery settles the interrupted run instead of leaving it running"
    );

    let entries = manager.storage().unwrap().entries(&session).unwrap();
    let recovered = entries
        .iter()
        .find(|entry| entry["type"] == "assistant")
        .unwrap_or_else(|| panic!("recovery must publish the interrupted message: {entries:?}"));
    let block = &recovered["content"][0];
    assert_eq!(block["type"], "tool_call", "{block}");
    assert_eq!(block["id"], "t1", "{block}");
    assert_eq!(block["name"], "shell", "{block}");
    assert_eq!(
        block["args"],
        json!({"cmd": "ls"}),
        "the staged arguments reach the published call through the body link"
    );
    // The provider-metadata *body* wins here (the call carries an empty object),
    // while the reasoning snapshot from the event itself is what the thinking
    // block keeps — both halves are asserted so neither can silently swap.
    assert_eq!(block["provider_metadata"], json!({}), "{block}");
    let thinking = &recovered["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|block| block["type"] == "reasoning")
        .cloned();
    if let Some(thinking) = thinking {
        assert_eq!(
            thinking["provider_metadata"]["openai"]["reasoning_id"], "rs_1",
            "a reasoning snapshot with no body channel is copied from the event: {thinking}"
        );
    }
}

/// A stream whose immutable context no longer parses cannot be replayed. Recovery
/// must **isolate** the run — recording why — rather than publish half a message
/// or quietly erase the facts it cannot read. This is the fault path the
/// `with_context` wrappers around the page read exist for.
#[test]
fn a_stream_whose_context_does_not_parse_is_isolated_with_its_reason() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("sessions");
    let manager = Manager::new(directory.clone());
    let (session, run) = seed(&manager, false);

    let identity = StreamIdentity {
        session_id: session.clone(),
        run_id: run.clone(),
        epoch: 1,
    };
    let text = "hi";
    let mut digest = ContentDigest::default();
    digest.push(text).unwrap();
    manager
        .storage()
        .unwrap()
        .db
        .call(move |db| {
            commit_batch(
                db,
                &identity,
                vec![ContentAppend {
                    key: "block".into(),
                    offset: 0,
                    text: text.into(),
                    seal: Some(digest),
                }],
                vec![Change {
                    first_cursor: 0,
                    last_cursor: 2,
                    event_type: "message_start".into(),
                    metadata: json!({
                        "data": {"entry_id": "e1"},
                        "timestamp": "2026-11-01T00:00:00Z"
                    }),
                    bodies: Vec::new(),
                }],
            )?;
            Ok(())
        })
        .unwrap();

    // The immutable descriptor is what every change in the stream is decoded
    // against; if it cannot be parsed, nothing in the run can be trusted.
    let (s, r) = (session.clone(), run.clone());
    manager
        .storage()
        .unwrap()
        .db
        .call(move |db| {
            let run = BoundIdentity::run(db, &s, &r)?;
            db.execute(
                // The immutable context is trigger-protected, so the fault goes in
                // the one row the schema leaves writable: the change's metadata.
                "UPDATE stream_changes SET metadata_json='{}' WHERE stream_id IN (SELECT id FROM stream_runs WHERE run_id=?1)",
                params![run],
            )?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();
    drop(manager);

    let manager = Manager::new(directory);
    manager.initialize().unwrap();

    assert_eq!(
        status(&manager, &session, &run),
        "recovery_failed",
        "a stream that cannot be decoded must be isolated, not settled as recovered"
    );
    let error = run_error(&manager, &session, &run);
    assert!(
        error.contains("invalid encoded recovery data") || error.contains("decode recovery stream"),
        "the recorded reason names the decode, not a guess: {error}"
    );
    // And the isolation is durable: the original stream rows are still there to
    // be inspected, which is the point of isolating instead of deleting.
    let (s, r) = (session.clone(), run.clone());
    let changes: i64 = manager
        .storage()
        .unwrap()
        .db
        .read(move |db| {
            let run = BoundIdentity::run(db, &s, &r)?;
            Ok(db.query_row(
                "SELECT count(*) FROM stream_changes WHERE stream_id IN (SELECT id FROM stream_runs WHERE run_id=?1)",
                params![run],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(
        changes, 1,
        "the unreadable stream is preserved, not dropped"
    );
}

#[test]
fn explicit_retry_can_settle_repaired_history_without_executing_work() {
    let root = tempfile::tempdir().unwrap();
    let manager = Manager::new(root.path().join("sessions"));
    let (session, run) = seed(&manager, false);
    let (s, r) = (session.clone(), run.clone());
    manager.storage().unwrap().db.call(move |db| {
        let r = BoundIdentity::run(db, &s, &r)?;
        let s = BoundIdentity::session(db, &s)?;
        db.execute("UPDATE runs SET status='recovery_failed',error='previous failure repaired offline' WHERE session_id=?1 AND run_id=?2", params![s,r])?;
        Ok(())
    }).unwrap();
    assert!(manager.ensure_execution_allowed(&session).is_err());
    manager.retry_session_recovery(&session).unwrap();
    manager.ensure_execution_allowed(&session).unwrap();
    assert_eq!(status(&manager, &session, &run), "interrupted");
    let entries = manager.storage().unwrap().entries(&session).unwrap();
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry["type"] == "run_terminal")
            .count(),
        1
    );
}

#[test]
fn database_failures_are_not_downgraded_to_isolated_runs() {
    let full = anyhow::Error::from(rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_FULL),
        None,
    ))
    .context(InvalidRecoveryData("tagged while reading run"));
    assert!(!is_local_recovery_failure(&full));
    let io = anyhow::Error::from(std::io::Error::other("disk IO failure"))
        .context(InvalidRecoveryData("tagged while reading run"));
    assert!(!is_local_recovery_failure(&io));
    let worker = anyhow::anyhow!("SQLite worker is unavailable");
    assert!(!is_local_recovery_failure(&worker));

    let root = tempfile::tempdir().unwrap();
    let manager = Manager::new(root.path().join("sessions"));
    let (session, run) = seed(&manager, false);
    manager.storage().unwrap().db.call(|db| {
        db.execute_batch("CREATE TRIGGER fail_terminal BEFORE INSERT ON entries WHEN NEW.entry_type='run_terminal' BEGIN SELECT RAISE(ABORT,'injected storage refusal'); END;")?;
        Ok(())
    }).unwrap();
    assert!(manager.storage().unwrap().recover_segmented_runs().is_err());
    assert_eq!(status(&manager, &session, &run), "running");
    assert_eq!(manager.storage_health().unwrap()["failedRunRecoveries"], 0);
}

/// A crash after a shell tool finished but before its result entry was written:
/// recovery rebuilds the tool entry from the canonical facts and re-links both
/// bodies (the model text and the structured facts) instead of copying them.
#[test]
fn a_crashed_shell_result_is_recovered_with_both_of_its_bodies() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("sessions");
    let manager = Manager::new(directory.clone());
    let (session, run) = seed(&manager, false);
    let store = manager.storage().unwrap().clone();
    let result = future_rpc::shell_result::ShellResult {
        command: "exit 3".into(),
        cwd: "/w".into(),
        duration_ms: 7,
        status: future_rpc::shell_result::ShellStatus::Exited,
        exit_code: Some(3),
        is_soft_fail: false,
        is_error: true,
        attempts: vec![future_rpc::shell_result::ShellAttempt {
            status: future_rpc::shell_result::ShellStatus::Exited,
            exit_code: Some(3),
            duration_ms: 7,
            output: "boom\n".into(),
            output_truncated: false,
            escalated: false,
        }],
        approval: None,
        note: None,
    };
    let summary = future_rpc::shell_result::model_summary(&result);
    Writer::default()
        .persist(
            &store,
            &session,
            &[json!({
                "session_id":session,"run_id":run,"epoch":1,"idx":0,"session_idx":-1,
                "timestamp":"2026-10-28T00:00:01Z","event_type":"tool_end",
                "data":serde_json::json!({
                    "entry_id":"tool-entry-1","tool_id":"tool-1","tool_name":"shell","text":summary,
                    "shell_result":serde_json::to_value(&result).unwrap(),
                }).to_string()
            })],
        )
        .unwrap();
    drop(manager);

    // The next start recovers the unfinished run.
    let manager = Manager::new(directory);
    manager.initialize().unwrap();
    assert_eq!(status(&manager, &session, &run), "interrupted");
    let entries = manager.storage().unwrap().entries(&session).unwrap();
    let tool = entries
        .iter()
        .find(|entry| entry["type"] == "tool")
        .unwrap_or_else(|| panic!("no recovered tool entry: {entries:?}"));
    assert_eq!(tool["meta"]["recovered_partial"], true);
    assert_eq!(tool["meta"]["run_id"], run);
    // The structured facts came back through their own reference, and the model
    // text through the summary reference of the same body.
    assert_eq!(
        tool["meta"]["shell_result"]["exit_code"],
        serde_json::json!(3),
        "{tool}"
    );
    assert_eq!(tool["content"][0]["content"], summary, "{tool}");
    assert_eq!(tool["content"][0]["is_error"], true);
}
