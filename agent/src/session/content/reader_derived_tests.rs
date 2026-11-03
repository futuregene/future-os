//! Derived range/cache specifications, authored for final validation only.
use super::*;
use crate::session::content::{bind_encoded, entries::stage_named};

fn fixture(raw: &str) -> (tempfile::TempDir, SqliteStore, Snapshot, Entry, Reference) {
    let root = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&root.path().join("agent.db")).unwrap();
    store.reserve_segmented("s").unwrap();
    store.replace("s",vec![json!({"id":"info","type":"session_info","timestamp":"2026-10-26T00:00:00Z","content":{"model":"mock"}})]).unwrap();
    let range = stage_named(&store, "s", "entry", "a", "/content/0/args", raw, false).unwrap();
    store.db.call(move |db| {
        let tx=crate::session::database::begin_write_transaction(db)?;
        tx.execute("UPDATE content_references SET encoding='tool_args_string_v1' WHERE session_id='s' AND owner_id='a'",[])?;
        crate::session::sqlite_store::insert_transcript_entries(&tx,"s",vec![json!({"id":"a","type":"assistant","role":"assistant","timestamp":"2026-10-26T00:00:01Z","meta":{"run_id":"r"},"content":[{"type":"tool_call","id":"call","name":"read","args":""}]})])?;
        tx.commit()?;Ok(())
    }).unwrap();
    let scope = Snapshot::capture(&store, "s").unwrap();
    let entry = scope.page(&store, -1).unwrap().pop().unwrap();
    let reference = Reference {
        field: "/content/0/args".into(),
        range,
        encoding: Encoding::ToolArgumentsStringV1,
    };
    (root, store, scope, entry, reference)
}

#[test]
fn repaired_argument_fragments_share_a_disposable_representation_and_recheck_ownership() {
    let raw = format!("{{\"path\":\"{}", "中🙂\\n".repeat(3000));
    let (_root, store, scope, entry, reference) = fixture(&raw);
    let mut expected = Value::String(raw);
    future_rpc::tool_arguments::repair_string_v1(&mut expected);
    let expected = serde_json::to_string(&expected).unwrap();
    let first = scope
        .derived_text(&store, &entry, &reference, true)
        .unwrap();
    let second = scope
        .derived_text(&store, &entry, &reference, true)
        .unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    let mut actual = String::new();
    while actual.len() < first.len {
        actual.push_str(&first.read(actual.len(), 251).unwrap());
    }
    assert_eq!(actual, expected);
    // Removing this exact cache key releases acceleration, never SQL content.
    let key = scope.render_key(
        &entry,
        &json!([
            "original-text-v1",
            reference.field,
            reference.range.id,
            reference.range.offset,
            reference.range.length,
            reference.encoding.name(),
            true
        ])
        .to_string(),
    );
    CACHE.lock().remove(&key);
    let rebuilt = scope
        .derived_text(&store, &entry, &reference, true)
        .unwrap();
    assert!(!Arc::ptr_eq(&first, &rebuilt));
    assert_eq!(rebuilt.len, first.len);
    assert_eq!(rebuilt.read(0, 251).unwrap(), first.read(0, 251).unwrap());
    store.db.call(|db| {
        db.execute("UPDATE content_references SET byte_length=byte_length-1 WHERE session_id='s' AND owner_id='a' AND field='/content/0/args'",[])?;Ok(())
    }).unwrap();
    assert!(scope
        .derived_text(&store, &entry, &reference, true)
        .is_err());
    assert!(scope.verify_reference(&store, &entry, &reference).is_err());
}

#[test]
fn shell_original_text_is_the_shared_summary_and_keeps_full_execution_facts() {
    let facts = future_rpc::shell_result::ShellResult {
        command: "synthetic".into(),
        cwd: "/synthetic".into(),
        duration_ms: 2,
        status: future_rpc::shell_result::ShellStatus::Exited,
        exit_code: Some(1),
        is_soft_fail: false,
        is_error: true,
        attempts: vec![future_rpc::shell_result::ShellAttempt {
            status: future_rpc::shell_result::ShellStatus::Exited,
            exit_code: Some(1),
            duration_ms: 2,
            output: "stdout中🙂".repeat(30000),
            output_truncated: false,
            escalated: false,
        }],
        approval: Some("approved".into()),
        note: Some("actionable note".into()),
    };
    let (_root, store, _, _, _) = fixture("{}");
    let raw = serde_json::to_string(&facts).unwrap();
    let range = stage_named(&store, "s", "entry", "t", "/meta/shell_result", &raw, true).unwrap();
    store.db.call(move |db| {
        let tx=crate::session::database::begin_write_transaction(db)?;
        bind_encoded(&tx,"s","entry","t","/content/0/content",range,Encoding::ShellSummary)?;
        crate::session::sqlite_store::insert_transcript_entries(&tx,"s",vec![json!({"id":"t","type":"tool","role":"tool","timestamp":"2026-10-26T00:00:02Z","meta":{"run_id":"r","shell_result":""},"content":[{"type":"tool_result","tool_call_id":"call","content":"","is_error":true}]})])?;
        tx.commit()?;Ok(())
    }).unwrap();
    let scope = Snapshot::capture(&store, "s").unwrap();
    let entry = scope.page(&store, -1).unwrap().pop().unwrap();
    let reference = Reference {
        field: "/content/0/content".into(),
        range,
        encoding: Encoding::ShellSummary,
    };
    let body = scope
        .derived_text(&store, &entry, &reference, false)
        .unwrap();
    let expected = future_rpc::shell_result::model_summary(&facts);
    assert_eq!(body.len, expected.len());
    assert_eq!(body.read(0, body.len).unwrap(), expected);
    assert_eq!(
        scope.field(&store, &entry, "/meta/shell_result").unwrap(),
        Some(serde_json::to_value(&facts).unwrap())
    );
    assert!(Arc::ptr_eq(
        &body,
        &scope
            .derived_text(&store, &entry, &reference, false)
            .unwrap()
    ));
}

#[test]
fn an_over_budget_derived_source_fails_before_assembling_its_missing_body() {
    let (_root, store, scope, entry, mut reference) = fixture("{}");
    reference.range.length = MAX_BYTES as i64;
    let bytes = reference.range.length;
    store.db.call(move |db| {
        db.execute("UPDATE content_references SET byte_length=?1 WHERE session_id='s' AND owner_id='a'",[bytes])?;Ok(())
    }).unwrap();
    let error = scope
        .derived_text(&store, &entry, &reference, true)
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("memory budget"), "{error}");
}
