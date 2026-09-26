use crate::remote::protocol::IncomingCmd;
use crate::remote::services::ReplySink;
use serde_json::{json, Value};

use super::reply;

pub(super) async fn execute(cmd: &IncomingCmd, sink: &dyn ReplySink) {
    match cmd.cmd_type.as_str() {
        "list_sessions" => match crate::remote_host::catalog::sessions("") {
            Some((payload, _)) => reply(sink, true, payload, None).await,
            None => reply(sink, false, Value::Null, Some("catalog_unavailable")).await,
        },
        "list_workspaces" => match crate::remote_host::catalog::workspaces() {
            Some((payload, _)) => reply(sink, true, payload, None).await,
            None => reply(sink, false, Value::Null, Some("catalog_unavailable")).await,
        },
        "generate_session_title" => {
            match crate::agent_bridge::generate_session_title(
                cmd.session_id.clone(),
                cmd.mode.clone(),
            )
            .await
            {
                Ok(data) => reply(sink, true, data, None).await,
                Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
            }
        }
        "set_session_name" => {
            match crate::agent_bridge::rename_session(cmd.session_id.clone(), cmd.name.clone())
                .await
            {
                Ok(()) => {
                    if let Ok(Some(thread)) =
                        crate::store::find_thread_by_agent_session(&cmd.session_id)
                    {
                        crate::emit_remote_activity(&thread.id);
                    }
                    reply(sink, true, json!({}), None).await
                }
                Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
            }
        }
        "set_session_pinned" => {
            match crate::store::pin_thread(crate::store::PinThreadInput {
                thread_id: cmd.thread_id.clone(),
                pinned: cmd.pinned,
            }) {
                Ok(_) => {
                    crate::emit_remote_activity(&cmd.thread_id);
                    reply(sink, true, json!({}), None).await
                }
                Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
            }
        }
        "delete_session" => {
            if cmd.thread_id.is_empty() {
                reply(sink, false, Value::Null, Some("missing thread_id")).await;
            } else {
                // Idempotent: the phone deletes a selection one session at a
                // time, so a child of a parent it already deleted (or a row
                // another client removed in the meantime) is gone, not an
                // error to report back.
                match crate::store::get_thread(&cmd.thread_id) {
                    Ok(None) => reply(sink, true, json!({}), None).await,
                    Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
                    Ok(Some(_)) => {
                        // The desktop deletes a conversation's descendants with
                        // it, exactly like its own GUI delete.
                        match crate::store::delete_thread_tree(&cmd.thread_id, false) {
                            Ok(_) => {
                                crate::emit_remote_activity(&cmd.thread_id);
                                reply(sink, true, json!({}), None).await
                            }
                            Err(error) => {
                                reply(sink, false, Value::Null, Some(&error.to_string())).await
                            }
                        }
                    }
                }
            }
        }
        "set_workspace_pinned" => {
            if cmd.workspace_id.is_empty() {
                reply(sink, false, Value::Null, Some("missing workspace_id")).await;
            } else {
                match crate::store::pin_workspace(crate::store::PinWorkspaceInput {
                    workspace_id: cmd.workspace_id.clone(),
                    pinned: cmd.pinned,
                }) {
                    Ok(_) => match crate::remote_host::catalog::workspaces() {
                        Some((payload, _)) => reply(sink, true, payload, None).await,
                        None => reply(sink, false, Value::Null, Some("catalog_unavailable")).await,
                    },
                    Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
                }
            }
        }
        "delete_workspace" => {
            if cmd.workspace_id.is_empty() {
                reply(sink, false, Value::Null, Some("missing workspace_id")).await;
            } else {
                match crate::commands::delete_workspace(cmd.workspace_id.clone()).await {
                    Ok(_) => {
                        crate::emit_threads_updated();
                        reply(sink, true, json!({}), None).await
                    }
                    Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
                }
            }
        }
        _ => unreachable!("catalog handler received {}", cmd.cmd_type),
    }
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;

    /// A phone that opens the workspace list before the desktop's store is
    /// ready must be told the catalogue is unavailable, not shown an empty
    /// workspace list: "no workspaces" and "cannot read workspaces" are
    /// different answers, and only one of them means the user has none.
    #[tokio::test]
    async fn a_store_that_cannot_be_read_is_not_reported_as_an_empty_catalogue() {
        let _home = crate::remote::test_support::HomeGuard::new("catalog-unavailable");
        let sink = crate::remote::test_support::RecordingSink::default();
        let cmd = IncomingCmd {
            cmd_type: "list_workspaces".into(),
            ..Default::default()
        };
        execute(&cmd, &sink).await;
        let (success, _data, error) = sink.last();
        assert!(!success, "an unreadable catalogue must not succeed");
        assert_eq!(error.as_deref(), Some("catalog_unavailable"));
    }

    /// Pinning a workspace is answered from a *fresh* snapshot. Here the write
    /// succeeds and the snapshot read does not, and the phone must be told the
    /// catalogue is unavailable rather than handed the previous snapshot as if
    /// the pin had taken effect. The store's own write path cannot produce a row
    /// that fails to decode, but an out-of-band writer can (a database from a
    /// different schema version, a hand-edited file) — so the fixture writes one
    /// row whose `name` column holds an integer, which the row decoder refuses
    /// to read as text. `pin_workspace` selects its row by id and is unaffected.
    #[tokio::test]
    async fn a_pinned_workspace_whose_snapshot_cannot_be_read_is_not_reported_as_pinned() {
        use crate::remote::test_support::{init_store, raw_store_connection, HomeGuard};
        let _home = HomeGuard::new("catalog-unavailable-after-pin");
        init_store();
        {
            let conn = raw_store_connection();
            conn.execute_batch(
                "INSERT INTO workspaces (
                     id, name, kind, path, created_at, updated_at
                 ) VALUES
                     ('ws_good', 'Good', 'user', '/tmp/good', 1, 1),
                     -- A BLOB, not an integer: the column's TEXT affinity would
                     -- convert a number to text on insert and the row would then
                     -- decode cleanly. A blob is stored as given.
                     ('ws_poison', X'00FF', 'user', '/tmp/poison', 1, 1);",
            )
            .expect("write the undecodable row");
        }
        assert!(
            crate::store::list_workspaces().is_err(),
            "the fixture must make the workspace list unreadable"
        );

        let sink = crate::remote::test_support::RecordingSink::default();
        let cmd = IncomingCmd {
            cmd_type: "set_workspace_pinned".into(),
            workspace_id: "ws_good".into(),
            pinned: true,
            ..Default::default()
        };
        execute(&cmd, &sink).await;
        let (success, _data, error) = sink.last();
        assert!(
            !success,
            "the pin must not be confirmed from a snapshot we cannot read"
        );
        assert_eq!(error.as_deref(), Some("catalog_unavailable"));
    }

    /// A failed lookup is not "the thread is already gone". The delete is
    /// idempotent on purpose: `Ok(None)` means the phone asked to remove a row
    /// that no longer exists, which is success. A store *read error* looks
    /// nothing like that, and reporting it as success would tell the phone a
    /// session is deleted while the desktop still holds it. An uninitialized
    /// store (the desktop has not finished starting) is the same failing state
    /// the catalogue test above uses.
    #[tokio::test]
    async fn a_failed_lookup_is_not_reported_as_already_deleted() {
        let _home = crate::remote::test_support::HomeGuard::new("catalog-delete-lookup");
        assert!(
            crate::store::get_thread("thread-1").is_err(),
            "the fixture must put the lookup in a failing state"
        );

        let sink = crate::remote::test_support::RecordingSink::default();
        let cmd = IncomingCmd {
            cmd_type: "delete_session".into(),
            thread_id: "thread-1".into(),
            ..Default::default()
        };
        execute(&cmd, &sink).await;
        let (success, _data, error) = sink.last();
        assert!(
            !success,
            "an unreadable store must not be reported as a completed delete"
        );
        assert!(
            error.is_some_and(|message| !message.is_empty()),
            "the store's own read error is surfaced to the phone"
        );
    }

    /// `get_thread` can succeed and the delete still fail. The handler must
    /// then report the failure rather than confirm a delete that did not
    /// happen — the phone would drop the session while the desktop kept it.
    /// The fixture provokes a store write failure the way the store's own
    /// `db.rs` tests do: an aborting trigger. A database damaged out of band
    /// (an older schema, a hand-edit) can leave exactly that behind, and the
    /// trigger only bites the delete, so the lookup that precedes it still
    /// answers normally.
    #[tokio::test]
    async fn a_delete_the_store_refuses_is_not_reported_as_deleted() {
        use crate::remote::test_support::{init_store, raw_store_connection, HomeGuard};
        let _home = HomeGuard::new("catalog-delete-refused");
        init_store();
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("Phone".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some("sess-catalog-delete-refused".to_string()),
        })
        .expect("create the thread to delete");
        {
            let conn = raw_store_connection();
            conn.execute_batch(
                "CREATE TRIGGER refuse_thread_delete BEFORE DELETE ON threads
                 BEGIN SELECT RAISE(ABORT, 'thread delete refused'); END;",
            )
            .expect("install the failing-write fixture");
        }

        let sink = crate::remote::test_support::RecordingSink::default();
        let cmd = IncomingCmd {
            cmd_type: "delete_session".into(),
            thread_id: thread.id.clone(),
            ..Default::default()
        };
        execute(&cmd, &sink).await;

        let (success, _data, error) = sink.last();
        assert!(!success, "a delete the store refused must not be confirmed");
        let message = error.expect("a failed delete carries the store's reason");
        assert!(
            message.contains("refused"),
            "the store's own error is surfaced: {message}"
        );
        assert!(
            crate::store::get_thread(&thread.id)
                .expect("get after a refused delete")
                .is_some(),
            "the refused delete must leave the row in place"
        );
    }

    /// This handler only ever sees the names the business dispatcher routes to
    /// it. A name from another family reaching it means the routing table and
    /// the handler disagreed, which must be loud: answering `success:false`
    /// would look to the phone like an ordinary refusal for a command that was
    /// never implemented.
    #[tokio::test]
    #[should_panic(expected = "handler received")]
    async fn a_command_from_another_family_is_not_answered() {
        let sink = crate::remote::test_support::RecordingSink::default();
        let cmd = IncomingCmd {
            bridge_instance_id: String::new(),
            session_id: String::new(),
            run_id: String::new(),
            cmd_type: "get_settings".into(),
            ..Default::default()
        };
        execute(&cmd, &sink).await;
    }
}
