use super::*;
use crate::session::{Manager, Session};
use future_rpc::proto::RpcCommand;

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
