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
        "fork_session" => {
            let parent_session_id = cmd.session_id.trim();
            let source_entry_id = cmd.source_entry_id.trim();
            let request_id = cmd.id.trim();
            if parent_session_id.is_empty() || source_entry_id.is_empty() {
                reply(
                    sink,
                    false,
                    Value::Null,
                    Some("missing session or source entry"),
                )
                .await;
                return;
            }
            if request_id.is_empty() {
                reply(sink, false, Value::Null, Some("missing request identity")).await;
                return;
            }
            // The phone addresses the parent by its Agent session id, exactly
            // like the other session-scoped mutations; the Desktop's fork path
            // is thread-scoped.
            let parent = match crate::store::find_thread_by_agent_session(parent_session_id) {
                Ok(Some(thread)) => thread,
                Ok(None) => {
                    reply(
                        sink,
                        false,
                        Value::Null,
                        Some("Fork source thread could not be loaded."),
                    )
                    .await;
                    return;
                }
                Err(error) => {
                    reply(sink, false, Value::Null, Some(&error.to_string())).await;
                    return;
                }
            };
            // `cmd.id` doubles as the fork request identity: the Desktop's
            // single-flight cache replays a retried command, and the Agent
            // dedupes a repeated `client_request_id`, so a lost reply can never
            // create a second child branch.
            match crate::agent_bridge::fork_agent_session(&parent.id, source_entry_id, request_id)
                .await
            {
                Ok(new_thread_id) => match crate::store::get_thread(&new_thread_id) {
                    Ok(Some(thread)) => {
                        let new_session_id = thread.agent_session_id.unwrap_or_default();
                        if new_session_id.trim().is_empty() {
                            reply(sink, false, Value::Null, Some("Fork produced no session."))
                                .await;
                            return;
                        }
                        // The child thread is a store write, so the phone's own
                        // catalogue pull converges; these two events tell the
                        // Desktop GUI to re-list and show the conversation.
                        crate::emit_threads_updated();
                        crate::emit_remote_activity(&new_thread_id);
                        reply(
                            sink,
                            true,
                            json!({ "threadId": new_thread_id, "sessionId": new_session_id }),
                            None,
                        )
                        .await;
                    }
                    Ok(None) => {
                        reply(
                            sink,
                            false,
                            Value::Null,
                            Some("Forked thread could not be loaded."),
                        )
                        .await
                    }
                    Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
                },
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
        "create_workspace" => {
            let path = cmd.path.trim();
            if path.is_empty() {
                reply(sink, false, Value::Null, Some("missing path")).await;
                return;
            }
            // The phone types the host path (it has no folder picker for the
            // desktop), so this is exactly the Desktop's own `create_workspace`:
            // the directory must already exist and a path that names an existing
            // workspace reopens that row instead of duplicating it.
            let name = cmd.name.trim();
            let input = crate::store::CreateWorkspaceInput {
                name: (!name.is_empty()).then(|| name.to_string()),
                path: path.to_string(),
                description: None,
                create_directory: Some(false),
            };
            match crate::commands::create_workspace(input) {
                Ok(workspace) => {
                    crate::emit_threads_updated();
                    // Answer from the fresh catalogue so the phone's own version
                    // gate applies, and carry the created row so it can select it
                    // without re-deriving the id from its path.
                    match crate::remote_host::catalog::workspaces() {
                        Some((mut payload, _)) => {
                            if let Ok(record) = serde_json::to_value(&workspace) {
                                payload["workspace"] = record;
                            }
                            reply(sink, true, payload, None).await
                        }
                        None => reply(sink, false, Value::Null, Some("catalog_unavailable")).await,
                    }
                }
                Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
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

    /// Forking a conversation is the phone's copy of the Desktop's own
    /// `fork_thread`. The parent is addressed by its Agent session id (like the
    /// other session-scoped mutations), the point by the persisted user entry,
    /// and the command's own `id` doubles as the idempotent request identity.
    /// A successful fork answers the new thread + session so the phone can open
    /// it, and the child carries the parent lineage the session list needs.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // mock_agent_lock serializes this family
    async fn fork_session_creates_and_reports_the_child_conversation() {
        let _lock = crate::remote::test_support::mock_agent_lock();
        let _home = crate::remote::test_support::HomeGuard::new("catalog-fork");
        crate::remote::test_support::init_store();
        let agent = crate::remote::test_support::ensure_mock_agent();
        agent.clear_scripts();
        crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("Parent".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some("sess-fork-parent".to_string()),
        })
        .expect("seed the parent conversation");
        agent.script_for(
            "fork",
            "sess-fork-parent",
            true,
            json!({ "sessionId": "sess-fork-child" }),
            "",
        );
        agent.set_session_entries(
            "sess-fork-child",
            json!({ "entries": [
                {
                    "id": "f1", "kind": "user", "role": "user", "createdAtMs": 1000,
                    "runId": "fork-run-1",
                    "blocks": [{ "kind": "text", "text": "question" }],
                },
                {
                    "id": "f2", "kind": "assistant", "role": "assistant", "createdAtMs": 1001,
                    "runId": "fork-run-1",
                    "blocks": [{ "kind": "text", "text": "answer" }],
                },
            ] }),
        );

        let sink = crate::remote::test_support::RecordingSink::default();
        let cmd = IncomingCmd {
            cmd_type: "fork_session".into(),
            id: "fork-request-1".into(),
            session_id: "sess-fork-parent".into(),
            source_entry_id: "f1".into(),
            ..Default::default()
        };
        execute(&cmd, &sink).await;

        let (success, data, error) = sink.last();
        assert!(success, "fork must succeed: {error:?}");
        assert_eq!(data["sessionId"], json!("sess-fork-child"));
        let child_id = data["threadId"].as_str().expect("forked thread id");
        let child = crate::store::get_thread(child_id)
            .expect("get forked thread")
            .expect("forked thread exists");
        assert_eq!(child.agent_session_id.as_deref(), Some("sess-fork-child"));
        assert_eq!(
            child.parent_session_id.as_deref(),
            Some("sess-fork-parent"),
            "the session list needs the fork lineage"
        );
        // The Desktop forked the exact entry the phone pointed at, through the
        // parent session; the Agent (not the Desktop) resolves the turn.
        assert!(agent.served("fork", "sess-fork-parent"));
        let fork_request = agent
            .last_full_request("fork")
            .expect("fork request recorded");
        assert_eq!(fork_request.entry_id, "f1");
        assert_eq!(fork_request.client_request_id, "fork-request-1");
        assert_eq!(fork_request.mode, "through_turn");

        agent.clear_scripts();
    }

    /// Argument validation happens before anything is read or written. An empty
    /// source entry or missing request identity is the phone's own protocol
    /// error, not a store failure, and must not reach the Agent or the store.
    #[tokio::test]
    async fn fork_session_rejects_missing_arguments_before_any_work() {
        let _home = crate::remote::test_support::HomeGuard::new("catalog-fork-args");
        crate::remote::test_support::init_store();
        let sink = crate::remote::test_support::RecordingSink::default();

        let mut cmd = IncomingCmd {
            cmd_type: "fork_session".into(),
            id: "request".into(),
            session_id: "sess".into(),
            ..Default::default()
        };
        execute(&cmd, &sink).await;
        let (success, _data, error) = sink.last();
        assert!(!success);
        assert!(
            error
                .as_deref()
                .unwrap_or_default()
                .contains("source entry"),
            "got: {error:?}"
        );

        cmd.source_entry_id = "entry".into();
        cmd.id = String::new();
        execute(&cmd, &sink).await;
        let (success, _data, error) = sink.last();
        assert!(!success);
        assert!(
            error
                .as_deref()
                .unwrap_or_default()
                .contains("request identity"),
            "got: {error:?}"
        );
    }

    /// A session id the Desktop's store does not own is a real error, not an
    /// implicitly-created empty conversation: forking must target the parent
    /// conversation the phone is actually reading.
    #[tokio::test]
    async fn fork_session_refuses_an_unknown_parent_session() {
        let _home = crate::remote::test_support::HomeGuard::new("catalog-fork-ghost");
        crate::remote::test_support::init_store();
        let sink = crate::remote::test_support::RecordingSink::default();
        let cmd = IncomingCmd {
            cmd_type: "fork_session".into(),
            id: "request".into(),
            session_id: "no-such-session".into(),
            source_entry_id: "entry".into(),
            ..Default::default()
        };
        execute(&cmd, &sink).await;
        let (success, _data, error) = sink.last();
        assert!(!success);
        assert!(
            error
                .as_deref()
                .unwrap_or_default()
                .contains("could not be loaded"),
            "got: {error:?}"
        );
    }

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

    /// The phone registers an existing desktop directory as a workspace — the
    /// same store write as the Desktop's own dialog, with the same
    /// directory-must-exist rule. The reply carries the created row *and* the
    /// fresh catalogue, so the phone can select the new workspace without
    /// re-deriving its id from the path it typed.
    #[tokio::test]
    async fn create_workspace_registers_a_directory_and_answers_the_catalogue() {
        let _home = crate::remote::test_support::HomeGuard::new("catalog-create-workspace");
        crate::remote::test_support::init_store();
        let dir = std::env::temp_dir().join(format!("futureos-catalog-ws-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("make the host directory");

        let sink = crate::remote::test_support::RecordingSink::default();
        let cmd = IncomingCmd {
            cmd_type: "create_workspace".into(),
            path: dir.display().to_string(),
            name: "Handpicked".into(),
            ..Default::default()
        };
        execute(&cmd, &sink).await;

        let (success, data, error) = sink.last();
        assert!(success, "create must succeed: {error:?}");
        assert_eq!(data["workspace"]["name"], json!("Handpicked"));
        assert_eq!(data["workspaces"].as_array().map(Vec::len), Some(1));
        let created = data["workspace"]["id"].as_str().expect("created id");
        assert!(
            data["workspaces"]
                .as_array()
                .is_some_and(|list| list.iter().any(|w| w["id"] == json!(created))),
            "the answered catalogue contains the created row"
        );
        // The stored spelling is the directory's canonical path.
        let canonical = std::fs::canonicalize(&dir)
            .expect("canonicalize the hosted directory")
            .display()
            .to_string();
        assert_eq!(data["workspace"]["path"], json!(canonical));
        assert_eq!(crate::store::list_workspaces().expect("list").len(), 1);

        // A second create for the same directory reopens the existing row: the
        // phone retrying a lost reply must not duplicate the workspace.
        let sink = crate::remote::test_support::RecordingSink::default();
        execute(&cmd, &sink).await;
        let (success, data, error) = sink.last();
        assert!(success, "re-create must succeed: {error:?}");
        assert_eq!(data["workspace"]["id"], json!(created));
        assert_eq!(crate::store::list_workspaces().expect("list").len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A missing path is the phone's own protocol error and must not reach the
    /// store; the store cannot register the empty string as a directory.
    #[tokio::test]
    async fn create_workspace_rejects_an_empty_path_before_any_work() {
        let _home = crate::remote::test_support::HomeGuard::new("catalog-create-workspace-args");
        crate::remote::test_support::init_store();
        let sink = crate::remote::test_support::RecordingSink::default();
        let cmd = IncomingCmd {
            cmd_type: "create_workspace".into(),
            path: "   ".into(),
            ..Default::default()
        };
        execute(&cmd, &sink).await;
        let (success, _data, error) = sink.last();
        assert!(!success);
        assert_eq!(error.as_deref(), Some("missing path"));
        assert!(crate::store::list_workspaces().expect("list").is_empty());
    }

    /// A path that is not an existing directory is refused with the store's own
    /// reason — the phone shows it, and no workspace row is created for a
    /// directory the desktop cannot use.
    #[tokio::test]
    async fn create_workspace_refuses_a_directory_that_does_not_exist() {
        let _home = crate::remote::test_support::HomeGuard::new("catalog-create-workspace-ghost");
        crate::remote::test_support::init_store();
        let missing = std::env::temp_dir().join(format!(
            "futureos-catalog-missing-{}-{}",
            std::process::id(),
            "nope"
        ));
        let sink = crate::remote::test_support::RecordingSink::default();
        let cmd = IncomingCmd {
            cmd_type: "create_workspace".into(),
            path: missing.display().to_string(),
            ..Default::default()
        };
        execute(&cmd, &sink).await;
        let (success, _data, error) = sink.last();
        assert!(!success, "a missing directory must not be registered");
        assert!(
            error
                .as_deref()
                .unwrap_or_default()
                .contains("does not exist"),
            "the store's reason is surfaced: {error:?}"
        );
        assert!(crate::store::list_workspaces().expect("list").is_empty());
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
