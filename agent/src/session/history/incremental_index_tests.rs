use super::*;
use crate::session::{Manager, Session};

fn assert_projection(manager: &Manager, id: &str) {
    let expected = project_entries(&manager.load(id).unwrap().entries);
    let actual = manager.history_page(id, None, Some(0), Some(1000)).unwrap();
    assert_eq!(actual["entries"], json!(expected));
}

#[test]
fn ordinary_appends_and_metadata_changes_do_not_rewrite_the_historical_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().join("sessions"));
    let mut session = Session::new("/synthetic", "mock");
    session.entries.push(SessionEntry::session_info(
        json!({"session_name":"old"}),
        "mock".into(),
        String::new(),
    ));
    for n in 0..50 {
        let mut user = SessionEntry::new_user("user", json!(format!("question-{n}")));
        user.meta = Some(json!({"run_id":format!("run-{n}")}));
        let mut assistant = SessionEntry::new_assistant(json!("answer"), Vec::new());
        assistant.meta = user.meta.clone();
        session.entries.extend([user, assistant]);
        session.entries.push(SessionEntry::run_terminal(
            &format!("run-{n}"),
            "completed",
            1,
            1,
            None,
        ));
    }
    manager.save(&session).unwrap();
    // A new exchange must not decode shapes from the already indexed prefix.
    // Re-arm the stamp after installing the decoding sentinel: this fixture
    // exercises a healthy index, while real out-of-band mutations must rebuild.
    manager.test_execute(
        "UPDATE history_shapes SET payload='off-page-invalid-shape' WHERE position=1;
         INSERT OR REPLACE INTO history_index_state(session_id,projection_version)
         SELECT id,1 FROM sessions;",
    );
    manager.test_execute(
        "CREATE TEMP TRIGGER forbid_prefix_reinsertion BEFORE INSERT ON history_display
        WHEN NEW.ordinal<99 BEGIN SELECT RAISE(ABORT,'historical prefix was rewritten'); END;",
    );
    manager
        .append_entries(
            &session.id,
            &[SessionEntry::new_user("user", json!("new question"))],
        )
        .unwrap();
    manager
        .append_entries(
            &session.id,
            &[SessionEntry::new_assistant(json!("new answer"), Vec::new())],
        )
        .unwrap();
    manager
        .append_entries(
            &session.id,
            &[SessionEntry::session_info(
                json!({"session_name":"new"}),
                "mock".into(),
                String::new(),
            )],
        )
        .unwrap();
    assert_projection(&manager, &session.id);
    // Retransmission and event-only commits must not reconcile any history.
    let duplicate = manager
        .load(&session.id)
        .unwrap()
        .entries
        .last()
        .unwrap()
        .clone();
    manager.test_execute(
        "CREATE TEMP TRIGGER forbid_all_reinsertion BEFORE INSERT ON history_display
        BEGIN SELECT RAISE(ABORT,'unchanged history was rewritten'); END;",
    );
    manager.append_entries(&session.id, &[duplicate]).unwrap();
}

#[test]
fn late_results_and_reused_tool_ids_expand_the_dirty_tail_to_their_old_owner() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().join("sessions"));
    let mut session = Session::new("/synthetic", "mock");
    let mut user = SessionEntry::new_user("user", json!("old question"));
    user.meta = Some(json!({"run_id":"old"}));
    let mut assistant = SessionEntry::new_assistant(
        json!(""),
        vec![crate::types::ToolCall {
            id: "shared-call".into(),
            call_type: "function".into(),
            function: crate::types::ToolCallFn {
                name: "read".into(),
                arguments: json!({}),
            },
        }],
    );
    assistant.meta = user.meta.clone();
    session.entries.extend([user, assistant]);
    session
        .entries
        .push(SessionEntry::run_terminal("old", "completed", 1, 2, None));
    manager.save(&session).unwrap();
    for n in 0..5 {
        manager
            .append_entries(
                &session.id,
                &[SessionEntry::new_user("user", json!(format!("later-{n}")))],
            )
            .unwrap();
    }
    let mut result = SessionEntry::new_tool("shared-call", "late real result");
    result.meta = Some(json!({"run_id":"old"}));
    manager.append_entries(&session.id, &[result]).unwrap();
    assert_projection(&manager, &session.id);
    manager
        .append_entries(
            &session.id,
            &[SessionEntry::new_tool("shared-call", "duplicate result")],
        )
        .unwrap();
    assert_projection(&manager, &session.id);
    let mut late_assistant = SessionEntry::new_assistant(json!("late final answer"), Vec::new());
    late_assistant.meta = Some(json!({"run_id":"old"}));
    manager
        .append_entries(&session.id, &[late_assistant])
        .unwrap();
    assert_projection(&manager, &session.id);
}

#[test]
fn replacing_history_with_no_visible_entries_clears_the_derived_index() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().join("sessions"));
    let mut session = Session::new("/synthetic", "mock");
    session
        .entries
        .push(SessionEntry::new_user("user", json!("old question")));
    manager.save(&session).unwrap();
    session.entries.clear();
    manager.save(&session).unwrap();
    assert_eq!(
        manager
            .history_page(&session.id, None, Some(0), Some(1000))
            .unwrap()["entries"],
        json!([])
    );
    session.entries.push(SessionEntry {
        entry_type: "model_change".into(),
        ..SessionEntry::new_user("", json!({"model":"mock"}))
    });
    manager.save(&session).unwrap();
    assert_projection(&manager, &session.id);
}
