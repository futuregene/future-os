use super::*;
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
