use std::sync::Arc;

use super::{
    generate_session_html, get_state_internal, AppState, ApprovalDecision, ApprovalDecisionStatus,
    RpcCommand, RpcResponse, ServerSession, SseBroadcaster, SseEvent,
};

/// Session write lock. parking_lot locks have no poisoning, so this is a
/// plain `.write()` — the macro remains so ~100 call sites stay uniform (and
/// the `$id` is kept for symmetry with the pre-parking_lot error path).
macro_rules! wlock {
    ($session:expr, $id:expr) => {
        $session.write()
    };
}
/// Session read lock — see `wlock!`.
macro_rules! rlock {
    ($session:expr, $id:expr) => {
        $session.read()
    };
}

/// Serialized-size budget for one paged `get_events_since` response. Every
/// event crosses the wire about three times (JSON `data` dual-write, typed
/// `ReplayEvent.data`, typed `EventPayload`), so this much journal-serialized
/// content stays well under the 32 MiB gRPC message cap.
const EVENTS_PAGE_BYTE_BUDGET: usize = 8 * 1024 * 1024;

/// Per-event wire size beyond the `data` payload (type, run/event ids,
/// timestamp, idx…), approximating the journal line.
const EVENT_WIRE_OVERHEAD: usize = 320;

/// Cut `events` to one page for a paging caller (`max_events > 0`): at most
/// `max_events` entries, and at most [`EVENTS_PAGE_BYTE_BUDGET`] of estimated
/// serialized size, whichever comes first. The first event always goes out —
/// even when it alone exceeds the budget — so the caller's cursor always
/// advances. Returns the page plus whether a tail remains. `max_events <= 0`
/// is the legacy unlimited behavior: no cut, `has_more = false`.
fn page_events_tail(events: Vec<SseEvent>, max_events: i64) -> (Vec<SseEvent>, bool) {
    if max_events <= 0 || events.is_empty() {
        return (events, false);
    }
    let count_cap = usize::try_from(max_events).unwrap_or(usize::MAX);
    let mut bytes = 0usize;
    let mut cut = 0usize;
    for event in &events {
        if cut >= count_cap {
            break;
        }
        let size = event.data.len() + EVENT_WIRE_OVERHEAD;
        if cut > 0 && bytes + size > EVENTS_PAGE_BYTE_BUDGET {
            break;
        }
        bytes += size;
        cut += 1;
    }
    let has_more = cut < events.len();
    let mut page = events;
    page.truncate(cut);
    (page, has_more)
}

/// Base directory for HTML exports. Always `/tmp` in production; overridable
/// in tests (the setter is `cfg(test)`-only) so the write-failure arm can be
/// reached deterministically.
static EXPORT_DIR_OVERRIDE: parking_lot::Mutex<Option<std::path::PathBuf>> =
    parking_lot::Mutex::new(None);

fn export_output_path(session_id: &str) -> std::path::PathBuf {
    let base = EXPORT_DIR_OVERRIDE
        .lock()
        .clone()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    base.join(format!(
        "future_agent_export_{}_{}.html",
        session_id,
        chrono::Local::now().format("%Y%m%d%H%M%S")
    ))
}

/// RAII guard for the test-only export-dir override. Holds the override for
/// its lifetime and restores `/tmp` on drop, so a panic can't leak a bad dir
/// into the parallel success-path export test.
#[cfg(test)]
struct ExportDirGuard;

#[cfg(test)]
impl ExportDirGuard {
    fn new(dir: std::path::PathBuf) -> Self {
        *EXPORT_DIR_OVERRIDE.lock() = Some(dir);
        ExportDirGuard
    }
}

#[cfg(test)]
impl Drop for ExportDirGuard {
    fn drop(&mut self) {
        *EXPORT_DIR_OVERRIDE.lock() = None;
    }
}

/// Serializes the two export tests, since the override is process-global.
#[cfg(test)]
static EXPORT_TEST_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

pub fn handle_command_internal(state: &AppState, cmd: RpcCommand) -> String {
    let id = &cmd.id;
    let cmd_type = &cmd.cmd_type;

    if cmd_type == "get_agent_info" {
        return get_agent_info_response(state, id);
    }
    if cmd_type == "list_models" {
        return list_models_response(
            id,
            &state.model_registry.read(),
            cmd.include_builtin_providers,
        );
    }

    // Credential refresh operates on every session, not one — handle it before
    // resolving a target session (which would needlessly create/load one).
    if cmd_type == "reload_auth" {
        // Rebuild the shared model registry FIRST so runtime-added/
        // removed providers and models.json edits become visible to every
        // session — set_model now resolves against this cache instead of
        // constructing a fresh Registry per call.
        refresh_registry_and_credentials(state);
        return RpcResponse::ok(id, "reload_auth", serde_json::json!({}));
    }

    // ── Config writes (audit item 2): the agent is the sole writer of
    // auth.json / models.json. Each mutation is applied through the agent's
    // own config layer and followed by the same registry rebuild + credential
    // refresh `reload_auth` performs, so clients no longer patch files
    // out-of-band and then paper over the stale in-memory state.
    if cmd_type == "set_auth" {
        return cmd_set_auth(state, id, &cmd);
    }
    if cmd_type == "upsert_provider" {
        return cmd_upsert_provider(state, id, &cmd);
    }
    if cmd_type == "delete_provider" {
        return cmd_delete_provider(state, id, &cmd);
    }

    // Dedicated post-login initialization: synchronously fetch the Future
    // provider's models (warming the cache), then rebuild the registry against
    // that warm cache so the very next `list_models` returns a complete list.
    // Unlike `reload_auth`, this blocks on the network fetch — it is only ever
    // called once by the GUI's onboarding init flow, never on a hot path.
    if cmd_type == "sync_future_models" {
        let synced = crate::models::sync_future_models_cache();
        *state.model_registry.write() = crate::models::Registry::new();
        state.reload_all_credentials();
        let model_count = state.model_registry.read().all_models().len();
        return RpcResponse::ok(
            id,
            "sync_future_models",
            serde_json::json!({ "synced": synced, "modelCount": model_count }),
        );
    }

    // Persist the onboarding model-picker's choice as the global default model
    // (settings.json `defaultModel`). Sessionless: it's a process-wide setting,
    // not tied to any one session. Rebuild the registry afterwards so the next
    // `list_models` reflects the new `isDefault` immediately. The value is
    // validated to exist in the catalog (the picker only offers credential-
    // reachable models, so no credential re-check here — `get_default_model_with`
    // re-validates reachability at resolution time anyway).
    if cmd_type == "set_default_model" {
        let model_id = cmd.model_id.trim().to_string();
        if model_id.is_empty() {
            return RpcResponse::build_fail(id, "set_default_model", "model_id is empty");
        }
        let exists = state
            .model_registry
            .read()
            .all_models()
            .iter()
            .any(|m| format!("{}/{}", m.provider, m.id) == model_id || m.id == model_id);
        if !exists {
            return RpcResponse::build_fail(
                id,
                "set_default_model",
                &format!("model `{model_id}` is not in the catalog"),
            );
        }
        let settings_path = std::path::PathBuf::from(crate::models::settings_path());
        let mut settings = match crate::config::load_settings(&settings_path) {
            Ok(s) => s,
            Err(e) => {
                return RpcResponse::build_fail(
                    id,
                    "set_default_model",
                    &format!("failed to load settings: {e}"),
                );
            }
        };
        settings.default_model = model_id.clone();
        if let Err(e) = settings.save(&settings_path) {
            return RpcResponse::build_fail(
                id,
                "set_default_model",
                &format!("failed to save settings: {e}"),
            );
        }
        *state.model_registry.write() = crate::models::Registry::new();
        return RpcResponse::ok(
            id,
            "set_default_model",
            serde_json::json!({ "defaultModel": model_id }),
        );
    }

    // ── Sessionless commands: dispatched WITHOUT resolving a target session.
    // Sessions are equal peers; these commands either operate on the whole
    // system (shutdown, lists), create sessions (new/switch/delete), or read
    // straight from disk (fork messages).
    match cmd_type.as_str() {
        "shutdown" => return cmd_shutdown(state, id),
        "list_sessions" => return cmd_list_sessions(state, &cmd, id),
        "list_session_ids" => return cmd_list_session_ids(state, id),
        "list_streaming_sessions" => return cmd_list_streaming_sessions(state, id),
        "new_session" => return cmd_new_session(state, &cmd, id),
        "switch_session" => return cmd_switch_session(state, &cmd, id),
        "delete_session" => return cmd_delete_session(state, &cmd, id),
        "get_fork_messages" => return cmd_get_fork_messages(state, &cmd, id),
        "get_commands" => return cmd_get_commands(id),
        // System-wide, no session needed: invalidates the skills discovery
        // cache. Must stay sessionless — the GUI/CLI call it right after
        // install/uninstall without a session_id, and a "session not found"
        // here silently left the cache stale (the installed list never
        // refreshed until restart / TTL expiry).
        "refresh_skills" => return cmd_refresh_skills(state, id),
        "set_enabled_models" => {
            // Scoped models are managed entirely by the TUI/client; the agent
            // returns all available models. Kept as a no-op for compatibility.
            return RpcResponse::ok(id, "set_enabled_models", serde_json::json!({}));
        }
        _ => {}
    }

    // ── Session-scoped commands: resolve the target session or fail.
    // No default-session fallback: an empty or unknown session_id is an
    // explicit error, never a silent redirect into another conversation.
    let Some(session) = state.get_session(&cmd.session_id) else {
        return RpcResponse::build_fail(
            id,
            cmd_type,
            "session not found — pass a valid session_id (new_session creates one)",
        );
    };

    match cmd_type.as_str() {
        "prompt" => {
            if state
                .shutting_down
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                return RpcResponse::build_fail(
                    id,
                    "prompt",
                    "agent is shutting down; no new prompts accepted",
                );
            }
            // NOTE: `session` was already resolved by the session-scoped
            // guard above — re-fetching it here can never fail, so the old
            // "session does not exist" arm was unreachable dead code.
            let mut sess = wlock!(session, id);
            let busy_policy = match crate::runtime::BusyPolicy::parse(&cmd.busy_policy) {
                Ok(policy) => policy,
                Err(error) => {
                    return RpcResponse::build_fail_code(
                        id,
                        "prompt",
                        "invalid_busy_policy",
                        &error,
                        serde_json::json!({
                            "provided": cmd.busy_policy,
                            "valid": crate::runtime::BusyPolicy::VALID_VALUES,
                        }),
                    );
                }
            };
            let client_request_id = if cmd.client_request_id.is_empty() {
                format!("request_{}", uuid::Uuid::new_v4().simple())
            } else {
                cmd.client_request_id.clone()
            };
            match sess.enqueue_prompt(
                &cmd.message,
                &cmd.images,
                &cmd.attachments,
                (!cmd.requested_run_id.is_empty()).then_some(cmd.requested_run_id.as_str()),
                &client_request_id,
                busy_policy,
            ) {
                Ok(ack) => {
                    RpcResponse::ok(id, "prompt", serde_json::to_value(ack).unwrap_or_default())
                }
                Err(error) => {
                    if let Some(queue_error) = error.downcast_ref::<crate::runtime::RunQueueError>()
                    {
                        let (code, details) = match queue_error {
                            crate::runtime::RunQueueError::Busy => (
                                "busy",
                                sess.runtime
                                    .snapshot()
                                    .map(|active| {
                                        serde_json::json!({
                                            "active_run_id": active.run_id,
                                            "active_epoch": active.epoch,
                                            "active_state": active.phase.as_str(),
                                        })
                                    })
                                    .unwrap_or(serde_json::json!({})),
                            ),
                            crate::runtime::RunQueueError::DuplicateRequestConflict(_) => (
                                "duplicate_request_conflict",
                                serde_json::json!({"client_request_id": client_request_id}),
                            ),
                            crate::runtime::RunQueueError::QueueFull { limit }
                            | crate::runtime::RunQueueError::GlobalQueueFull { limit } => {
                                ("queue_full", serde_json::json!({"limit": limit}))
                            }
                            crate::runtime::RunQueueError::RequestTooLarge { actual, limit }
                            | crate::runtime::RunQueueError::QueueBytesExceeded { actual, limit }
                            | crate::runtime::RunQueueError::GlobalQueueBytesExceeded {
                                actual,
                                limit,
                            } => (
                                "attachment_too_large",
                                serde_json::json!({"actual_bytes": actual, "limit_bytes": limit}),
                            ),
                            crate::runtime::RunQueueError::Deleting => (
                                "deleting",
                                serde_json::json!({"session_id": cmd.session_id}),
                            ),
                            crate::runtime::RunQueueError::PersistenceUnavailable(reason) => (
                                "persistence_unavailable",
                                serde_json::json!({"reason": reason}),
                            ),
                            crate::runtime::RunQueueError::AttachmentUnavailable {
                                path, ..
                            } => ("attachment_unavailable", serde_json::json!({"path": path})),
                            crate::runtime::RunQueueError::InvalidRunId(run_id) => {
                                ("invalid_run_id", serde_json::json!({"run_id": run_id}))
                            }
                            _ => ("scheduler_error", serde_json::json!({})),
                        };
                        RpcResponse::build_fail_code(
                            id,
                            "prompt",
                            code,
                            &queue_error.to_string(),
                            details,
                        )
                    } else {
                        RpcResponse::build_fail(id, "prompt", &error.to_string())
                    }
                }
            }
        }
        "cancel_queued_run" => {
            if cmd.run_id.is_empty() {
                return RpcResponse::build_fail_code(
                    id,
                    "cancel_queued_run",
                    "run_not_queued",
                    "run_id is required",
                    serde_json::json!({}),
                );
            }
            match wlock!(session, id).cancel_queued_run(
                &cmd.run_id,
                crate::runtime::QueuedCancellationReason::Cancelled,
            ) {
                Ok(cancelled) => RpcResponse::ok(
                    id,
                    "cancel_queued_run",
                    serde_json::json!({
                        "run_id": cancelled.run_id,
                        "run_sequence": cancelled.run_sequence,
                        "state": "cancelled",
                        "reason": "cancelled",
                    }),
                ),
                Err(error) => RpcResponse::build_fail_code(
                    id,
                    "cancel_queued_run",
                    "run_not_queued",
                    &error.to_string(),
                    serde_json::json!({"run_id": cmd.run_id}),
                ),
            }
        }
        "prune_run_events" => {
            if cmd.run_id.is_empty()
                || !cmd
                    .run_id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            {
                return RpcResponse::build_fail_code(
                    id,
                    "prune_run_events",
                    "invalid_run_id",
                    "run_id is required and must be a safe identifier",
                    serde_json::json!({"run_id": cmd.run_id}),
                );
            }
            let session = rlock!(session, id);
            if session
                .scheduler
                .active()
                .is_some_and(|(active, _)| active.run_id == cmd.run_id)
            {
                return RpcResponse::build_fail_code(
                    id,
                    "prune_run_events",
                    "run_active",
                    "cannot prune the journal of an active run",
                    serde_json::json!({"run_id": cmd.run_id}),
                );
            }
            let path = session
                .session_manager
                .run_data_path(&cmd.session_id)
                .join(format!("{}.jsonl", cmd.run_id));
            match std::fs::remove_file(&path) {
                Ok(()) => {
                    RpcResponse::ok(id, "prune_run_events", serde_json::json!({"pruned": true}))
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    RpcResponse::ok(id, "prune_run_events", serde_json::json!({"pruned": true}))
                }
                Err(error) => RpcResponse::build_fail_code(
                    id,
                    "prune_run_events",
                    "prune_failed",
                    &error.to_string(),
                    serde_json::json!({"run_id": cmd.run_id, "retryable": true}),
                ),
            }
        }
        "abort_session" => {
            let (active_run_id, cancelled) = {
                let mut sess = wlock!(session, id);
                let cancelled = sess
                    .cancel_all_queued_runs(crate::runtime::QueuedCancellationReason::Cancelled);
                let active_run_id = sess.runtime.snapshot().map(|active| active.run_id);
                if let Some(run_id) = active_run_id.as_deref() {
                    let _ = sess.runtime.request_abort(Some(run_id));
                }
                (active_run_id, cancelled)
            };
            RpcResponse::ok(
                id,
                "abort_session",
                serde_json::json!({
                    "active_run_id": active_run_id,
                    "queued_cancelled": cancelled.len(),
                    "state": "cancelling",
                }),
            )
        }
        "retry_persistence" => match wlock!(session, id).recover_persistence_degraded() {
            Ok(lease) => RpcResponse::ok(
                id,
                "retry_persistence",
                serde_json::json!({
                    "run_id": lease.run_id,
                    "state": "interrupted",
                    "recovered": true,
                }),
            ),
            Err(error) => RpcResponse::build_fail_code(
                id,
                "retry_persistence",
                "persistence_recovery_failed",
                &error.to_string(),
                serde_json::json!({"retryable": true}),
            ),
        },
        "abort" => {
            // abort() only needs &self — take a read lock so a concurrent
            // reader (get_state polling) can never make the abort a no-op,
            // which a failed try_write() silently did.
            let abort_result = {
                let sess = rlock!(session, id);
                sess.abort_run((!cmd.run_id.is_empty()).then_some(cmd.run_id.as_str()))
                    .map(|()| sess.session_id.clone())
            };
            let session_id = match abort_result {
                Ok(session_id) => session_id,
                Err(error) => return RpcResponse::build_fail(id, "abort", &error.to_string()),
            };
            state
                .approval_gate
                .cancel_session(&session_id, "Cancelled because the run was terminated.");
            RpcResponse::ok(id, "abort", serde_json::json!({"run_id": cmd.run_id}))
        }
        "approval_decision" => {
            let (approved, status) = match cmd.mode.as_str() {
                "approved" => (true, ApprovalDecisionStatus::Approved),
                "rejected" => (false, ApprovalDecisionStatus::Rejected),
                "cancelled" => (false, ApprovalDecisionStatus::Cancelled),
                _ => {
                    return RpcResponse::build_fail(
                        id,
                        "approval_decision",
                        "mode must be approved, rejected, or cancelled",
                    );
                }
            };
            match state.approval_gate.decide(
                &cmd.entry_id,
                &cmd.session_id,
                ApprovalDecision {
                    approved,
                    note: cmd.message.clone(),
                    status,
                },
            ) {
                Ok(()) => RpcResponse::ok(
                    id,
                    "approval_decision",
                    serde_json::json!({"approvalRequestId": cmd.entry_id, "status": cmd.mode}),
                ),
                Err(error) => RpcResponse::build_fail(id, "approval_decision", &error),
            }
        }
        // NOTE: `new_session` is intentionally NOT matched here — it is handled
        // in the sessionless branch above (a new session has no existing session
        // to resolve). An arm here would be unreachable dead code.
        "get_state" => {
            // The session-scoped guard above already resolved `session`, so
            // get_state_internal's only None path (unknown session) is
            // unreachable here.
            let state_val = get_state_internal(
                state,
                &cmd.session_id,
                (!cmd.run_id.is_empty()).then_some(cmd.run_id.as_str()),
            )
            .expect("session-scoped guard guarantees a live session");
            RpcResponse::ok(id, "get_state", state_val)
        }
        "get_messages" => {
            let msgs = rlock!(session, id).get_messages();
            RpcResponse::ok(id, "get_messages", serde_json::json!({"messages": msgs}))
        }
        "get_events_since" => {
            // P1: backfill current-run events with idx > since_idx (Bridge reconnect).
            let replay = {
                let sess = rlock!(session, id);
                sess.broadcaster.events_since(&cmd.run_id, cmd.since_idx)
            };
            let (run_id, events, _min_idx, projection) = match replay {
                Ok(replay) => replay,
                Err(error) => {
                    return RpcResponse::build_fail(id, "get_events_since", &error.to_string());
                }
            };
            // A cursor older than the replay ring returns a complete compressed
            // projection instead of a knowingly incomplete event tail.
            let truncated = projection.is_some();
            // Paging (proto max_events): a long run's journal far exceeds the
            // gRPC message cap when returned whole, so a paging caller gets the
            // tail cut to its page size (bounded by a serialized-size budget)
            // and re-requests from the last idx while has_more is set.
            let (events, has_more) = page_events_tail(events, cmd.max_events);
            // Typed payload (audit item 1): ReplayEventPayload / EventsSincePayload.
            let events = events
                .iter()
                .map(crate::rpc::replay_event_payload)
                .collect::<Vec<_>>();
            let projection = projection.map(|snapshot| crate::rpc::payloads::ProjectionPayload {
                run_id: snapshot.run_id,
                cursor: snapshot.cursor,
                events: snapshot
                    .events
                    .iter()
                    .map(crate::rpc::replay_event_payload)
                    .collect(),
            });
            let payload = crate::rpc::payloads::EventsSincePayload {
                run_id,
                events,
                truncated,
                projection,
                has_more,
            };
            RpcResponse::ok(
                id,
                "get_events_since",
                serde_json::to_value(payload).unwrap_or_default(),
            )
        }
        "get_session_events_since" => {
            let replay = rlock!(session, id)
                .broadcaster
                .session_events_since(cmd.since_idx);
            match replay {
                Ok(events) => RpcResponse::ok(
                    id,
                    "get_session_events_since",
                    serde_json::json!({
                        "events": events.into_iter().map(|event| serde_json::json!({
                            "type": event.event_type,
                            "data": event.data,
                            "sessionId": event.session_id,
                            "sessionIdx": event.session_idx,
                            "eventId": event.event_id,
                            "timestamp": event.timestamp,
                        })).collect::<Vec<_>>()
                    }),
                ),
                Err(error) => {
                    RpcResponse::build_fail(id, "get_session_events_since", &error.to_string())
                }
            }
        }
        "set_model" => {
            let (result, model_id) = {
                let mut sess = wlock!(session, id);
                let model_id = cmd.model_id.clone();
                (sess.set_model(&model_id), model_id)
            };
            match result {
                Ok(()) => {
                    {
                        let sess = rlock!(session, id);
                        sess.broadcaster.broadcast(SseEvent::new(
                            "model_changed",
                            serde_json::json!({"model": model_id}),
                        ));
                    }
                    RpcResponse::ok(id, "set_model", serde_json::json!({"model": model_id}))
                }
                Err(e) => RpcResponse::build_fail(id, "set_model", &e.to_string()),
            }
        }
        "set_thinking_level" => {
            let level = cmd.level.clone();
            wlock!(session, id).set_thinking_level(&level);
            let sess = rlock!(session, id);
            sess.broadcaster.broadcast(SseEvent::new(
                "thinking_level_changed",
                serde_json::json!({"level": level}),
            ));
            RpcResponse::ok(id, "set_thinking_level", serde_json::json!({}))
        }
        "compact" => {
            // compact() never returns Err (its body has no error path), so
            // the old Err arm was unreachable.
            let result = wlock!(session, id)
                .compact(&cmd.custom_instructions)
                .expect("compact never fails");
            RpcResponse::ok(id, "compact", result)
        }
        "set_auto_compaction" => {
            let enabled = cmd.enabled;
            wlock!(session, id).set_auto_compaction(enabled);
            let sess = rlock!(session, id);
            sess.broadcaster.broadcast(SseEvent::new(
                "auto_compaction_changed",
                serde_json::json!({"enabled": enabled}),
            ));
            RpcResponse::ok(id, "set_auto_compaction", serde_json::json!({}))
        }
        "set_auto_retry" => {
            wlock!(session, id).set_auto_retry(cmd.enabled);
            RpcResponse::ok(id, "set_auto_retry", serde_json::json!({}))
        }
        "set_system_prompt" => {
            session.write().set_system_prompt(&cmd.system_prompt);
            RpcResponse::ok(id, "set_system_prompt", serde_json::json!({}))
        }
        "set_tools" => {
            let tools = cmd.tools.clone();
            wlock!(session, id).set_tools(&tools);
            let sess = rlock!(session, id);
            sess.broadcaster.broadcast(SseEvent::new(
                "tools_changed",
                serde_json::json!({"tools": tools}),
            ));
            RpcResponse::ok(id, "set_tools", serde_json::json!({"tools": tools}))
        }
        "disable_tools" => {
            wlock!(session, id).disable_tools();
            let sess = rlock!(session, id);
            sess.broadcaster.broadcast(SseEvent::new(
                "tools_changed",
                serde_json::json!({"tools": serde_json::Value::Array(vec![])}),
            ));
            RpcResponse::ok(id, "disable_tools", serde_json::json!({}))
        }
        "disable_builtin_tools" => {
            wlock!(session, id).disable_builtin_tools();
            RpcResponse::ok(id, "disable_builtin_tools", serde_json::json!({}))
        }
        "append_system_prompt" => {
            session.write().append_system_prompt(&cmd.system_prompt);
            RpcResponse::ok(id, "append_system_prompt", serde_json::json!({}))
        }
        "steer" => {
            session.write().steer(&cmd.system_prompt);
            RpcResponse::ok(id, "steer", serde_json::json!({}))
        }
        "set_ephemeral" => {
            wlock!(session, id).set_ephemeral(cmd.ephemeral);
            RpcResponse::ok(
                id,
                "set_ephemeral",
                serde_json::json!({"ephemeral": cmd.ephemeral}),
            )
        }
        "shell" => {
            let result = wlock!(session, id).execute_shell(&cmd.command);
            match result {
                Ok(r) => RpcResponse::ok(id, "shell", r),
                Err(e) => RpcResponse::build_fail(id, "shell", &e.to_string()),
            }
        }
        "get_session_stats" => {
            let stats = rlock!(session, id).get_session_stats();
            RpcResponse::ok(id, "get_session_stats", stats)
        }
        "get_runtime_metrics" => {
            let metrics = rlock!(session, id).get_runtime_metrics();
            RpcResponse::ok(id, "get_runtime_metrics", metrics)
        }
        "fork" => cmd_fork(state, &session, &cmd, id),
        "get_session_entries" => {
            // `session` was already resolved by the session-scoped guard above,
            // so the old re-lookup and its "unknown id -> empty entries" arm
            // were unreachable dead code (the guard returns "session not found"
            // for unrecognised ids instead).
            cmd_get_session_entries(&session, id)
        }
        "get_last_assistant_text" => {
            let text = rlock!(session, id).get_last_assistant_text();
            RpcResponse::ok(
                id,
                "get_last_assistant_text",
                serde_json::json!({"text": if text.is_empty() { None } else { Some(text) }}),
            )
        }
        "set_session_name" => {
            let (session_manager, session_id, persistence) = {
                let mut sess = wlock!(session, id);
                sess.set_session_name(&cmd.name);
                (
                    sess.session_manager.clone(),
                    sess.session_id.clone(),
                    sess.persistence.clone(),
                )
            };
            // Update session_info in the same order as run persistence.
            if session_manager.find(&session_id).is_some() {
                if let Err(error) = persistence
                    .update_info("session_name", serde_json::Value::String(cmd.name.clone()))
                {
                    tracing::error!("Failed to persist session name: {error:#}");
                }
            }
            let broadcaster = {
                let sess = rlock!(session, id);
                sess.broadcaster.clone()
            };
            broadcaster.broadcast(SseEvent::new(
                "session_name_changed",
                serde_json::json!({"name": cmd.name}),
            ));
            RpcResponse::ok(id, "set_session_name", serde_json::json!({}))
        }
        "abort_retry" => {
            rlock!(session, id).abort();
            RpcResponse::ok(id, "abort_retry", serde_json::json!({}))
        }
        "cycle_model" => {
            // Cycle to next available model.  Scoping is client-side (TUI/GUI).
            // Use the cached registry — Registry::new() re-parses the 1.9 MB
            // catalog AND may do blocking network I/O (future provider
            // refresh) on every call.
            let auth = crate::AuthStore::load();
            let models: Vec<String> = state
                .model_registry
                .read()
                .all_models()
                .into_iter()
                .filter(|m| !m.api_key.is_empty() || auth.get(&m.provider).is_some())
                .map(|m| format!("{}/{}", m.provider, m.id))
                .collect();

            if models.is_empty() {
                return RpcResponse::ok(
                    id,
                    "cycle_model",
                    serde_json::json!({"model": "", "thinkingLevel": ""}),
                );
            }

            let current = rlock!(session, id).model.clone();
            let idx = models.iter().position(|m| m == &current).unwrap_or(0);
            let next_idx = (idx + 1) % models.len();
            let next_model = &models[next_idx];

            // Use set_model to update session, agent_loop, compat, and endpoint
            if let Err(e) = wlock!(session, id).set_model(next_model) {
                return RpcResponse::build_fail(id, "cycle_model", &e.to_string());
            }
            let sess = rlock!(session, id);
            sess.broadcaster.broadcast(SseEvent::new(
                "model_changed",
                serde_json::json!({"model": next_model}),
            ));

            RpcResponse::ok(
                id,
                "cycle_model",
                serde_json::json!({
                    "model": next_model,
                    "thinkingLevel": rlock!(session, id).thinking_level.clone(),
                    "isScoped": false
                }),
            )
        }
        "cycle_thinking_level" => {
            // Cycle thinking level: off -> minimal -> low -> medium -> high -> xhigh -> off
            let levels = ["off", "minimal", "low", "medium", "high", "xhigh"];
            let current = rlock!(session, id).thinking_level.clone();
            let idx = levels.iter().position(|l| *l == current).unwrap_or(0);
            let next_idx = (idx + 1) % levels.len();
            let next_level = levels[next_idx];

            // Update session thinking level and propagate to provider
            wlock!(session, id).set_thinking_level(next_level);
            let sess = rlock!(session, id);
            sess.broadcaster.broadcast(SseEvent::new(
                "thinking_level_changed",
                serde_json::json!({"level": next_level}),
            ));

            RpcResponse::ok(
                id,
                "cycle_thinking_level",
                serde_json::json!({"level": next_level}),
            )
        }
        "clone" => cmd_clone(state, &session, id),
        "export_html" => {
            // Export session to HTML file
            let sess = rlock!(session, id);
            let session_id = sess.session_id();
            let model = sess.model.clone();
            let cwd = sess.cwd.clone();
            let messages = sess.get_messages();
            drop(sess);

            // Generate HTML
            let html = generate_session_html(&session_id, &model, &cwd, &messages);

            // Write to a unique temp file to avoid clobbering concurrent exports.
            let output_path = export_output_path(&session_id);
            let output_path_str = output_path.to_string_lossy().to_string();
            if let Err(e) = std::fs::write(&output_path, html) {
                return RpcResponse::build_fail(
                    id,
                    "export_html",
                    &format!("failed to write file: {}", e),
                );
            }

            RpcResponse::ok(
                id,
                "export_html",
                serde_json::json!({"path": output_path_str}),
            )
        }
        "reload_config" => cmd_reload_config(state, &session, id),
        "set_cwd" => {
            // Trim trailing whitespace / separators so the saved cwd is
            // always a clean directory path — "project/ " produces a
            // phantom workspace name (" ") on import.
            let cwd: String = cmd.cwd.trim().trim_end_matches(['/', '\\']).to_string();
            let (session_manager, session_id, persistence) = {
                let mut sess = wlock!(session, id);
                sess.set_cwd(&cwd);
                (
                    sess.session_manager.clone(),
                    sess.session_id.clone(),
                    sess.persistence.clone(),
                )
            };
            // Persist to session JSONL so the cwd survives restarts.
            if session_manager.find(&session_id).is_some() {
                if let Err(error) =
                    persistence.update_info("cwd", serde_json::Value::String(cwd.clone()))
                {
                    tracing::error!("Failed to persist cwd: {error:#}");
                }
            }
            let broadcaster = {
                let sess = rlock!(session, id);
                sess.broadcaster.clone()
            };
            broadcaster.broadcast(SseEvent::new(
                "cwd_changed",
                serde_json::json!({"cwd": cwd}),
            ));
            RpcResponse::ok(id, "set_cwd", serde_json::json!({"cwd": cwd}))
        }
        "add_session_rule" => {
            // Same-run "allow in this workspace/chat": message = path glob,
            // mode = access ("read"|"write"). The GUI calls this alongside
            // writing the rule file so the rule takes effect this run too.
            session.read().add_session_rule(&cmd.message, &cmd.mode);
            RpcResponse::ok(id, "add_session_rule", serde_json::json!({}))
        }
        "set_sandbox_policy" => {
            let Some(policy) = cmd.sandbox_policy else {
                return RpcResponse::build_fail(
                    id,
                    "set_sandbox_policy",
                    "missing sandbox_policy payload",
                );
            };
            let summary = serde_json::json!({
                "tier": policy.tier.as_str(),
                "sandboxAvailable": crate::sandbox::platform_sandbox_available(),
            });
            let tier = policy.tier.as_str().to_string();
            wlock!(session, id).set_sandbox_policy(policy);
            let sess = rlock!(session, id);
            sess.broadcaster.broadcast(SseEvent::new(
                "sandbox_policy_changed",
                serde_json::json!({"tier": tier}),
            ));
            RpcResponse::ok(id, "set_sandbox_policy", summary)
        }
        "set_permission_level" => {
            let valid = ["all", "workspace", "none"];
            if !valid.contains(&cmd.level.as_str()) {
                return RpcResponse::build_fail(
                    id,
                    "set_permission_level",
                    &format!("invalid level: {}. valid: all, workspace, none", cmd.level),
                );
            }
            wlock!(session, id).set_permission_level(&cmd.level);
            let sess = rlock!(session, id);
            sess.broadcaster.broadcast(SseEvent::new(
                "permission_level_changed",
                serde_json::json!({"level": cmd.level}),
            ));
            RpcResponse::ok(
                id,
                "set_permission_level",
                serde_json::json!({"permissionLevel": cmd.level}),
            )
        }
        _ => RpcResponse::build_fail(id, cmd_type, &format!("unknown command: {}", cmd_type)),
    }
}

fn get_agent_info_response(state: &AppState, id: &str) -> String {
    let skills_count =
        crate::skills::discover_skills_cached(&crate::skills::global_skill_dirs()).len();
    RpcResponse::ok(
        id,
        "get_agent_info",
        serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "agentInstanceId": state.agent_instance_id,
            "skillsCount": skills_count,
        }),
    )
}

fn list_models_response(
    id: &str,
    registry: &crate::models::Registry,
    include_builtin_providers: bool,
) -> String {
    let auth = crate::AuthStore::load();

    // Always return all available models.  Scoping / defaults are client-side.
    let mut models: Vec<crate::models::Model> = registry
        .all_models()
        .into_iter()
        .filter(|model| !model.api_key.is_empty() || auth.get(&model.provider).is_some())
        .filter(|model| model.output.iter().any(|o| o == "text"))
        .collect();

    models.sort_by(|left, right| {
        left.provider
            .cmp(&right.provider)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.id.cmp(&right.id))
    });
    models.dedup_by(|left, right| left.id == right.id && left.provider == right.provider);

    // Use the same default-model resolution as cmd_new_session so the list
    // and actual session creation agree on which model is the default.
    let effective_default = crate::models::get_default_model_with(registry)
        .and_then(|full| full.rsplit_once('/').map(|(_, id)| id.to_string()))
        .or_else(|| models.first().map(|m| m.id.clone()))
        .unwrap_or_default();

    let payload_models: Vec<serde_json::Value> = models
        .into_iter()
        .map(|model| {
            let id = model.id;
            let label = if model.name.is_empty() {
                id.clone()
            } else {
                model.name.clone()
            };
            let thinking_level = if model.reasoning { "high" } else { "off" };
            serde_json::json!({
                "id": id.clone(),
                "label": label,
                "provider": model.provider.clone(),
                "supportsImages": model.input.iter().any(|input| input == "image"),
                "thinkingLevel": thinking_level.to_string(),
                "contextWindow": model.context_window,
                "isDefault": id == effective_default,
                "description": model.description,
                "descriptionEn": model.description_en,
                "recommended": model.recommended,
            })
        })
        .collect();

    let mut payload = serde_json::json!({
        "models": payload_models,
        "defaultModel": effective_default,
        "isScoped": false,
    });
    if include_builtin_providers {
        // Catalog summaries so clients (GUI Providers page) can fetch the
        // built-in catalog at runtime instead of compiling agent source in.
        payload["builtinProviders"] = serde_json::to_value(registry.builtin_provider_summaries())
            .unwrap_or_else(|_| serde_json::json!({}));
    }
    RpcResponse::ok(id, "list_models", payload)
}

/// Rebuild the shared model registry so provider/models.json changes become
/// visible to every session, then refresh each live session's cached
/// credentials. Shared by `reload_auth` and the config-write commands, which
/// apply it inline so clients need no follow-up refresh round-trip.
fn refresh_registry_and_credentials(state: &AppState) {
    *state.model_registry.write() = crate::models::Registry::new();
    state.reload_all_credentials();
}

/// Apply one auth.json mutation and refresh live state (see dispatch comment).
fn cmd_set_auth(state: &AppState, id: &str, cmd: &RpcCommand) -> String {
    let Some(mutation) = cmd.auth_update.as_ref() else {
        return RpcResponse::build_fail(id, "set_auth", "missing auth_update payload");
    };
    if mutation.provider.trim().is_empty() {
        return RpcResponse::build_fail(id, "set_auth", "auth_update.provider is empty");
    }
    let carries_change = mutation.key.is_some()
        || mutation.base_url.is_some()
        || mutation.clear_key
        || mutation.clear_base_url
        || mutation.remove_entry
        || mutation.remove_platform_base_url;
    if !carries_change {
        return RpcResponse::build_fail(id, "set_auth", "auth_update carries no change");
    }
    if let Err(error) = crate::config::providers::mutate_auth(mutation) {
        return RpcResponse::build_fail(id, "set_auth", &error);
    }
    refresh_registry_and_credentials(state);
    RpcResponse::ok(
        id,
        "set_auth",
        serde_json::json!({ "provider": mutation.provider }),
    )
}

/// Create/update a models.json provider (plus optional auth.json key) and
/// refresh live state (see dispatch comment).
fn cmd_upsert_provider(state: &AppState, id: &str, cmd: &RpcCommand) -> String {
    let Some(spec) = cmd.provider_config.as_ref() else {
        return RpcResponse::build_fail(id, "upsert_provider", "missing provider_config payload");
    };
    if spec.id.trim().is_empty() {
        return RpcResponse::build_fail(id, "upsert_provider", "provider_config.id is empty");
    }
    let carries_change = spec.name.is_some()
        || spec.api.is_some()
        || spec.base_url.is_some()
        || spec.clear_base_url
        || !spec.models.is_empty()
        || spec.api_key.is_some();
    if !carries_change {
        return RpcResponse::build_fail(id, "upsert_provider", "provider_config carries no change");
    }
    // The agent is the authority on its own built-in catalog: reject any write
    // that would *define* a custom provider (name/api/models/key) under an id
    // that belongs to a built-in provider or the Future platform. Pure base-URL
    // overrides (no name/api/models/key) are still allowed — that is how clients
    // legitimately point a built-in provider at a different endpoint. Guarding
    // here (not only in the GUI) keeps the invariant correct no matter which
    // client issues the write and whether any client-side catalog is stale or
    // temporarily unavailable.
    let defines_custom_provider = spec.name.is_some()
        || spec.api.is_some()
        || !spec.models.is_empty()
        || spec.api_key.is_some();
    if defines_custom_provider
        && state
            .model_registry
            .read()
            .builtin_provider_ids()
            .contains(spec.id.trim())
    {
        return RpcResponse::build_fail(
            id,
            "upsert_provider",
            &format!(
                "Provider ID `{}` is reserved for a built-in provider.",
                spec.id.trim()
            ),
        );
    }
    if let Err(error) = crate::config::providers::upsert_provider(spec) {
        return RpcResponse::build_fail(id, "upsert_provider", &error);
    }
    refresh_registry_and_credentials(state);
    RpcResponse::ok(id, "upsert_provider", serde_json::json!({ "id": spec.id }))
}

/// Remove a provider's models.json entry AND auth.json entry, then refresh
/// live state (see dispatch comment).
fn cmd_delete_provider(state: &AppState, id: &str, cmd: &RpcCommand) -> String {
    let Some(spec) = cmd.provider_config.as_ref() else {
        return RpcResponse::build_fail(id, "delete_provider", "missing provider_config payload");
    };
    let provider_id = spec.id.trim();
    if provider_id.is_empty() {
        return RpcResponse::build_fail(id, "delete_provider", "provider_config.id is empty");
    }
    // The agent is the authority on its own catalog: refuse to delete a
    // built-in provider or the Future platform entry via this command. Clients
    // legitimately remove built-in overrides / the Future login through the
    // dedicated set_auth / upsert paths — a direct `delete_provider` must never
    // be able to wipe the Future sign-in credentials or a built-in's key/URL
    // override. The GUI already guards this; guarding here closes the bypass
    // for any other gRPC client.
    if state
        .model_registry
        .read()
        .builtin_provider_ids()
        .contains(provider_id)
    {
        return RpcResponse::build_fail(
            id,
            "delete_provider",
            &format!(
                "Provider ID `{provider_id}` is reserved for a built-in provider and cannot be deleted."
            ),
        );
    }
    if let Err(error) = crate::config::providers::delete_provider(provider_id) {
        return RpcResponse::build_fail(id, "delete_provider", &error);
    }
    refresh_registry_and_credentials(state);
    RpcResponse::ok(
        id,
        "delete_provider",
        serde_json::json!({ "id": provider_id }),
    )
}

fn cmd_shutdown(state: &AppState, id: &str) -> String {
    state
        .shutting_down
        .store(true, std::sync::atomic::Ordering::SeqCst);
    RpcResponse::ok(
        id,
        "shutdown",
        serde_json::json!({"shutting_down": true, "note": "Existing runs continue; new prompts are rejected."}),
    )
}

fn cmd_list_sessions(state: &AppState, cmd: &RpcCommand, id: &str) -> String {
    // Propagate enumeration errors instead of coercing them to an empty list.
    // A directory that is momentarily unreadable must NOT be reported as "the
    // agent has zero sessions" — clients that reconcile their own mirrors
    // against this list (GUI orphan cleanup) would treat every known thread as
    // deleted and hard-delete them. Failure lets callers distinguish "could not
    // enumerate" (skip / retry) from "genuinely empty".
    let summaries = match state.session_manager.list_all() {
        Ok(summaries) => summaries,
        Err(error) => {
            return RpcResponse::build_fail(
                id,
                "list_sessions",
                &format!("failed to enumerate sessions: {error}"),
            );
        }
    };
    // Scope by the caller's cwd when provided (empty = all sessions).
    let cwd_filter = cmd.cwd.trim().to_string();

    // Snapshot streaming flags of live sessions.  Collect within a single
    // outer read guard — safe because we only acquire inner read locks, and
    // ParkingLot RwLock allows concurrent reads.
    let active_flags: std::collections::HashMap<String, bool> = {
        let active = state.sessions.read();
        active
            .iter()
            .map(|(sid, sess)| {
                let streaming = sess
                    .read()
                    .is_streaming
                    .load(std::sync::atomic::Ordering::Relaxed);
                (sid.clone(), streaming)
            })
            .collect()
    };

    // Typed payload (audit item 1): canonical camelCase keys, with snake_case
    // legacy aliases alongside so pre-migration clients keep working.
    let sessions: Vec<serde_json::Value> = summaries
        .into_iter()
        .filter(|s| cwd_filter.is_empty() || s.cwd == cwd_filter)
        .map(|s| {
            let is_streaming = active_flags.get(&s.id).copied().unwrap_or(false);
            let mut value = serde_json::to_value(crate::rpc::payloads::SessionSummaryPayload {
                id: s.id,
                session_name: s.name,
                model: s.model,
                cwd: s.cwd,
                updated_at: s.updated_at.format("%Y-%m-%d %H:%M:%S").to_string(),
                parent_session_id: s.parent_session_id,
                first_message: s.first_message,
                query_count: s.query_count,
                is_streaming,
            })
            .unwrap_or_default();
            crate::rpc::payloads::inject_legacy_aliases(
                &mut value,
                &[
                    ("sessionName", "session_name"),
                    ("updatedAt", "updated_at"),
                    ("parentSessionId", "parent_session_id"),
                    ("firstMessage", "first_message"),
                    ("queryCount", "query_count"),
                    ("isStreaming", "is_streaming"),
                ],
            );
            value
        })
        .collect();
    RpcResponse::ok(
        id,
        "list_sessions",
        serde_json::json!({"sessions": sessions}),
    )
}

/// Enumeration of session ids by FILENAME ONLY — no file contents are read.
///
/// This is the reconciliation-safe variant of `list_sessions`: a session whose
/// JSONL is momentarily unreadable, truncated, or corrupt still exists on disk
/// and so is still reported as live here. Clients that reconcile their own
/// mirrors against this list (the GUI's orphan-thread cleanup) can never
/// mistake a transient read failure for a deleted session and hard-delete
/// local state. Only a genuine directory-listing error fails the command.
fn cmd_list_session_ids(state: &AppState, id: &str) -> String {
    let ids = match state.session_manager.list_ids() {
        Ok(ids) => ids,
        Err(error) => {
            return RpcResponse::build_fail(
                id,
                "list_session_ids",
                &format!("failed to enumerate session files: {error}"),
            );
        }
    };
    RpcResponse::ok(id, "list_session_ids", serde_json::json!({ "ids": ids }))
}

/// Lightweight streaming-status query: scans ONLY the in-memory session map
/// (hydrated sessions) — never touches disk and never hydrates.  A session
/// that isn't in the map can't be streaming (runs are always started through
/// a hydrated ServerSession), so this is the exact set of active runs.
fn cmd_list_streaming_sessions(state: &AppState, id: &str) -> String {
    let ids: Vec<String> = state
        .sessions
        .read()
        .iter()
        .filter(|(_, sess)| {
            sess.read()
                .is_streaming
                .load(std::sync::atomic::Ordering::Relaxed)
        })
        .map(|(sid, _)| sid.clone())
        .collect();
    RpcResponse::ok(
        id,
        "list_streaming_sessions",
        serde_json::json!({"sessionIds": ids}),
    )
}

/// Bind a client to an existing session.  Sessions are equal peers, so
/// "switching" just means resolving (and hydrating) the target — the client
/// addresses it by id from then on.
fn cmd_switch_session(state: &AppState, cmd: &RpcCommand, id: &str) -> String {
    if cmd.session_id.is_empty() {
        return RpcResponse::build_fail(
            id,
            "switch_session",
            "No session selected. Choose a session from the list to switch to.",
        );
    }
    match state.get_session(&cmd.session_id) {
        Some(_) => RpcResponse::ok(
            id,
            "switch_session",
            serde_json::json!({"cancelled": false}),
        ),
        None => RpcResponse::build_fail(
            id,
            "switch_session",
            &format!("session `{}` not found", cmd.session_id),
        ),
    }
}

fn cmd_delete_session(state: &AppState, cmd: &RpcCommand, id: &str) -> String {
    if cmd.session_id.is_empty() {
        return RpcResponse::build_fail(
            id,
            "delete_session",
            "No session selected to delete. Choose a session first.",
        );
    }
    let live = state.sessions.read().get(&cmd.session_id).cloned();
    if let Some(session) = live {
        let (active, cancelled_count) = {
            let mut session = session.write();
            session.deleting = true;
            let cancelled = session
                .cancel_all_queued_runs(crate::runtime::QueuedCancellationReason::SessionDeleted);
            let active = session.runtime.snapshot();
            if let Some(active) = &active {
                let _ = session.runtime.request_abort(Some(&active.run_id));
            }
            (active, cancelled.len())
        };
        if let Some(active) = active.filter(|active| {
            session.read().runtime.has_owned_task()
                || !matches!(
                    active.phase,
                    crate::runtime::RunPhase::CancellationStuck
                        | crate::runtime::RunPhase::PersistenceDegraded
                )
        }) {
            return RpcResponse::build_fail_code(
                id,
                "delete_session",
                "deleting",
                "session deletion is waiting for the active run to stop; retry delete_session",
                serde_json::json!({
                    "session_id": cmd.session_id,
                    "active_run_id": active.run_id,
                    "queued_cancelled": cancelled_count,
                    "retryable": true,
                }),
            );
        }
        // Hard deletion is a close-then-delete barrier. The session write lock
        // excludes concurrent metadata commands while the ordered transcript
        // writer drains; closing the event journal then fences late broadcasts
        // before either filesystem tree is removed.
        let session = session.write();
        if let Err(error) = session.persistence.close() {
            return RpcResponse::build_fail_code(
                id,
                "delete_session",
                "delete_failed",
                &format!("failed to close session persistence: {error}"),
                serde_json::json!({"session_id": cmd.session_id, "retryable": true}),
            );
        }
        session.broadcaster.close_journal();
    }

    // The in-memory session is fenced before disk removal. Keep it in the map
    // if deletion fails so a retry cannot accidentally rehydrate/accept work
    // against partially deleted state.
    if let Err(e) = state.session_manager.delete(&cmd.session_id) {
        return RpcResponse::build_fail_code(
            id,
            "delete_session",
            "delete_failed",
            &e.to_string(),
            serde_json::json!({"session_id": cmd.session_id, "retryable": true}),
        );
    }
    state.sessions.write().remove(&cmd.session_id);
    RpcResponse::ok(id, "delete_session", serde_json::json!({"deleted": true}))
}

/// Load user entries of a session from disk (fork-point picker).  Reads the
/// file directly — no in-memory session required.
fn cmd_get_fork_messages(state: &AppState, cmd: &RpcCommand, id: &str) -> String {
    let user_entries: Vec<serde_json::Value> = state
        .session_manager
        .load(&cmd.session_id)
        .map(|s| {
            s.entries
                .iter()
                .filter(|e| e.entry_type == crate::session::ENTRY_TYPE_USER)
                .map(|e| {
                    let content_text = e
                        .content
                        .as_ref()
                        .map(|c| {
                            if let Some(arr) = c.as_array() {
                                // First text block only — later text blocks are
                                // the agent-injected attachment-path list.
                                arr.iter()
                                    .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                                    .next()
                                    .unwrap_or("")
                                    .to_string()
                            } else {
                                c.as_str().unwrap_or("").to_string()
                            }
                        })
                        .unwrap_or_default();
                    serde_json::json!({
                        "id": e.id,
                        "role": e.role,
                        "content": content_text,
                        "timestamp": e.timestamp.format("%Y-%m-%d %H:%M:%S").to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    RpcResponse::ok(
        id,
        "get_fork_messages",
        serde_json::json!({"messages": user_entries}),
    )
}

fn cmd_get_commands(id: &str) -> String {
    // Return commands from skills (similar to Go's extensions + prompts)
    let skills = crate::skills::discover_skills_cached(&crate::skills::global_skill_dirs());

    let mut commands: Vec<serde_json::Value> = skills
        .into_iter()
        .map(|s| {
            serde_json::json!({
                "name": s.name,
                "description": s.description,
                "nameZh": s.name_zh,
                "descriptionZh": s.description_zh,
                "source": "skill"
            })
        })
        .collect();
    commands.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or("")
            .cmp(b["name"].as_str().unwrap_or(""))
    });

    RpcResponse::ok(
        id,
        "get_commands",
        serde_json::json!({"commands": commands}),
    )
}

fn cmd_new_session(state: &AppState, cmd: &RpcCommand, id: &str) -> String {
    // Create a new session with shared agent_loop, preserving model/thinking
    // Use TUI-provided cwd if available, otherwise default workspace.
    // Trim trailing whitespace / separators so the saved cwd doesn't
    // produce a phantom workspace name (e.g. "project/ " → name " ").
    let session_cwd = if !cmd.cwd.is_empty() {
        cmd.cwd.trim().trim_end_matches(['/', '\\']).to_string()
    } else {
        super::session::default_workspace()
    };
    // No active/default session to inherit from — everything comes from
    // AppState-level singletons and the loop template.  The fresh loop is
    // minted from the template (never used for runs), so creation succeeds
    // even while every existing session is mid-stream.
    let broadcaster = Arc::new(SseBroadcaster::new());
    let approval_gate = state.approval_gate.clone();
    // Reuse the AppState's session manager rather than minting a new one via
    // `Manager::default_for`: the session store is a single flat directory
    // (`default_session_dir` ignores its cwd argument), so both point at the
    // same place in production — and reusing keeps tests (whose AppState uses
    // an isolated temp dir) from writing into the real session store.
    let session_manager = state.session_manager.clone();
    let inherit_model = state.loop_template.model.clone();

    let fresh_loop = state.loop_template.independent_copy();

    let new_session_id = if cmd.session_id.is_empty() {
        crate::utils::generate_id()
    } else {
        cmd.session_id.clone()
    };

    // If this session ID already exists on disk (e.g. a forked session),
    // load the existing entries and restore them after creating the session.
    let existing_entries = session_manager
        .load(&new_session_id)
        .ok()
        .filter(|s| !s.entries.is_empty())
        .map(|s| (s.entries, s.model.clone()));

    let mut new_sess = ServerSession::new_with_queue_budget(
        new_session_id.clone(),
        Arc::new(tokio::sync::RwLock::new(fresh_loop)),
        session_manager.clone(),
        &session_cwd,
        broadcaster,
        approval_gate,
        state.model_registry.clone(),
        state.queue_budget.clone(),
    );
    // Resolve the default model from the cached registry (not inherited from
    // the active session) so that CLI one-shot runs always start from the
    // preferred default.  GUI/TUI explicitly set model_id on the command,
    // which overrides this below.
    let default_model = crate::models::get_default_model_with(&state.model_registry.read())
        .unwrap_or_else(|| inherit_model.clone());
    // Apply via set_model: it sets the canonical model AND rebuilds the
    // loop's provider client for that model's endpoint/key/compat.  A bare
    // `loop_.model = bare_id` leaves the provider on the template's startup
    // model, which breaks whenever the current default differs.
    if let Err(e) = new_sess.set_model(&default_model.clone()) {
        tracing::warn!("[new_session] could not sync model to fresh loop: {e}");
    }
    // Always start new sessions at the preferred thinking level.
    new_sess.thinking_level = "xhigh".to_string();

    // Apply user settings (previously applied only to the startup "default
    // session" — with sessions as equal peers, every new session gets them).
    let settings_path = std::path::PathBuf::from(crate::models::settings_path());
    if let Ok(settings) = crate::config::load_settings(&settings_path) {
        if !settings.default_permission_level.is_empty() {
            new_sess.set_permission_level(&settings.default_permission_level);
        }
        new_sess.set_auto_compaction(settings.compaction_enabled());
        new_sess.set_auto_retry(settings.retry_enabled());
    }

    // Default created_by to "tui" for sessions created without explicit
    // source info (e.g. TUI, channels); clients pass their identity via the
    // typed created_by field.
    new_sess.created_by = "tui".to_string();
    if !cmd.parent_session.is_empty() {
        new_sess.parent_session_id = cmd.parent_session.clone();
    }

    // Session provenance: typed created_by/source_meta fields first.
    if !cmd.created_by.is_empty() {
        new_sess.created_by = cmd.created_by.clone();
    }
    if !cmd.source_meta.is_empty() {
        if let Ok(meta) = serde_json::from_str::<serde_json::Value>(&cmd.source_meta) {
            new_sess.source_meta = meta;
        }
    }
    // Legacy fallback: old clients smuggle {"createdBy":...,"sourceMeta":...}
    // JSON through custom_instructions (which belongs to the compact command).
    if new_sess.created_by == "tui" && !cmd.custom_instructions.is_empty() {
        if let Ok(meta) = serde_json::from_str::<serde_json::Value>(&cmd.custom_instructions) {
            if let Some(src) = meta
                .get("createdBy")
                .or_else(|| meta.get("source"))
                .and_then(|v| v.as_str())
            {
                new_sess.created_by = src.to_string();
            }
            if new_sess.source_meta.is_null() {
                if let Some(m) = meta.get("sourceMeta").or_else(|| meta.get("meta")) {
                    new_sess.source_meta = m.clone();
                }
            }
        }
    }
    // Apply model and thinking level from the command if provided
    // (client sends these during session creation so the session
    // starts with the user's selection, without needing a separate
    // set_model/set_thinking_level RPC).
    if !cmd.model_id.is_empty() {
        new_sess.model = cmd.model_id.clone();
    }
    if !cmd.level.is_empty() {
        new_sess.thinking_level = cmd.level.clone();
    }

    // Restore entries from a pre-existing session (forked or persisted).
    if let Some((entries, disk_model)) = existing_entries {
        // Gate image re-hydration on the model that will actually run
        // (disk model wins over the command's default).
        let effective_model = if disk_model.is_empty() {
            new_sess.model.clone()
        } else {
            disk_model.clone()
        };
        let supports_images = crate::models::model_accepts_images_with(
            &state.model_registry.read(),
            &effective_model,
        );
        let mut msgs = new_sess.messages.write();
        *msgs = crate::session::entries_to_agent_messages(&entries, supports_images);
        if !disk_model.is_empty() {
            new_sess.model = disk_model.clone();
        }
    }

    // Sync the final session model into the fresh agent loop (may differ
    // from the default model set above due to cmd.model_id or disk_model
    // overrides).
    if let Err(e) = new_sess.set_model(&new_sess.model.clone()) {
        tracing::warn!("[new_session] could not sync agent loop model: {e}");
    }

    // Add to sessions map
    let new_id = state.create_session(new_sess);

    RpcResponse::ok(id, "new_session", serde_json::json!({"sessionId": new_id}))
}

fn cmd_get_session_entries(session: &Arc<parking_lot::RwLock<ServerSession>>, id: &str) -> String {
    // Return displayable entries from a session plus the session_info
    // metadata entry (model, thinking_level, session_name, cwd).
    let (session_manager, session_id) = {
        let sess = rlock!(session, id);
        (sess.session_manager.clone(), sess.session_id.clone())
    };
    let entries: Vec<serde_json::Value> = session_manager
        .load(&session_id)
        .map(|s| {
            // The authoritative metadata is the last session_info snapshot
            // (the append-only commit path appends a fresh one per run). Surface
            // it in the first session_info slot — where clients (CLI info, fork)
            // look — and drop the stale earlier ones so callers see exactly one.
            let authoritative_info = s
                .entries
                .iter()
                .rev()
                .find(|e| e.entry_type == crate::session::ENTRY_TYPE_SESSION_INFO)
                .and_then(|e| e.content.clone());
            let mut emitted_session_info = false;
            // Per-run output tokens + wall-clock duration live only in the
            // `run_terminal` marker's content JSON (the assistant entry's content
            // is a block array, so it can't carry them). Bind each marker to the
            // assistant entry that precedes it — the run's final reply — so the
            // GUI/mobile footer ("time · N tokens") survives a reload. Positional
            // binding (not run_id) because the on-disk assistant entry's meta may
            // lack run_id on the append-only fast path. Clearing the pointer after
            // a marker keeps a terminal of a reply-less run (cancel/error before
            // any assistant entry) from overwriting the previous run's stats.
            let mut last_assistant_id: Option<String> = None;
            let mut run_stats: std::collections::HashMap<String, (i64, i64)> =
                std::collections::HashMap::new();
            for marker in &s.entries {
                if marker.entry_type == crate::session::ENTRY_TYPE_ASSISTANT {
                    last_assistant_id = Some(marker.id.clone());
                } else if marker.entry_type == crate::session::ENTRY_TYPE_RUN_TERMINAL {
                    if let (Some(aid), Some(content)) =
                        (last_assistant_id.as_deref(), marker.content.as_ref())
                    {
                        let tokens = content
                            .get("run_tokens")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);
                        let duration = content
                            .get("run_duration_ms")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);
                        run_stats.insert(aid.to_string(), (tokens, duration));
                    }
                    last_assistant_id = None;
                }
            }
            s.entries
                .iter()
                .filter(|e| {
                    if !matches!(
                        e.entry_type.as_str(),
                        "user" | "assistant" | "tool" | "session_info"
                    ) {
                        return false;
                    }
                    // Keep only the first session_info slot; its content is
                    // replaced with the authoritative (last) snapshot below, and
                    // the stale later snapshots are dropped.
                    if e.entry_type == crate::session::ENTRY_TYPE_SESSION_INFO {
                        if emitted_session_info {
                            return false;
                        }
                        emitted_session_info = true;
                    }
                    true
                })
                .map(|e| {
                    let content_text = e
                        .content
                        .as_ref()
                        .map(|c| {
                            if let Some(arr) = c.as_array() {
                                let texts = arr
                                    .iter()
                                    .filter_map(|b| b.get("text").and_then(|t| t.as_str()));
                                if e.role == "user" {
                                    // A user entry's visible text is only their typed
                                    // message (the first text block). Any later text
                                    // block is agent-injected attachment context
                                    // (file paths), which must not leak into the bubble.
                                    texts.take(1).collect::<Vec<_>>().join(" ")
                                } else {
                                    texts.collect::<Vec<_>>().join(" ")
                                }
                            } else {
                                c.as_str().unwrap_or("").to_string()
                            }
                        })
                        .unwrap_or_default();
                    // Build the display content for this entry. Only include the
                    // actual visible text — no thinking or tool formatting.
                    // The forked session's messages should look identical to
                    // original GUI messages (which store thinking/tools in
                    // run events, not in the message content).
                    let full_content = if e.entry_type == "tool" {
                        // Tool entries: show the result text, or a placeholder.
                        if content_text.is_empty() {
                            String::new()
                        } else {
                            content_text
                        }
                    } else {
                        // User and assistant entries: just the text content.
                        content_text
                    };

                    // Typed payload (audit item 1): SessionEntryPayload mirrors
                    // the on-disk entry schema (snake_case keys).
                    let mut payload = crate::rpc::payloads::SessionEntryPayload {
                        id: e.id.clone(),
                        role: e.role.clone(),
                        content: serde_json::Value::String(full_content),
                        name: e.name.clone(),
                        tool_args: e.tool_args.clone(),
                        timestamp: e.timestamp.to_rfc3339(),
                        thinking: None,
                        meta: None,
                        tool_calls: None,
                        output_tokens: None,
                        duration_ms: None,
                    };
                    // Include thinking and tool_calls for the new agent-based
                    // message display (entryProjection.ts).
                    if !e.thinking.is_empty() {
                        payload.thinking = Some(e.thinking.clone());
                    }
                    // Structured per-entry metadata (e.g. user attachments with
                    // their cached thumbnails) so the GUI can rebuild attachment
                    // chips after reload — the JSONL is the only message source.
                    if let Some(ref meta) = e.meta {
                        payload.meta = Some(meta.clone());
                    }
                    if !e.tool_calls.is_empty() {
                        payload.tool_calls = serde_json::to_value(&e.tool_calls).ok();
                    }
                    // Surface this reply's output tokens + duration on the final
                    // assistant entry of each run (bound from the run_terminal
                    // marker above) so the footer can show "time · N tokens" after
                    // a reload — entriesToMessages / the mobile reducer read these
                    // top-level fields.
                    if e.entry_type == crate::session::ENTRY_TYPE_ASSISTANT {
                        if let Some((tokens, duration)) = run_stats.get(&e.id) {
                            if *tokens > 0 {
                                payload.output_tokens = Some(*tokens);
                            }
                            if *duration > 0 {
                                payload.duration_ms = Some(*duration);
                            }
                        }
                    }
                    // For session_info entries, include the original content
                    // JSON (session_name, cwd, parent_session_id, …) so callers can
                    // read fork metadata without a second RPC.
                    if e.entry_type == crate::session::ENTRY_TYPE_SESSION_INFO {
                        // Use the authoritative (last) snapshot's content so the
                        // single emitted session_info reflects current metadata,
                        // not the stale values from session creation.
                        if let Some(ref content) = authoritative_info {
                            payload.content = content.clone();
                        }
                    }
                    serde_json::to_value(&payload).unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default();
    RpcResponse::ok(
        id,
        "get_session_entries",
        serde_json::json!({"entries": entries}),
    )
}

/// Test-only hook fired inside `fork`/`clone` after the forked session is
/// built but before its model is synced into the fresh agent loop, keyed on
/// the parent session id. Lets a test force the model-sync failure warn.
#[cfg(test)]
type ModelSyncHook = Option<(String, Box<dyn Fn(&mut ServerSession) + Send>)>;

#[cfg(test)]
static MODEL_SYNC_FAIL_HOOK: parking_lot::Mutex<ModelSyncHook> = parking_lot::Mutex::new(None);

fn cmd_fork(
    state: &AppState,
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    let entry_id = &cmd.entry_id;
    if entry_id.is_empty() {
        return RpcResponse::build_fail(
            id,
            "fork",
            "No message selected to fork from. Choose a user message to fork at.",
        );
    }

    // Extract needed data from session
    let (session_manager, broadcaster, _cwd, current_session_id) = {
        let sess = rlock!(session, id);
        (
            sess.session_manager.clone(),
            sess.broadcaster.clone(),
            sess.cwd.clone(),
            sess.session_id.clone(),
        )
    };
    // The fork gets its own agent loop — sharing the parent's loop would let
    // a run in one session block (or be aborted by) the other.
    let agent_loop = Arc::new(tokio::sync::RwLock::new(
        state.loop_template.independent_copy(),
    ));

    // Resolve parent session: use cmd.parent_session if provided,
    // otherwise fork from the current session.
    let parent_id = if !cmd.parent_session.is_empty() {
        cmd.parent_session.clone()
    } else {
        current_session_id.clone()
    };

    // Get parent session from manager
    let parent = match session_manager.load(&parent_id) {
        Ok(s) => s,
        Err(_) => {
            return RpcResponse::build_fail(
                id,
                "fork",
                "Session not found on disk — it may have been deleted or moved.",
            );
        }
    };

    // Fork a new session
    let forked = crate::session::fork_session(&parent, entry_id);
    let forked_id = forked.id.clone();

    // Save the forked session
    if let Err(e) = session_manager.save(&forked) {
        return RpcResponse::build_fail(
            id,
            "fork",
            &format!("failed to save forked session: {}", e),
        );
    }

    // Add to sessions map.  Load the forked entries into
    // in-memory messages so the first prompt doesn't overwrite
    // the saved history on disk — session_prompt.rs saves
    // self.messages back to disk (via File::create), truncating
    // anything not held in memory.
    let mut new_sess = ServerSession::new_with_queue_budget(
        forked_id.clone(),
        agent_loop,
        session_manager,
        &forked.cwd,
        broadcaster,
        state.approval_gate.clone(),
        state.model_registry.clone(),
        state.queue_budget.clone(),
    );
    let supports_images =
        crate::models::model_accepts_images_with(&state.model_registry.read(), &forked.model);
    let msgs = crate::session::entries_to_agent_messages(&forked.entries, supports_images);
    *new_sess.messages.write() = msgs;
    if !forked.model.is_empty() {
        new_sess.model = forked.model.clone();
        #[cfg(test)]
        {
            let mut slot = MODEL_SYNC_FAIL_HOOK.lock();
            if matches!(slot.as_ref(), Some((sid, _)) if sid == &parent_id) {
                if let Some((_, hook)) = slot.take() {
                    hook(&mut new_sess);
                }
            }
        }
        // Sync the fork's own agent loop so the first prompt uses the
        // forked model, not whatever the template seeded.
        if let Err(e) = new_sess.set_model(&new_sess.model.clone()) {
            tracing::warn!("[fork] could not sync agent loop model: {e}");
        }
    }
    state.create_session(new_sess);

    RpcResponse::ok(id, "fork", serde_json::json!({"sessionId": forked_id}))
}

fn cmd_clone(
    state: &AppState,
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    id: &str,
) -> String {
    // Extract needed data from session
    let (session_manager, broadcaster, _cwd, session_id) = {
        let sess = rlock!(session, id);
        if sess.messages.read().is_empty() {
            return RpcResponse::build_fail(
                id,
                "clone",
                "Nothing to clone — the current session has no messages yet.",
            );
        }
        (
            sess.session_manager.clone(),
            sess.broadcaster.clone(),
            sess.cwd.clone(),
            sess.session_id.clone(),
        )
    };
    // Own agent loop for the clone (same reasoning as fork).
    let agent_loop = Arc::new(tokio::sync::RwLock::new(
        state.loop_template.independent_copy(),
    ));

    // Get parent session from manager
    let parent = match session_manager.load(&session_id) {
        Ok(s) => s,
        Err(_) => {
            return RpcResponse::build_fail(
                id,
                "clone",
                "Session not found on disk — it may have been deleted or moved.",
            );
        }
    };

    let leaf_id = parent
        .entries
        .last()
        .map(|e| e.id.clone())
        .unwrap_or_default();
    if leaf_id.is_empty() {
        return RpcResponse::build_fail(
            id,
            "clone",
            "Nothing to clone — no messages found in session.",
        );
    }

    // Fork from leaf
    let forked = crate::session::fork_session(&parent, &leaf_id);
    let forked_id = forked.id.clone();

    // Save the forked session
    if let Err(e) = session_manager.save(&forked) {
        return RpcResponse::build_fail(
            id,
            "clone",
            &format!("failed to save cloned session: {}", e),
        );
    }

    // Add to sessions map.  Load the cloned entries into
    // in-memory messages (same reason as fork — prevents
    // the first prompt from truncating history on disk).
    let mut new_sess = ServerSession::new_with_queue_budget(
        forked_id.clone(),
        agent_loop,
        session_manager,
        &forked.cwd,
        broadcaster,
        state.approval_gate.clone(),
        state.model_registry.clone(),
        state.queue_budget.clone(),
    );
    let supports_images =
        crate::models::model_accepts_images_with(&state.model_registry.read(), &forked.model);
    let msgs = crate::session::entries_to_agent_messages(&forked.entries, supports_images);
    *new_sess.messages.write() = msgs;
    if !forked.model.is_empty() {
        new_sess.model = forked.model.clone();
        #[cfg(test)]
        {
            let mut slot = MODEL_SYNC_FAIL_HOOK.lock();
            if matches!(slot.as_ref(), Some((sid, _)) if sid == &session_id) {
                if let Some((_, hook)) = slot.take() {
                    hook(&mut new_sess);
                }
            }
        }
        if let Err(e) = new_sess.set_model(&new_sess.model.clone()) {
            tracing::warn!("[clone] could not sync agent loop model: {e}");
        }
    }
    state.create_session(new_sess);

    RpcResponse::ok(id, "clone", serde_json::json!({"cancelled": false}))
}

fn cmd_refresh_skills(state: &AppState, id: &str) -> String {
    // Always invalidate the cache. install/uninstall write to disk *after* the
    // previous scan, so the cache is stale for them no matter how recently it
    // was refreshed; invalidation is O(1) and the rescan below repopulates it
    // (and warms the cache so the GUI's follow-up get_commands hits the fast
    // path with the new state).
    //
    // This used to be gated behind a 5 s minimum-interval rate limit, but that
    // limit was process-global and kept getting consumed by the harmless scan
    // the GUI fires on startup / page open / agent (re)connect. The invalidation
    // that actually matters — the one right after a write — then landed inside
    // the window and was silently skipped, so get_commands kept returning the
    // pre-install cache and the Skills view showed the old installed/uninstalled
    // state until the app was restarted (which resets the limit and forces a
    // scan). Burst protection is not needed here: invalidation has no I/O cost,
    // and a rescan is a cheap local walk of two directories.
    crate::skills::invalidate_skills_cache();
    let skills = crate::skills::discover_skills_cached(&crate::skills::global_skill_dirs());
    let skill_names: Vec<String> = skills.iter().map(|s| s.name.clone()).collect();
    // Keep the get_state snapshot in step with the discovery cache — reload_config
    // updates it too, but that path needs a session, and refresh_skills is the
    // sessionless post-install/uninstall entry point. Without this, get_state
    // kept reporting the pre-install skill list until the next reload_config.
    *state.welcome_skills.write() = skill_names.clone();
    RpcResponse::ok(
        id,
        "refresh_skills",
        serde_json::json!({
            "skills_count": skill_names.len(),
            "skills": skill_names,
            "refreshed": true,
        }),
    )
}

fn cmd_reload_config(
    state: &AppState,
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    id: &str,
) -> String {
    // Re-discover skills and re-read context files, then rebuild system prompt.
    let (cwd, tools, session_id) = {
        let sess = rlock!(session, id);
        let loop_ = match sess.agent_loop.try_read() {
            Ok(l) => l,
            Err(_) => {
                return RpcResponse::build_fail(
                    id,
                    "reload_config",
                    "agent is busy, retry in a moment",
                );
            }
        };
        (
            sess.cwd.clone(),
            loop_.tools.clone(),
            sess.session_id.clone(),
        )
    };

    // Re-discover skills (blocking I/O, no locks held).  Invalidate the
    // 60s cache first — an explicit reload must see on-disk changes now.
    crate::skills::invalidate_skills_cache();
    let skills = crate::skills::discover_skills_cached(&crate::skills::global_skill_dirs());
    let skill_names: Vec<String> = skills.iter().map(|s| s.name.clone()).collect();

    // Re-read context files
    let mut agent_content = String::new();
    for fname in &["CLAUDE.md", "AGENTS.md", "GEMINI.md"] {
        let p = std::path::Path::new(&cwd).join(fname);
        if p.exists() {
            if let Ok(content) = std::fs::read_to_string(&p) {
                agent_content = content;
                break;
            }
        }
    }
    let context_lines: Vec<String> = if agent_content.is_empty() {
        vec![]
    } else {
        vec![agent_content.clone()]
    };

    // Rebuild system prompt
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let new_prompt = crate::prompt::build_prompt(&crate::prompt::PromptOptions {
        working_directory: cwd.clone(),
        date: today,
        tools: tools.clone(),
        skills: skills.clone(),
        agent_content: agent_content.clone(),
        session_id: session_id.clone(),
        ..Default::default()
    });

    // Update welcome_* state for get_state
    *state.welcome_skills.write() = skill_names.clone();
    *state.welcome_context.write() = context_lines;

    // Update running session's system prompt
    let sess = rlock!(session, id);
    if let Ok(mut r#loop) = sess.agent_loop.try_write() {
        r#loop.system_prompt = new_prompt.clone();
        r#loop.config.system_prompt = new_prompt;
    }

    // Broadcast to all subscribers so other clients (TUI/GUI) update their
    // skill lists and context-file displays in near real-time.
    let sess = rlock!(session, id);
    sess.broadcaster.broadcast(SseEvent::new(
        "config_reloaded",
        serde_json::json!({
            "skills": skill_names,
            "contextFiles": if agent_content.is_empty() { vec![] } else { vec!["CLAUDE.md".to_string()] },
        }),
    ));

    RpcResponse::ok(
        id,
        "reload_config",
        serde_json::json!({
            "skills": skill_names,
            "contextFiles": if agent_content.is_empty() { vec![] } else { vec!["CLAUDE.md".to_string()] },
        }),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agent::Loop,
        rpc::ApprovalGate,
        types::{LLMProvider, Message, StreamEvent, ToolDef},
    };
    use std::collections::HashMap;
    use tokio::sync::mpsc;
    use tokio_stream::wrappers::ReceiverStream;

    struct EmptyProvider;

    #[async_trait::async_trait]
    impl LLMProvider for EmptyProvider {
        async fn stream_chat(
            &self,
            _model: String,
            _messages: Vec<Message>,
            _tools: Vec<ToolDef>,
            _system_prompt: String,
        ) -> anyhow::Result<ReceiverStream<StreamEvent>> {
            let (_tx, rx) = mpsc::channel(1);
            Ok(ReceiverStream::new(rx))
        }
    }

    fn test_workspace() -> String {
        crate::test_support::unique_temp_path("cmd-test")
            .to_string_lossy()
            .to_string()
    }

    /// Unique, isolated session directory for a test's AppState. Each call
    /// gets its own temp dir (timestamp + random hex) so parallel tests never
    /// share a `default.jsonl`, and nothing is ever written to the real
    /// `~/.future/agent/sessions` store (which `Manager::default_for` would
    /// target, since `default_session_dir` ignores its cwd argument).
    fn test_session_dir() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("futureos-cmd-sess-{}", crate::utils::generate_id()))
    }

    fn make_app_state() -> AppState {
        make_app_state_with(
            test_session_dir(),
            Arc::new(crate::runtime::GlobalQueueBudget::defaults()),
        )
    }

    fn make_app_state_with(
        session_dir: std::path::PathBuf,
        queue_budget: Arc<crate::runtime::GlobalQueueBudget>,
    ) -> AppState {
        let cwd = test_workspace();
        let model_registry = Arc::new(parking_lot::RwLock::new(crate::models::Registry::new()));
        let session_manager = Arc::new(crate::session::Manager::new(session_dir));
        let approval_gate = ApprovalGate::default();
        // One live session named "default" — sessions are equal peers now,
        // so tests address it explicitly by id.
        let session = ServerSession::new_with_queue_budget(
            "default".to_string(),
            Arc::new(tokio::sync::RwLock::new(Loop::new(
                Arc::new(EmptyProvider),
                "mock",
            ))),
            session_manager.clone(),
            &cwd,
            Arc::new(SseBroadcaster::new()),
            approval_gate.clone(),
            model_registry.clone(),
            queue_budget.clone(),
        );
        let sessions: HashMap<String, Arc<parking_lot::RwLock<ServerSession>>> = [(
            "default".to_string(),
            Arc::new(parking_lot::RwLock::new(session)),
        )]
        .into_iter()
        .collect();
        AppState {
            agent_instance_id: "agent-test-instance".to_string(),
            sessions: Arc::new(parking_lot::RwLock::new(sessions)),
            queue_budget,
            session_manager,
            welcome_version: "0.0.0".to_string(),
            welcome_cwd: cwd.clone(),
            welcome_skills: Arc::new(parking_lot::RwLock::new(vec![])),
            welcome_context: Arc::new(parking_lot::RwLock::new(vec![])),
            welcome_exts: vec![],
            explicit_session: false,
            approval_gate,
            verbose: false,
            shutting_down: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            model_registry: model_registry.clone(),
            loop_template: Arc::new(Loop::new(Arc::new(EmptyProvider), "mock")),
        }
    }

    fn make_cmd(cmd_type: &str) -> RpcCommand {
        serde_json::from_str(&format!(
            r#"{{"id":"test_cmd","type":"{}","sessionId":"default"}}"#,
            cmd_type
        ))
        .unwrap()
    }

    fn make_cmd_for(cmd_type: &str, session_id: &str) -> RpcCommand {
        serde_json::from_str(&format!(
            r#"{{"id":"test_cmd","type":"{}","sessionId":"{}"}}"#,
            cmd_type, session_id
        ))
        .unwrap()
    }

    fn parse_response(json: &str) -> serde_json::Value {
        serde_json::from_str(json).unwrap()
    }

    fn is_lifecycle_marker(entry_type: &str) -> bool {
        matches!(
            entry_type,
            crate::session::ENTRY_TYPE_RUN_STARTED | crate::session::ENTRY_TYPE_RUN_TERMINAL
        )
    }

    // ── Config-write commands (set_auth / upsert_provider / delete_provider) ──
    // Success paths write auth.json/models.json under $HOME, so they run under
    // a redirected HOME (shared TestHome, serialized on crate::HOME_ENV_LOCK).
    use crate::test_support::TestHome;

    fn read_json(path: &std::path::Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn set_auth_rejects_missing_payload_provider_and_noop() {
        let state = make_app_state();

        let resp = parse_response(&handle_command_internal(&state, make_cmd("set_auth")));
        assert_eq!(resp["success"], false);
        assert!(resp["error"].as_str().unwrap().contains("auth_update"));

        let mut cmd = make_cmd("set_auth");
        cmd.auth_update = Some(crate::config::providers::AuthMutation {
            provider: "  ".to_string(),
            key: Some("k".to_string()),
            ..Default::default()
        });
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert!(resp["error"].as_str().unwrap().contains("provider"));

        let mut cmd = make_cmd("set_auth");
        cmd.auth_update = Some(crate::config::providers::AuthMutation {
            provider: "future".to_string(),
            ..Default::default()
        });
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert!(resp["error"].as_str().unwrap().contains("no change"));
    }

    #[test]
    fn set_auth_writes_auth_json_and_reports_success() {
        let home = TestHome::new();
        let state = make_app_state();

        let mut cmd = make_cmd("set_auth");
        cmd.auth_update = Some(crate::config::providers::AuthMutation {
            provider: "future".to_string(),
            key: Some("k1".to_string()),
            base_url: Some("https://future-os.cn/api".to_string()),
            ..Default::default()
        });
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["provider"], "future");

        let stored = read_json(&home.auth_path());
        assert_eq!(stored["future"]["key"], "k1");
        assert_eq!(stored["future"]["base_url"], "https://future-os.cn/api");
        assert_eq!(stored["future"]["type"], "api_key");
    }

    #[test]
    fn upsert_provider_writes_both_files_and_delete_removes_them() {
        let home = TestHome::new();
        let state = make_app_state();

        let mut cmd = make_cmd("upsert_provider");
        cmd.provider_config = Some(crate::config::providers::ProviderUpsertSpec {
            id: "myprov".to_string(),
            name: Some("My Provider".to_string()),
            api: Some("anthropic".to_string()),
            base_url: Some("https://api.example.com".to_string()),
            api_key: Some("sk-key".to_string()),
            models: vec![crate::config::providers::ProviderModelSpec {
                id: "m1".to_string(),
                name: "Model One".to_string(),
                modalities: vec!["text".to_string()],
            }],
            ..Default::default()
        });
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);

        let models = read_json(&home.models_path());
        assert_eq!(models["providers"]["myprov"]["name"], "My Provider");
        assert_eq!(models["providers"]["myprov"]["models"][0]["id"], "m1");
        let auth = read_json(&home.auth_path());
        assert_eq!(auth["myprov"]["key"], "sk-key");

        // create mode must reject the now-existing id
        let mut cmd = make_cmd("upsert_provider");
        cmd.provider_config = Some(crate::config::providers::ProviderUpsertSpec {
            id: "myprov".to_string(),
            name: Some("Other".to_string()),
            create_only: true,
            ..Default::default()
        });
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert!(resp["error"].as_str().unwrap().contains("already exists"));

        // delete removes the models.json entry and the auth entry
        let mut cmd = make_cmd("delete_provider");
        cmd.provider_config = Some(crate::config::providers::ProviderUpsertSpec {
            id: "myprov".to_string(),
            ..Default::default()
        });
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);

        let models = read_json(&home.models_path());
        assert!(models["providers"].get("myprov").is_none());
        let auth = read_json(&home.auth_path());
        assert!(auth.get("myprov").is_none());
    }

    #[test]
    fn provider_commands_reject_missing_payload_and_empty_id() {
        let state = make_app_state();

        for cmd_type in ["upsert_provider", "delete_provider"] {
            let resp = parse_response(&handle_command_internal(&state, make_cmd(cmd_type)));
            assert_eq!(resp["success"], false, "{cmd_type} without payload");

            let mut cmd = make_cmd(cmd_type);
            cmd.provider_config = Some(crate::config::providers::ProviderUpsertSpec {
                id: " ".to_string(),
                name: Some("x".to_string()),
                ..Default::default()
            });
            let resp = parse_response(&handle_command_internal(&state, cmd));
            assert_eq!(resp["success"], false, "{cmd_type} with empty id");
        }
    }

    #[test]
    fn unknown_command_returns_error() {
        let state = make_app_state();
        let cmd = make_cmd("nonexistent_command");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert!(resp["error"].as_str().unwrap().contains("unknown command"));
    }

    #[test]
    fn prompt_rejects_unknown_busy_policy_with_stable_code() {
        let state = make_app_state();
        let mut cmd = make_cmd("prompt");
        cmd.message = "hello".to_string();
        cmd.busy_policy = "frobnicate".to_string();

        let resp = parse_response(&handle_command_internal(&state, cmd));

        assert_eq!(resp["success"], false);
        assert_eq!(resp["error_code"], "invalid_busy_policy");
        assert_eq!(resp["error_data"]["provided"], "frobnicate");
    }

    #[test]
    fn prompt_enqueue_if_busy_returns_canonical_queued_ack() {
        let state = make_app_state();
        let session = state.get_session("default").unwrap();
        let active = session
            .read()
            .runtime
            .begin(Some("run-active"), Some("request-active"))
            .unwrap();
        assert_eq!(active.run_id, "run-active");

        let mut cmd = make_cmd("prompt");
        cmd.message = "queued later".to_string();
        cmd.busy_policy = "enqueue_if_busy".to_string();
        cmd.client_request_id = "request-next".to_string();

        let resp = parse_response(&handle_command_internal(&state, cmd));

        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["accepted_state"], "queued");
        assert_eq!(resp["data"]["queue_position"], 1);
        assert_eq!(
            session.read().scheduler.queued()[0].client_request_id,
            "request-next"
        );
    }

    #[test]
    fn cancel_queued_run_removes_only_the_requested_run() {
        let state = make_app_state();
        let session = state.get_session("default").unwrap();
        session
            .read()
            .runtime
            .begin(Some("run-active"), Some("request-active"))
            .unwrap();
        for number in 1..=2 {
            let mut prompt = make_cmd("prompt");
            prompt.message = format!("queued {number}");
            prompt.busy_policy = "enqueue_if_busy".to_string();
            prompt.client_request_id = format!("request-{number}");
            prompt.requested_run_id = format!("run-{number}");
            assert_eq!(
                parse_response(&handle_command_internal(&state, prompt))["success"],
                true
            );
        }

        let mut cancel = make_cmd("cancel_queued_run");
        cancel.run_id = "run-1".to_string();
        let response = parse_response(&handle_command_internal(&state, cancel));
        assert_eq!(response["success"], true);
        assert_eq!(response["data"]["state"], "cancelled");
        assert!(session.read().scheduled_setting_summary("run-1").is_none());
        assert_eq!(
            session
                .read()
                .scheduler
                .queued()
                .iter()
                .map(|run| run.run_id.as_str())
                .collect::<Vec<_>>(),
            vec!["run-2"]
        );
    }

    #[test]
    fn delete_session_fences_admission_and_reclaims_queued_snapshots() {
        let state = make_app_state();
        let session = state.get_session("default").unwrap();
        session
            .read()
            .runtime
            .begin(Some("run-active"), Some("request-active"))
            .unwrap();

        let mut queued = make_cmd("prompt");
        queued.message = "must be reclaimed".to_string();
        queued.busy_policy = "enqueue_if_busy".to_string();
        queued.requested_run_id = "run-queued".to_string();
        queued.client_request_id = "request-queued".to_string();
        assert_eq!(
            parse_response(&handle_command_internal(&state, queued))["success"],
            true
        );
        assert!(session
            .read()
            .scheduled_setting_summary("run-queued")
            .is_some());

        let deleting = parse_response(&handle_command_internal(&state, make_cmd("delete_session")));
        assert_eq!(deleting["success"], false);
        assert_eq!(deleting["error_code"], "deleting");
        assert_eq!(deleting["error_data"]["queued_cancelled"], 1);
        assert!(session.read().deleting);
        assert!(session.read().scheduler.queued().is_empty());
        assert!(session
            .read()
            .scheduled_setting_summary("run-queued")
            .is_none());

        let mut rejected = make_cmd("prompt");
        rejected.message = "too late".to_string();
        rejected.client_request_id = "request-too-late".to_string();
        let rejected = parse_response(&handle_command_internal(&state, rejected));
        assert_eq!(rejected["success"], false);
        assert_eq!(rejected["error_code"], "deleting");
    }

    #[test]
    fn delete_idle_session_removes_the_live_runtime() {
        let state = make_app_state();
        let session = state.get_session("default").unwrap();
        assert!(state.sessions.read().contains_key("default"));

        let response = parse_response(&handle_command_internal(&state, make_cmd("delete_session")));

        assert_eq!(response["success"], true);
        assert_eq!(response["data"]["deleted"], true);
        assert!(!state.sessions.read().contains_key("default"));
        assert!(session
            .read()
            .persistence
            .append(vec![crate::session::SessionEntry::new_assistant(
                serde_json::json!("late write"),
                vec![],
            )])
            .is_err());
    }

    #[test]
    fn delete_reclaims_taskless_persistence_degraded_session() {
        let state = make_app_state();
        let session = state.get_session("default").unwrap();
        let lease = session
            .read()
            .runtime
            .begin(Some("run-degraded"), Some("request-degraded"))
            .unwrap();
        assert!(session
            .read()
            .runtime
            .mark_persistence_degraded(&lease, "disk full"));
        assert!(!session.read().runtime.has_owned_task());

        let response = parse_response(&handle_command_internal(&state, make_cmd("delete_session")));

        assert_eq!(response["success"], true);
        assert_eq!(response["data"]["deleted"], true);
        assert!(!state.sessions.read().contains_key("default"));
    }

    #[test]
    fn get_agent_info_returns_version() {
        let state = make_app_state();
        let cmd = make_cmd("get_agent_info");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert!(resp["data"]["version"].is_string());
        assert_eq!(resp["data"]["agentInstanceId"], "agent-test-instance");
    }

    #[test]
    fn get_state_returns_session_info() {
        let state = make_app_state();
        state
            .get_session("default")
            .unwrap()
            .read()
            .scheduler
            .accept(
                "queued-request",
                Some("queued-run"),
                crate::runtime::BusyPolicy::EnqueueIfBusy,
                serde_json::json!({"message":"later"}),
            )
            .unwrap();
        let cmd = make_cmd("get_state");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert!(resp["data"]["sessionId"].is_string());
        assert_eq!(resp["data"]["agentInstanceId"], "agent-test-instance");
        assert_eq!(resp["data"]["queuedCount"], 1);
        assert_eq!(resp["data"]["queuedRuns"][0]["runId"], "queued-run");
        assert_eq!(resp["data"]["queuedRuns"][0]["displayText"], "later");
    }

    #[test]
    fn get_state_reports_pending_approvals_for_owning_session() {
        let state = make_app_state();
        state
            .approval_gate
            .insert_pending_for_test("approval_req1", "default");
        state
            .approval_gate
            .insert_pending_for_test("approval_req2", "other-session");

        let resp = parse_response(&handle_command_internal(&state, make_cmd("get_state")));
        assert_eq!(resp["success"], true);
        // Only the session's own pending requests surface — never another
        // session's (ownership rule, same as approval decisions).
        let pending = resp["data"]["pendingApprovals"].as_array().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0]["approval_request_id"], "approval_req1");
        assert_eq!(pending[0]["session_id"], "default");
    }

    #[test]
    fn get_state_pending_approvals_empty_when_none() {
        let state = make_app_state();
        let resp = parse_response(&handle_command_internal(&state, make_cmd("get_state")));
        assert_eq!(resp["success"], true);
        assert_eq!(
            resp["data"]["pendingApprovals"].as_array().unwrap().len(),
            0
        );
    }

    #[test]
    fn get_state_reports_interrupted_run_when_journal_unterminated() {
        let state = make_app_state();
        // Each make_app_state() now gets an isolated temp session dir (see
        // test_session_dir), so this test no longer shares a file with other
        // tests; the explicit id just names the session under test. get_state
        // hydrates the session from disk on demand.
        let session_id = "gi-interrupted";
        let info = crate::session::SessionEntry::session_info(
            serde_json::json!({"cwd": state.welcome_cwd, "model": "mock", "session_name": "n"}),
            "mock".to_string(),
            "low".to_string(),
        );
        let session = crate::session::Session::snapshot(
            session_id.to_string(),
            state.welcome_cwd.clone(),
            "mock".to_string(),
            "n".to_string(),
            String::new(),
            vec![
                info,
                crate::session::SessionEntry::new_user("user", serde_json::json!("hi")),
                crate::session::SessionEntry::run_started("run-interrupted", 3),
            ],
        );
        state.session_manager.save(&session).unwrap();

        let resp = parse_response(&handle_command_internal(
            &state,
            make_cmd_for("get_state", session_id),
        ));
        assert_eq!(resp["success"], true);
        // No live run, so activeRun is null and the unterminated run is surfaced
        // as interrupted_by_restart for the GUI's startup reconcile to consume.
        assert!(resp["data"]["activeRun"].is_null());
        assert_eq!(resp["data"]["interruptedRun"]["runId"], "run-interrupted");
        assert_eq!(
            resp["data"]["interruptedRun"]["state"],
            crate::session::RUN_STATE_INTERRUPTED_BY_RESTART
        );
        let _ = state.session_manager.delete(session_id);
    }

    #[test]
    fn get_state_omits_interrupted_run_once_terminal_present() {
        let state = make_app_state();
        let session_id = "gi-terminal";
        let info = crate::session::SessionEntry::session_info(
            serde_json::json!({"cwd": state.welcome_cwd, "model": "mock", "session_name": "n"}),
            "mock".to_string(),
            "low".to_string(),
        );
        let session = crate::session::Session::snapshot(
            session_id.to_string(),
            state.welcome_cwd.clone(),
            "mock".to_string(),
            "n".to_string(),
            String::new(),
            vec![
                info,
                crate::session::SessionEntry::new_user("user", serde_json::json!("hi")),
                crate::session::SessionEntry::run_started("run-done", 1),
                crate::session::SessionEntry::run_terminal(
                    "run-done",
                    crate::session::RUN_STATE_COMPLETED,
                    5,
                    50,
                    None,
                ),
            ],
        );
        state.session_manager.save(&session).unwrap();

        let mut command = make_cmd_for("get_state", session_id);
        command.run_id = "run-done".to_string();
        let resp = parse_response(&handle_command_internal(&state, command));
        assert_eq!(resp["success"], true);
        assert!(resp["data"]["activeRun"].is_null());
        assert!(resp["data"]["interruptedRun"].is_null());
        assert_eq!(resp["data"]["requestedRun"]["run_id"], "run-done");
        assert_eq!(
            resp["data"]["requestedRun"]["state"],
            crate::session::RUN_STATE_COMPLETED
        );
        let _ = state.session_manager.delete(session_id);
    }

    #[test]
    fn get_state_preserves_markerless_legacy_history_without_reporting_interruption() {
        // Backward compatibility: sessions written before run lifecycle markers
        // (run_started/run_terminal) existed carry no run identity in their
        // JSONL. They must never be misclassified as an interrupted run, and
        // the compatibility read must not rewrite or discard their history
        // (no run_id backfill is performed on legacy data).
        let state = make_app_state();
        let session_id = "gi-legacy";
        let info = crate::session::SessionEntry::session_info(
            serde_json::json!({"cwd": state.welcome_cwd, "model": "mock", "session_name": "n"}),
            "mock".to_string(),
            "low".to_string(),
        );
        let session = crate::session::Session::snapshot(
            session_id.to_string(),
            state.welcome_cwd.clone(),
            "mock".to_string(),
            "n".to_string(),
            String::new(),
            vec![
                info,
                crate::session::SessionEntry::new_user("user", serde_json::json!("legacy message")),
                crate::session::SessionEntry::new_assistant(
                    serde_json::json!("legacy reply"),
                    vec![],
                ),
            ],
        );
        state.session_manager.save(&session).unwrap();

        let resp = parse_response(&handle_command_internal(
            &state,
            make_cmd_for("get_state", session_id),
        ));
        assert_eq!(resp["success"], true);
        assert!(resp["data"]["activeRun"].is_null());
        // No run_started marker → nothing unterminated → not interrupted.
        assert!(resp["data"]["interruptedRun"].is_null());
        let loaded = state.session_manager.load(session_id).unwrap();
        assert!(loaded
            .entries
            .iter()
            .any(|entry| entry.content == Some(serde_json::json!("legacy message"))));
        assert!(loaded
            .entries
            .iter()
            .any(|entry| entry.content == Some(serde_json::json!("legacy reply"))));
        assert!(loaded
            .entries
            .iter()
            .all(|entry| { !is_lifecycle_marker(entry.entry_type.as_str()) }));
        let _ = state.session_manager.delete(session_id);
    }

    #[test]
    fn lifecycle_marker_helpers_recognize_markers() {
        assert!(is_lifecycle_marker(crate::session::ENTRY_TYPE_RUN_STARTED));
        assert!(is_lifecycle_marker(crate::session::ENTRY_TYPE_RUN_TERMINAL));
        assert!(!is_lifecycle_marker("user"));
        assert!(!is_lifecycle_marker("assistant"));
    }

    #[test]
    fn refresh_skills_returns_skill_list() {
        let state = make_app_state();
        let cmd = make_cmd("refresh_skills");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert!(resp["data"]["skills_count"].is_number());
        assert!(resp["data"]["skills"].is_array());
        assert_eq!(
            resp["data"]["skills_count"].as_u64().unwrap(),
            resp["data"]["skills"].as_array().unwrap().len() as u64
        );
        assert!(resp["data"]["refreshed"].is_boolean());
        // The get_state snapshot must follow the discovery cache: reload_config
        // needs a session, so refresh_skills is the only post-install path that
        // can update it. A stale welcome_skills made get_state keep reporting
        // the pre-install skill list.
        let welcomed = state.welcome_skills.read().clone();
        let returned: Vec<String> = resp["data"]["skills"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_string())
            .collect();
        assert_eq!(welcomed, returned);
    }

    #[test]
    fn refresh_skills_works_without_session_id() {
        // Regression: refresh_skills is sessionless. The GUI/CLI fire it right
        // after install/uninstall with NO session_id; when it lived in the
        // session-scoped branch this returned "session not found", the skills
        // cache was never invalidated, and the installed list stayed stale
        // until restart / TTL expiry. make_cmd() always injects a session id,
        // so it hid this — build the command by hand with an empty session.
        let state = make_app_state();
        let cmd: RpcCommand =
            serde_json::from_str(r#"{"id":"test_cmd","type":"refresh_skills","sessionId":""}"#)
                .unwrap();
        assert!(cmd.session_id.is_empty());
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["command"], "refresh_skills");
    }

    #[test]
    fn sessionless_commands_do_not_require_session_id() {
        // Regression: every sessionless command must be dispatched WITHOUT
        // resolving a session. If one is accidentally moved into the
        // session-scoped branch, an empty session_id trips the resolution gate
        // and the caller gets "session not found — pass a valid session_id..."
        // (that exact phrase is unique to the gate). make_cmd() always injects
        // a session id so it can't catch this — build each command by hand with
        // an empty session and assert we never hit the gate.
        //
        // `reload_auth` and `shutdown` are deliberately excluded: they carry
        // process-global side effects (credential reload / shutdown flag) that
        // don't belong in a swept table.
        let sessionless = [
            "get_agent_info",
            "list_models",
            "list_sessions",
            "list_streaming_sessions",
            "new_session",
            "switch_session",
            "delete_session",
            "get_fork_messages",
            "get_commands",
            "refresh_skills",
            "set_enabled_models",
        ];
        for cmd_type in sessionless {
            let state = make_app_state();
            let cmd: RpcCommand = serde_json::from_str(&format!(
                r#"{{"id":"test_cmd","type":"{cmd_type}","sessionId":""}}"#
            ))
            .unwrap();
            assert!(cmd.session_id.is_empty());
            let resp = parse_response(&handle_command_internal(&state, cmd));
            let error = resp["error"].as_str().unwrap_or("");
            // The command must actually exist (the fallback echoes cmd_type, so
            // a successful dispatch and a typo both return command == cmd_type —
            // "unknown command" in the error is the real tell).
            assert!(
                !error.contains("unknown command"),
                "sessionless cmd {cmd_type} is not a known command: {error}"
            );
            // And it must not have failed at the session-resolution gate. A
            // command may still fail for its own reasons (e.g. switch_session
            // with an empty target) — that's fine; only the gate phrase is a
            // regression signal.
            assert!(
                !error.contains("pass a valid session_id"),
                "sessionless cmd {cmd_type} required a session: {error}"
            );
        }
    }

    #[test]
    fn shutdown_sets_flag() {
        let state = make_app_state();
        let cmd = make_cmd("shutdown");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert!(state
            .shutting_down
            .load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn prompt_rejected_after_shutdown() {
        let state = make_app_state();
        let cmd = make_cmd("shutdown");
        handle_command_internal(&state, cmd);
        let mut cmd = make_cmd("prompt");
        cmd.message = "hello".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert!(resp["error"].as_str().unwrap().contains("shutting down"));
    }

    #[test]
    fn set_permission_level_valid() {
        let state = make_app_state();
        let mut cmd = make_cmd("set_permission_level");
        cmd.level = "workspace".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["permissionLevel"], "workspace");
    }

    #[test]
    fn set_permission_level_invalid() {
        let state = make_app_state();
        let mut cmd = make_cmd("set_permission_level");
        cmd.level = "invalid_level".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert!(resp["error"].as_str().unwrap().contains("invalid level"));
    }

    #[test]
    fn set_thinking_level_works() {
        let state = make_app_state();
        let mut cmd = make_cmd("set_thinking_level");
        cmd.level = "high".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
    }

    #[test]
    fn set_auto_compaction_works() {
        let state = make_app_state();
        let mut cmd = make_cmd("set_auto_compaction");
        cmd.enabled = false;
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
    }

    #[test]
    fn set_auto_retry_works() {
        let state = make_app_state();
        let mut cmd = make_cmd("set_auto_retry");
        cmd.enabled = true;
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
    }

    #[test]
    fn set_ephemeral_works() {
        let state = make_app_state();
        let mut cmd = make_cmd("set_ephemeral");
        cmd.ephemeral = true;
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["ephemeral"], true);
    }

    #[test]
    fn abort_works() {
        let state = make_app_state();
        let cmd = make_cmd("abort");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
    }

    #[test]
    fn stale_run_scoped_commands_are_rejected_without_touching_current_run() {
        let state = make_app_state();
        let session = state.get_session("default").unwrap();
        let lease = session
            .read()
            .runtime
            .begin(Some("run-current"), None)
            .unwrap();

        let mut abort = make_cmd("abort");
        abort.run_id = "run-old".to_string();
        let response = parse_response(&handle_command_internal(&state, abort));
        assert_eq!(response["success"], false);
        assert!(response["error"]
            .as_str()
            .is_some_and(|error| error.contains("run-old")));
        assert_eq!(
            session.read().runtime.snapshot().unwrap().phase,
            crate::runtime::RunPhase::Starting
        );
        let mut abort = make_cmd("abort");
        abort.run_id = "run-current".to_string();
        let response = parse_response(&handle_command_internal(&state, abort));
        assert_eq!(response["success"], true);
        assert_eq!(
            session.read().runtime.snapshot().unwrap().phase,
            crate::runtime::RunPhase::Cancelling
        );
        assert!(session.read().runtime.begin_finalizing(&lease));
        assert!(session.read().runtime.finish(&lease));
    }

    #[test]
    fn get_messages_returns_empty() {
        let state = make_app_state();
        let cmd = make_cmd("get_messages");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert!(resp["data"]["messages"].is_array());
    }

    #[test]
    fn get_session_stats_works() {
        let state = make_app_state();
        let cmd = make_cmd("get_session_stats");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert!(resp["data"]["sessionId"].is_string());
    }

    #[test]
    fn get_runtime_metrics_exposes_five_observability_values() {
        let state = make_app_state();
        let session = state.get_session("default").unwrap();
        let (runtime, broadcaster) = {
            let session = session.read();
            (session.runtime.clone(), session.broadcaster.clone())
        };
        let lease = runtime.begin(Some("run-metrics"), None).unwrap();
        broadcaster.record_lag();

        let cmd = make_cmd("get_runtime_metrics");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert!(resp["data"]["sessionId"].is_string());
        assert_eq!(resp["data"]["activeRunGauge"], 1);
        assert_eq!(resp["data"]["activeRunId"], "run-metrics");
        assert_eq!(resp["data"]["broadcastLag"], 1);
        for field in ["staleEpochDrops", "persistenceDegraded", "ringTruncations"] {
            assert_eq!(resp["data"][field], 0, "unexpected {field}");
        }

        assert!(runtime.begin_finalizing(&lease));
        assert!(runtime.finish(&lease));
    }

    #[test]
    fn cycle_thinking_level_advances() {
        let state = make_app_state();
        let cmd = make_cmd("cycle_thinking_level");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert!(resp["data"]["level"].is_string());
    }

    #[test]
    fn set_enabled_models_accepted() {
        let state = make_app_state();
        let cmd = make_cmd("set_enabled_models");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
    }

    #[test]
    fn disable_tools_works() {
        let state = make_app_state();
        let cmd = make_cmd("disable_tools");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
    }

    #[test]
    fn disable_builtin_tools_works() {
        let state = make_app_state();
        let cmd = make_cmd("disable_builtin_tools");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
    }

    #[test]
    fn set_system_prompt_works() {
        let state = make_app_state();
        let mut cmd = make_cmd("set_system_prompt");
        cmd.system_prompt = "You are helpful".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
    }

    #[test]
    fn append_system_prompt_works() {
        let state = make_app_state();
        let mut cmd = make_cmd("append_system_prompt");
        cmd.system_prompt = "Extra instructions".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
    }

    #[test]
    fn set_cwd_trims_trailing_slash() {
        let state = make_app_state();
        let mut cmd = make_cmd("set_cwd");
        cmd.cwd = "/tmp/project/ ".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["cwd"], "/tmp/project");
    }

    /// `create_session` swaps in a fresh private broadcaster (fork/clone pass
    /// the parent's); the event journal must be rebound to that broadcaster or
    /// events silently stay memory-only and the durable journal is never
    /// written. Regression guard for the Runs-panel blanking bug.
    #[test]
    fn create_session_rebinds_event_journal_to_live_broadcaster() {
        let state = make_app_state();
        let session_id = "journal-rebind".to_string();

        // Mimic fork/clone: construct with a broadcaster that create_session
        // will discard, so the live one must be (re)configured by it.
        let new_sess = ServerSession::new_with_queue_budget(
            session_id.clone(),
            Arc::new(tokio::sync::RwLock::new(Loop::new(
                Arc::new(EmptyProvider),
                "mock",
            ))),
            state.session_manager.clone(),
            &test_workspace(),
            Arc::new(SseBroadcaster::new()),
            ApprovalGate::default(),
            state.model_registry.clone(),
            state.queue_budget.clone(),
        );
        state.create_session(new_sess);

        let session_arc = {
            let sessions = state.sessions.read();
            sessions.get(&session_id).unwrap().clone()
        };
        let live_broadcaster = session_arc.read().broadcaster.clone();
        live_broadcaster.start_run("run-j".to_string(), 1);
        live_broadcaster.broadcast(crate::rpc::SseEvent::new(
            "text_chunk",
            serde_json::json!({"text": "hello"}),
        ));

        let journal = state
            .session_manager
            .run_data_path(&session_id)
            .join("run-j.jsonl");
        assert!(
            journal.exists(),
            "live broadcaster must write the durable event journal"
        );
    }

    #[test]
    fn set_sandbox_policy_missing_payload() {
        let state = make_app_state();
        let cmd = make_cmd("set_sandbox_policy");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert!(resp["error"]
            .as_str()
            .unwrap()
            .contains("missing sandbox_policy"));
    }

    #[test]
    fn compact_empty_session() {
        let state = make_app_state();
        let cmd = make_cmd("compact");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["messagesRemoved"], 0);
    }

    #[test]
    fn approval_decision_invalid_mode() {
        let state = make_app_state();
        let mut cmd = make_cmd("approval_decision");
        cmd.mode = "invalid".to_string();
        cmd.entry_id = "req_1".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert!(resp["error"]
            .as_str()
            .unwrap()
            .contains("approved, rejected, or cancelled"));
    }

    #[test]
    fn shell_echo() {
        let state = make_app_state();
        std::fs::create_dir_all(&state.welcome_cwd).unwrap();
        let mut cmd = make_cmd("shell");
        cmd.command = "echo test_output".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert!(resp["data"]["output"]
            .as_str()
            .unwrap()
            .contains("test_output"));
        assert_eq!(resp["data"]["exitCode"], 0);
    }

    #[test]
    fn abort_retry_works() {
        let state = make_app_state();
        let cmd = make_cmd("abort_retry");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
    }

    #[test]
    fn list_sessions_returns_array() {
        let state = make_app_state();
        let cmd = make_cmd("list_sessions");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert!(resp["data"]["sessions"].is_array());
    }

    #[test]
    fn list_session_ids_reports_all_files_including_corrupt() {
        let state = make_app_state();
        // Persist one real session.
        let mut session = crate::session::Session::new("/tmp", "mock", "");
        session
            .entries
            .push(crate::session::SessionEntry::session_info(
                serde_json::json!({"cwd": "/tmp", "model": "mock"}),
                "mock".to_string(),
                "low".to_string(),
            ));
        state.session_manager.save(&session).unwrap();
        // Drop a corrupt JSONL next to it — must STILL be reported as a live
        // session id (orphan cleanup depends on filename-only enumeration).
        let corrupt_id = "corrupt-session";
        std::fs::write(
            state
                .session_manager
                .dir
                .join(format!("{corrupt_id}.jsonl")),
            "{ not json",
        )
        .unwrap();

        let resp = parse_response(&handle_command_internal(
            &state,
            make_cmd("list_session_ids"),
        ));
        assert_eq!(resp["success"], true);
        let mut ids: Vec<String> = resp["data"]["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        ids.sort();
        assert_eq!(ids, vec![session.id.clone(), corrupt_id.to_string()]);
    }

    /// Audit item 1 contract: list_sessions rows carry canonical camelCase
    /// keys AND the legacy snake_case spellings, with identical values, so
    /// pre-migration clients keep working.
    #[test]
    fn list_sessions_emits_canonical_and_legacy_keys() {
        let state = make_app_state();
        // list_sessions reads persisted session summaries, so persist one —
        // the summary fields come from the session_info entry.
        let mut session = crate::session::Session::new("/tmp", "mock", "");
        session.name = "My session".to_string();
        session
            .entries
            .push(crate::session::SessionEntry::session_info(
                serde_json::json!({"cwd": "/tmp", "model": "mock", "session_name": "My session"}),
                "mock".to_string(),
                "low".to_string(),
            ));
        session.entries.push(crate::session::SessionEntry::new_user(
            "user",
            serde_json::json!("hello"),
        ));
        state.session_manager.save(&session).unwrap();

        let resp = parse_response(&handle_command_internal(&state, make_cmd("list_sessions")));
        assert_eq!(resp["success"], true);
        let sessions = resp["data"]["sessions"].as_array().unwrap();
        assert_eq!(sessions.len(), 1);
        let entry = &sessions[0];
        assert!(!entry["id"].as_str().unwrap().is_empty());
        assert_eq!(entry["sessionName"], "My session");
        assert_eq!(entry["firstMessage"], "hello");
        for (canonical, legacy) in [
            ("sessionName", "session_name"),
            ("updatedAt", "updated_at"),
            ("parentSessionId", "parent_session_id"),
            ("firstMessage", "first_message"),
            ("queryCount", "query_count"),
            ("isStreaming", "is_streaming"),
        ] {
            assert!(entry.get(canonical).is_some(), "missing `{canonical}`");
            assert_eq!(
                entry.get(canonical),
                entry.get(legacy),
                "`{canonical}` and `{legacy}` must carry the same value"
            );
        }
    }

    /// Audit item 1 contract: get_state carries canonical camelCase keys; the
    /// one key whose spelling changed (`sessionName`) is additionally emitted
    /// under its legacy `session_name` name for pre-migration clients.
    #[test]
    fn get_state_emits_canonical_and_legacy_session_name() {
        let state = make_app_state();
        state.sessions.read()["default"]
            .write()
            .set_session_name("My session");
        let resp = parse_response(&handle_command_internal(&state, make_cmd("get_state")));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["sessionName"], "My session");
        assert_eq!(resp["data"]["session_name"], "My session");
        // Spot-check canonical camelCase keys around it.
        assert!(resp["data"].get("agentInstanceId").is_some());
        assert!(resp["data"].get("autoCompactionEnabled").is_some());
        assert!(resp["data"].get("queuedCount").is_some());
    }

    #[test]
    fn list_streaming_sessions_reports_only_streaming() {
        let state = make_app_state();
        let cmd = make_cmd("list_streaming_sessions");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert_eq!(
            resp["data"]["sessionIds"].as_array().unwrap().len(),
            0,
            "nothing streams at startup"
        );

        state.sessions.read()["default"]
            .read()
            .is_streaming
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let resp = parse_response(&handle_command_internal(
            &state,
            make_cmd("list_streaming_sessions"),
        ));
        let ids = resp["data"]["sessionIds"].as_array().unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], "default");
    }

    #[test]
    fn reload_auth_works() {
        let state = make_app_state();
        let cmd = make_cmd("reload_auth");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
    }

    #[test]
    fn get_events_since_rejects_unknown_run() {
        let state = make_app_state();
        let mut cmd = make_cmd("get_events_since");
        cmd.run_id = "run_1".to_string();
        cmd.since_idx = -1;
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert!(resp["error"]
            .as_str()
            .is_some_and(|error| error.contains("not configured") || error.contains("not known")));
    }

    fn chunk_event(data_size: usize) -> SseEvent {
        SseEvent::new(
            "text_chunk",
            serde_json::json!({"text": "x".repeat(data_size)}),
        )
    }

    #[test]
    fn page_events_tail_unlimited_without_max_events() {
        let events = vec![chunk_event(10); 3];
        for max_events in [0, -1] {
            let (page, has_more) = super::page_events_tail(events.clone(), max_events);
            assert_eq!(page.len(), 3);
            assert!(!has_more);
        }
    }

    #[test]
    fn page_events_tail_count_cap_sets_has_more() {
        let events = vec![chunk_event(10); 5];
        let (page, has_more) = super::page_events_tail(events, 2);
        assert_eq!(page.len(), 2);
        assert!(has_more);

        // Exact fit: no tail remains, has_more stays false.
        let events = vec![chunk_event(10); 2];
        let (page, has_more) = super::page_events_tail(events, 2);
        assert_eq!(page.len(), 2);
        assert!(!has_more);
    }

    #[test]
    fn page_events_tail_byte_budget_cuts_before_count_cap() {
        // Events sized to exactly a quarter of the budget (data = text plus
        // the 11-byte `{"text":""}` JSON envelope): four fit, the fifth is
        // cut even though the count cap allows more.
        let quarter = super::EVENTS_PAGE_BYTE_BUDGET / 4 - super::EVENT_WIRE_OVERHEAD - 11;
        let events = vec![chunk_event(quarter); 5];
        let (page, has_more) = super::page_events_tail(events, 10);
        assert_eq!(page.len(), 4);
        assert!(has_more);
    }

    #[test]
    fn page_events_tail_oversized_first_event_still_progresses() {
        // A single event larger than the budget must still go out alone —
        // otherwise the caller's cursor never advances and paging deadlocks.
        let events = vec![
            chunk_event(super::EVENTS_PAGE_BYTE_BUDGET + 1),
            chunk_event(10),
        ];
        let (page, has_more) = super::page_events_tail(events, 10);
        assert_eq!(page.len(), 1);
        assert!(has_more);
    }

    #[test]
    fn get_events_since_pages_a_live_run_with_max_events() {
        let state = make_app_state();
        let session = state.get_session("default").expect("default session");
        let broadcaster = {
            let sess = session.read();
            sess.broadcaster.start_run("run_page".to_string(), 1);
            sess.broadcaster.clone()
        };
        for idx in 0..5 {
            broadcaster.broadcast(SseEvent::new(
                "text_chunk",
                serde_json::json!({"text": format!("chunk-{idx}")}),
            ));
        }

        // Page 1: since the beginning, two events per page.
        let mut cmd = make_cmd("get_events_since");
        cmd.run_id = "run_page".to_string();
        cmd.since_idx = -1;
        cmd.max_events = 2;
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        let data = &resp["data"];
        // The paged envelope must still encode its typed payload (dual-write).
        assert!(future_rpc::encode::response_payload("get_events_since", data).is_some());
        let events = data["events"].as_array().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(data["hasMore"], true);
        assert_eq!(events[0]["idx"], 0);
        assert_eq!(events[1]["idx"], 1);

        // Page 2 follows from the last idx; the final page reports no tail.
        let mut cursor = events.last().unwrap()["idx"].as_i64().unwrap();
        let mut seen = events.len();
        loop {
            let mut cmd = make_cmd("get_events_since");
            cmd.run_id = "run_page".to_string();
            cmd.since_idx = cursor;
            cmd.max_events = 2;
            let resp = parse_response(&handle_command_internal(&state, cmd));
            let data = &resp["data"];
            let events = data["events"].as_array().unwrap();
            seen += events.len();
            let has_more = data["hasMore"].as_bool().unwrap_or(false);
            if let Some(last) = events.last() {
                cursor = last["idx"].as_i64().unwrap();
            }
            if !has_more {
                break;
            }
            assert!(!events.is_empty(), "has_more page must not be empty");
        }
        assert_eq!(seen, 5);
        assert_eq!(cursor, 4);

        // Legacy unpaged read: the whole tail, no hasMore key on the wire.
        let mut cmd = make_cmd("get_events_since");
        cmd.run_id = "run_page".to_string();
        cmd.since_idx = -1;
        let resp = parse_response(&handle_command_internal(&state, cmd));
        let data = &resp["data"];
        assert_eq!(data["events"].as_array().unwrap().len(), 5);
        assert!(data.get("hasMore").is_none());
    }

    #[test]
    fn set_default_model_reports_unsaveable_settings() {
        let home = TestHome::new();
        let state = make_app_state();
        // A valid but READ-ONLY settings.json: load succeeds, save fails.
        let settings_path = home.settings_path();
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        std::fs::write(&settings_path, "{}").unwrap();
        let mut perms = std::fs::metadata(&settings_path).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&settings_path, perms).unwrap();
        let candidate = {
            let registry = state.model_registry.read();
            let model = registry.all_models().first().unwrap().clone();
            format!("{}/{}", model.provider, model.id)
        };
        let mut cmd = make_cmd("set_default_model");
        cmd.model_id = candidate;
        let resp = parse_response(&handle_command_internal(&state, cmd));
        let mut perms = std::fs::metadata(&settings_path).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        std::fs::set_permissions(&settings_path, perms).unwrap();
        assert_eq!(resp["success"], false);
        assert!(resp["error"]
            .as_str()
            .unwrap()
            .contains("failed to save settings"));
    }

    #[test]
    fn reload_config_reports_busy_loop_and_skips_locked_update() {
        let state = make_app_state();
        let session = state.get_session("default").unwrap();
        let agent_loop = session.read().agent_loop.clone();
        {
            // A held WRITE guard makes the first try_read fail.
            let _write_guard = agent_loop.try_write().unwrap();
            let resp = parse_response(&handle_command_internal(&state, make_cmd("reload_config")));
            assert_eq!(resp["success"], false);
            assert!(resp["error"].as_str().unwrap().contains("agent is busy"));
        }
        // A held READ guard passes the try_read but blocks the final try_write
        // — the command still succeeds, just without updating the prompt.
        let _read_guard = agent_loop.try_read().unwrap();
        let resp = parse_response(&handle_command_internal(&state, make_cmd("reload_config")));
        assert_eq!(resp["success"], true);
    }

    #[test]
    fn reload_config_tolerates_unreadable_context_file() {
        let state = make_app_state();
        // A CLAUDE.md that is a DIRECTORY exists but cannot be read.
        let cwd = state.welcome_cwd.clone();
        std::fs::create_dir_all(std::path::Path::new(&cwd).join("CLAUDE.md")).unwrap();
        let resp = parse_response(&handle_command_internal(&state, make_cmd("reload_config")));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["contextFiles"], serde_json::json!([]));
        let _ = std::fs::remove_dir_all(&cwd);
    }

    #[test]
    fn get_commands_returns_list() {
        let state = make_app_state();
        let cmd = make_cmd("get_commands");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        let commands = resp["data"]["commands"].as_array().unwrap();
        // Commands list may be empty in minimal environments (no skills installed)
        assert!(commands.iter().all(|c| c.is_object()));
    }

    #[test]
    fn add_session_rule_works() {
        let state = make_app_state();
        let mut cmd = make_cmd("add_session_rule");
        cmd.message = "/tmp/**".to_string();
        cmd.mode = "read".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
    }

    /// The gRPC boundary dual-writes a typed payload for the Tier-1 read
    /// commands; this pins that the agent's REAL envelopes always encode
    /// (a None here would silently degrade typed clients to the JSON
    /// fallback). get_events_since is covered by the future-rpc parity
    /// fixtures — it needs a live run this fixture does not have.
    #[test]
    fn typed_payload_encodes_real_read_command_envelopes() {
        let state = make_app_state();
        // Session-scoped read commands.
        for cmd_type in ["get_state", "list_sessions", "get_session_entries"] {
            let envelope = parse_response(&handle_command_internal(&state, make_cmd(cmd_type)));
            assert_eq!(envelope["success"], true, "{cmd_type} must succeed");
            let data = &envelope["data"];
            let payload = future_rpc::encode::response_payload(cmd_type, data);
            assert!(payload.is_some(), "{cmd_type}: typed payload must encode");
        }
        // Sessionless commands.
        for cmd_type in [
            "get_agent_info",
            "list_models",
            "get_commands",
            "refresh_skills",
        ] {
            let envelope = parse_response(&handle_command_internal(&state, make_cmd(cmd_type)));
            assert_eq!(envelope["success"], true, "{cmd_type} must succeed");
            let data = &envelope["data"];
            let payload = future_rpc::encode::response_payload(cmd_type, data);
            assert!(payload.is_some(), "{cmd_type}: typed payload must encode");
        }
    }

    // ── coverage batch 1: sessionless dispatch + config-write paths ─────────

    #[test]
    fn sync_future_models_without_credentials_reports_not_synced() {
        let _home = TestHome::new();
        let state = make_app_state();
        let resp = parse_response(&handle_command_internal(
            &state,
            make_cmd("sync_future_models"),
        ));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["synced"], false);
        assert!(resp["data"]["modelCount"].is_number());
    }

    #[test]
    fn set_default_model_rejects_empty_and_unknown_ids() {
        let _home = TestHome::new();
        let state = make_app_state();

        let resp = parse_response(&handle_command_internal(
            &state,
            make_cmd("set_default_model"),
        ));
        assert_eq!(resp["success"], false);
        assert!(resp["error"]
            .as_str()
            .unwrap()
            .contains("model_id is empty"));

        let mut cmd = make_cmd("set_default_model");
        cmd.model_id = "no-such-provider/no-such-model".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert!(resp["error"]
            .as_str()
            .unwrap()
            .contains("not in the catalog"));
    }

    #[test]
    fn set_default_model_persists_catalog_entry() {
        let home = TestHome::new();
        let state = make_app_state();
        let candidate = {
            let registry = state.model_registry.read();
            let model = registry
                .all_models()
                .first()
                .expect("builtin catalog is never empty")
                .clone();
            format!("{}/{}", model.provider, model.id)
        };
        let mut cmd = make_cmd("set_default_model");
        cmd.model_id = candidate.clone();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["defaultModel"], candidate);
        let settings = read_json(&home.settings_path());
        assert_eq!(settings["defaultModel"], candidate);
    }

    #[test]
    fn set_default_model_reports_unloadable_settings() {
        let home = TestHome::new();
        let state = make_app_state();
        // Corrupt settings.json so load_settings fails.
        let settings_path = home.settings_path();
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        std::fs::write(&settings_path, "{not json").unwrap();
        let candidate = {
            let registry = state.model_registry.read();
            let model = registry.all_models().first().unwrap().clone();
            format!("{}/{}", model.provider, model.id)
        };
        let mut cmd = make_cmd("set_default_model");
        cmd.model_id = candidate;
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert!(resp["error"]
            .as_str()
            .unwrap()
            .contains("failed to load settings"));
    }

    // ── coverage batch 1: prompt-adjacent dispatch arms ─────────────────────

    #[test]
    fn prompt_generates_client_request_id_when_omitted() {
        let state = make_app_state();
        let session = state.get_session("default").unwrap();
        session
            .read()
            .runtime
            .begin(Some("run-active"), Some("request-active"))
            .unwrap();

        let mut cmd = make_cmd("prompt");
        cmd.message = "hello".to_string();
        cmd.busy_policy = "enqueue_if_busy".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        let session = state.get_session("default").unwrap();
        let request_id = session.read().scheduler.queued()[0]
            .client_request_id
            .clone();
        assert!(
            request_id.starts_with("request_"),
            "generated client_request_id, got {request_id:?}"
        );
    }

    #[test]
    fn prompt_reports_duplicate_request_conflict() {
        let state = make_app_state();
        let session = state.get_session("default").unwrap();
        session
            .read()
            .runtime
            .begin(Some("run-active"), Some("request-active"))
            .unwrap();

        let mut first = make_cmd("prompt");
        first.message = "one".to_string();
        first.busy_policy = "enqueue_if_busy".to_string();
        first.client_request_id = "dup-req".to_string();
        let resp = parse_response(&handle_command_internal(&state, first));
        assert_eq!(resp["success"], true);

        let mut second = make_cmd("prompt");
        second.message = "two — different body, same request id".to_string();
        second.busy_policy = "enqueue_if_busy".to_string();
        second.client_request_id = "dup-req".to_string();
        let resp = parse_response(&handle_command_internal(&state, second));
        assert_eq!(resp["success"], false);
        assert_eq!(resp["error_code"], "duplicate_request_conflict");
    }

    #[test]
    fn prompt_rejects_unsafe_requested_run_id() {
        let state = make_app_state();
        let mut cmd = make_cmd("prompt");
        cmd.message = "hello".to_string();
        cmd.busy_policy = "enqueue_if_busy".to_string();
        cmd.requested_run_id = "bad run id!".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert_eq!(resp["error_code"], "invalid_run_id");
    }

    #[test]
    fn prune_run_events_validates_run_id() {
        let state = make_app_state();

        let mut cmd = make_cmd("prune_run_events");
        cmd.run_id = String::new();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert_eq!(resp["error_code"], "invalid_run_id");

        let mut cmd = make_cmd("prune_run_events");
        cmd.run_id = "../escape".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert_eq!(resp["error_code"], "invalid_run_id");
    }

    #[test]
    fn prune_run_events_removes_journal_and_tolerates_missing_file() {
        let state = make_app_state();
        let run_data = state.session_manager.run_data_path("default");
        std::fs::create_dir_all(&run_data).unwrap();
        std::fs::write(run_data.join("run-prune.jsonl"), "{}").unwrap();

        let mut cmd = make_cmd("prune_run_events");
        cmd.run_id = "run-prune".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["pruned"], true);
        assert!(!run_data.join("run-prune.jsonl").exists());

        // Already gone → still pruned (NotFound is success).
        let mut cmd = make_cmd("prune_run_events");
        cmd.run_id = "run-prune".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["pruned"], true);
    }

    #[test]
    fn abort_session_on_idle_session_cancels_nothing() {
        let state = make_app_state();
        let resp = parse_response(&handle_command_internal(&state, make_cmd("abort_session")));
        assert_eq!(resp["success"], true);
        assert!(resp["data"]["active_run_id"].is_null());
        assert_eq!(resp["data"]["queued_cancelled"], 0);
        assert_eq!(resp["data"]["state"], "cancelling");
    }

    #[test]
    fn abort_session_cancels_queued_runs() {
        let state = make_app_state();
        let session = state.get_session("default").unwrap();
        session
            .read()
            .runtime
            .begin(Some("run-active"), Some("request-active"))
            .unwrap();

        let mut cmd = make_cmd("prompt");
        cmd.message = "queued".to_string();
        cmd.busy_policy = "enqueue_if_busy".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);

        let resp = parse_response(&handle_command_internal(&state, make_cmd("abort_session")));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["queued_cancelled"], 1);
        assert_eq!(resp["data"]["active_run_id"], "run-active");
    }

    #[test]
    fn retry_persistence_on_healthy_session_fails() {
        let state = make_app_state();
        let resp = parse_response(&handle_command_internal(
            &state,
            make_cmd("retry_persistence"),
        ));
        assert_eq!(resp["success"], false);
        assert_eq!(resp["error_code"], "persistence_recovery_failed");
    }

    // ── coverage batch 1: approval_decision ─────────────────────────────────

    #[test]
    fn approval_decision_unknown_request_fails() {
        let state = make_app_state();
        let mut cmd = make_cmd("approval_decision");
        cmd.mode = "approved".to_string();
        cmd.entry_id = "no-such-request".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert!(resp["error"].as_str().unwrap().contains("not pending"));
    }

    #[test]
    fn approval_decision_rejects_wrong_session_and_approves_owning_session() {
        let state = make_app_state();
        let rx = state
            .approval_gate
            .insert_pending_for_test("ap-own", "default");

        // Ownership is keyed on cmd.session_id: a decision naming a pending
        // entry owned by a *different* session is rejected.
        let _rx_other = state
            .approval_gate
            .insert_pending_for_test("ap-other", "other-session");
        let mut cmd = make_cmd_for("approval_decision", "default");
        cmd.entry_id = "ap-other".to_string();
        cmd.mode = "approved".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert!(resp["error"].as_str().unwrap().contains("does not belong"));

        // …and the owning session's decision lands on the waiting channel.
        let mut cmd = make_cmd_for("approval_decision", "default");
        cmd.entry_id = "ap-own".to_string();
        cmd.mode = "approved".to_string();
        cmd.message = "looks fine".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["approvalRequestId"], "ap-own");
        assert_eq!(resp["data"]["status"], "approved");
        let decision = rx.try_recv().expect("decision delivered");
        assert!(decision.approved);
        assert_eq!(decision.note, "looks fine");
    }

    #[test]
    fn approval_decision_rejected_and_cancelled_modes() {
        let state = make_app_state();
        for (mode, expected) in [
            ("rejected", ApprovalDecisionStatus::Rejected),
            ("cancelled", ApprovalDecisionStatus::Cancelled),
        ] {
            let request_id = format!("ap-{mode}");
            let _rx = state
                .approval_gate
                .insert_pending_for_test(&request_id, "default");
            let mut cmd = make_cmd("approval_decision");
            cmd.entry_id = request_id;
            cmd.mode = mode.to_string();
            let resp = parse_response(&handle_command_internal(&state, cmd));
            assert_eq!(resp["success"], true, "{mode}");
            let decision = _rx.try_recv().expect("decision delivered");
            assert!(!decision.approved);
            assert_eq!(decision.status, expected);
        }
    }

    // ── coverage batch 1: simple session-scoped setters ─────────────────────

    #[test]
    fn set_model_updates_session_and_broadcasts() {
        let state = make_app_state();
        let mut cmd = make_cmd("set_model");
        cmd.model_id = "mock".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["model"], "mock");
    }

    #[test]
    fn set_tools_broadcasts_new_tool_list() {
        let state = make_app_state();
        let mut cmd = make_cmd("set_tools");
        cmd.tools = vec!["read".to_string(), "write".to_string()];
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["tools"], serde_json::json!(["read", "write"]));
    }

    #[test]
    fn steer_and_set_ephemeral_and_last_assistant_text() {
        let state = make_app_state();

        let mut cmd = make_cmd("steer");
        cmd.system_prompt = "be terse".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);

        // No assistant reply yet → null text.
        let resp = parse_response(&handle_command_internal(
            &state,
            make_cmd("get_last_assistant_text"),
        ));
        assert_eq!(resp["success"], true);
        assert!(resp["data"]["text"].is_null());
    }

    #[test]
    fn set_session_name_on_unpersisted_session_broadcasts() {
        let state = make_app_state();
        let mut cmd = make_cmd("set_session_name");
        cmd.name = "my session".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        let session = state.get_session("default").unwrap();
        assert_eq!(session.read().session_name, "my session");
    }

    #[test]
    fn set_session_name_persists_to_disk_session_info() {
        let state = make_app_state();
        // Persist the session (with a session_info entry) so the update_info
        // branch fires and the name lands on disk.
        save_via(
            &state,
            "default",
            "mock",
            vec![crate::session::SessionEntry::session_info(
                serde_json::json!({"cwd": state.welcome_cwd, "model": "mock"}),
                "mock".to_string(),
                "low".to_string(),
            )],
        );

        let mut cmd = make_cmd("set_session_name");
        cmd.name = "persisted name".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        let loaded = state.session_manager.load("default").unwrap();
        assert_eq!(loaded.name, "persisted name");
    }

    #[test]
    fn cycle_model_with_no_credentialled_models_returns_empty() {
        let _home = TestHome::new();
        let state = make_app_state();
        let resp = parse_response(&handle_command_internal(&state, make_cmd("cycle_model")));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["model"], "");
        assert_eq!(resp["data"]["thinkingLevel"], "");
    }

    #[test]
    fn cycle_model_advances_to_next_credentialled_model() {
        let home = TestHome::new();
        let state = make_app_state();
        // Credential the provider of the first two catalog models so cycling
        // has somewhere to go.
        let providers: Vec<String> = {
            let registry = state.model_registry.read();
            let models = registry.all_models();
            let mut providers: Vec<String> = models.iter().map(|m| m.provider.clone()).collect();
            providers.sort();
            providers.dedup();
            providers.truncate(2);
            providers
        };
        assert!(!providers.is_empty(), "builtin catalog is never empty");
        let mut auth = serde_json::json!({});
        for provider in &providers {
            auth[provider] = serde_json::json!({"type": "api_key", "key": "k"});
        }
        let auth_path = home.auth_path();
        std::fs::create_dir_all(auth_path.parent().unwrap()).unwrap();
        std::fs::write(&auth_path, serde_json::to_string_pretty(&auth).unwrap()).unwrap();

        let resp = parse_response(&handle_command_internal(&state, make_cmd("cycle_model")));
        assert_eq!(resp["success"], true);
        let next = resp["data"]["model"].as_str().unwrap();
        assert!(!next.is_empty());
        assert_eq!(resp["data"]["isScoped"], false);
    }

    #[test]
    #[cfg(not(windows))]
    fn export_html_writes_file() {
        let _gate = EXPORT_TEST_LOCK.lock();
        let state = make_app_state();
        let resp = parse_response(&handle_command_internal(&state, make_cmd("export_html")));
        assert_eq!(resp["success"], true);
        let path = resp["data"]["path"].as_str().unwrap();
        assert!(path.contains("future_agent_export_"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn get_session_events_since_returns_empty_tail() {
        let state = make_app_state();
        let resp = parse_response(&handle_command_internal(
            &state,
            make_cmd("get_session_events_since"),
        ));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["events"], serde_json::json!([]));
    }

    // ── coverage batch 1: switch/delete session ─────────────────────────────

    #[test]
    fn switch_session_validates_and_succeeds() {
        let state = make_app_state();

        let cmd = make_cmd_for("switch_session", "");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert!(resp["error"]
            .as_str()
            .unwrap()
            .contains("No session selected"));

        let cmd = make_cmd_for("switch_session", "ghost");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert!(resp["error"].as_str().unwrap().contains("not found"));

        let cmd = make_cmd_for("switch_session", "default");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["cancelled"], false);
    }

    #[test]
    fn delete_session_requires_session_id() {
        let state = make_app_state();
        let cmd = make_cmd_for("delete_session", "");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert!(resp["error"]
            .as_str()
            .unwrap()
            .contains("No session selected to delete"));
    }

    #[test]
    fn delete_session_reports_unremovable_disk_file() {
        let state = make_app_state();
        save_via(
            &state,
            "ghost",
            "mock",
            vec![crate::session::SessionEntry::new_user(
                "user",
                serde_json::json!("x"),
            )],
        );
        // Replace the JSONL file with a directory so remove_file fails.
        let path = state.session_manager.find("ghost").expect("saved session");
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir_all(&path).unwrap();

        let cmd = make_cmd_for("delete_session", "ghost");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert_eq!(resp["error_code"], "delete_failed");
        let _ = std::fs::remove_dir_all(&path);
    }

    // ── coverage batch 1: get_fork_messages ─────────────────────────────────

    fn save_via(
        state: &AppState,
        session_id: &str,
        model: &str,
        entries: Vec<crate::session::SessionEntry>,
    ) {
        let snapshot = crate::session::Session::snapshot(
            session_id.to_string(),
            state.welcome_cwd.clone(),
            model.to_string(),
            String::new(),
            String::new(),
            entries,
        );
        state.session_manager.save(&snapshot).unwrap();
    }

    #[test]
    fn get_fork_messages_unknown_session_returns_empty() {
        let state = make_app_state();
        let cmd = make_cmd_for("get_fork_messages", "ghost");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["messages"], serde_json::json!([]));
    }

    #[test]
    fn get_fork_messages_extracts_first_text_block_only() {
        let state = make_app_state();
        let user_plain = crate::session::SessionEntry::new_user("user", serde_json::json!("plain"));
        let user_blocks = crate::session::SessionEntry::new_user(
            "user",
            serde_json::json!([
                {"type": "text", "text": "visible question"},
                {"type": "text", "text": "agent-injected attachment list"},
            ]),
        );
        let assistant =
            crate::session::SessionEntry::new_assistant(serde_json::json!("answer"), vec![]);
        save_via(
            &state,
            "fork-src",
            "mock",
            vec![user_plain, user_blocks, assistant],
        );

        let cmd = make_cmd_for("get_fork_messages", "fork-src");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        let messages = resp["data"]["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2, "only user entries are fork points");
        assert_eq!(messages[0]["content"], "plain");
        assert_eq!(messages[1]["content"], "visible question");
        assert!(messages[0]["timestamp"].is_string());
    }

    // ── coverage batch 1: new_session variants ──────────────────────────────

    #[test]
    fn new_session_generates_id_and_registers() {
        let state = make_app_state();
        let resp = parse_response(&handle_command_internal(&state, make_cmd("new_session")));
        assert_eq!(resp["success"], true);
        let new_id = resp["data"]["sessionId"].as_str().unwrap();
        assert!(!new_id.is_empty());
        assert!(state.get_session(new_id).is_some());
    }

    #[test]
    fn new_session_honors_explicit_id_cwd_model_level_and_provenance() {
        let state = make_app_state();
        let mut cmd = make_cmd_for("new_session", "ns-explicit");
        cmd.cwd = "/tmp/some-workspace/ ".to_string();
        cmd.model_id = "explicit/model".to_string();
        cmd.level = "low".to_string();
        cmd.created_by = "gui".to_string();
        cmd.source_meta = "{\"thread\":\"t1\"}".to_string();
        cmd.parent_session = "parent-1".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["sessionId"], "ns-explicit");
        let session = state.get_session("ns-explicit").unwrap();
        let sess = session.read();
        assert_eq!(sess.created_by, "gui");
        assert_eq!(sess.source_meta, serde_json::json!({"thread": "t1"}));
        assert_eq!(sess.parent_session_id, "parent-1");
        assert_eq!(sess.model, "explicit/model");
        assert_eq!(sess.thinking_level, "low");
        assert_eq!(sess.cwd, "/tmp/some-workspace");
    }

    #[test]
    fn new_session_legacy_provenance_via_custom_instructions() {
        let state = make_app_state();
        let mut cmd = make_cmd_for("new_session", "ns-legacy");
        cmd.custom_instructions =
            r#"{"createdBy":"mobile","sourceMeta":{"chat":"c1"}}"#.to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        let session = state.get_session("ns-legacy").unwrap();
        let sess = session.read();
        assert_eq!(sess.created_by, "mobile");
        assert_eq!(sess.source_meta, serde_json::json!({"chat": "c1"}));
    }

    #[test]
    fn new_session_restores_entries_from_disk() {
        let state = make_app_state();
        save_via(
            &state,
            "ns-restore",
            "mock",
            vec![
                crate::session::SessionEntry::session_info(
                    serde_json::json!({"cwd": state.welcome_cwd, "model": "disk/model-x"}),
                    "disk/model-x".to_string(),
                    "low".to_string(),
                ),
                crate::session::SessionEntry::new_user("user", serde_json::json!("restored hi")),
                crate::session::SessionEntry::new_assistant(
                    serde_json::json!("restored reply"),
                    vec![],
                ),
            ],
        );
        let cmd = make_cmd_for("new_session", "ns-restore");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        let session = state.get_session("ns-restore").unwrap();
        let sess = session.read();
        assert_eq!(sess.model, "disk/model-x");
        assert_eq!(sess.messages.read().len(), 2);
    }

    // ── coverage batch 1: get_session_entries ───────────────────────────────

    #[test]
    fn get_session_entries_empty_for_unknown_live_session() {
        let state = make_app_state();
        // "default" is live but has nothing on disk.
        let resp = parse_response(&handle_command_internal(
            &state,
            make_cmd("get_session_entries"),
        ));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["entries"], serde_json::json!([]));
    }

    #[test]
    fn get_session_entries_renders_roles_and_run_stats() {
        let state = make_app_state();
        let info_old = crate::session::SessionEntry::session_info(
            serde_json::json!({"cwd": "old", "model": "mock", "session_name": "old"}),
            "mock".to_string(),
            "low".to_string(),
        );
        let user = crate::session::SessionEntry::new_user(
            "user",
            serde_json::json!([
                {"type": "text", "text": "question"},
                {"type": "text", "text": "attachment paths"},
            ]),
        );
        let mut assistant = crate::session::SessionEntry::new_assistant(
            serde_json::json!([{"type": "text", "text": "answer"}]),
            vec![],
        );
        assistant.thinking = "deep thought".to_string();
        let tool = crate::session::SessionEntry::new_tool("call-1", "tool output");
        let terminal = crate::session::SessionEntry::run_terminal(
            "run-1",
            crate::session::RUN_STATE_COMPLETED,
            42,
            1500,
            None,
        );
        let info_new = crate::session::SessionEntry::session_info(
            serde_json::json!({"cwd": "new", "model": "mock", "session_name": "fresh"}),
            "mock".to_string(),
            "xhigh".to_string(),
        );
        save_via(
            &state,
            "default",
            "mock",
            vec![info_old, user, assistant, tool, terminal, info_new],
        );

        let resp = parse_response(&handle_command_internal(
            &state,
            make_cmd("get_session_entries"),
        ));
        assert_eq!(resp["success"], true);
        let entries = resp["data"]["entries"].as_array().unwrap();
        // session_info (deduped to one), user, assistant, tool.
        assert_eq!(entries.len(), 4);
        let info = &entries[0];
        assert_eq!(info["content"]["session_name"], "fresh");
        let user_entry = &entries[1];
        assert_eq!(user_entry["content"], "question");
        let assistant_entry = &entries[2];
        assert_eq!(assistant_entry["content"], "answer");
        assert_eq!(assistant_entry["thinking"], "deep thought");
        assert_eq!(assistant_entry["output_tokens"], 42);
        assert_eq!(assistant_entry["duration_ms"], 1500);
        let tool_entry = &entries[3];
        assert_eq!(tool_entry["content"], "tool output");
    }

    // ── coverage batch 1: fork / clone ──────────────────────────────────────

    #[test]
    fn fork_requires_entry_id() {
        let state = make_app_state();
        let resp = parse_response(&handle_command_internal(&state, make_cmd("fork")));
        assert_eq!(resp["success"], false);
        assert!(resp["error"]
            .as_str()
            .unwrap()
            .contains("No message selected"));
    }

    #[test]
    fn fork_fails_when_parent_not_on_disk() {
        let state = make_app_state();
        let mut cmd = make_cmd("fork");
        cmd.entry_id = "entry-1".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert!(resp["error"]
            .as_str()
            .unwrap()
            .contains("not found on disk"));
    }

    #[test]
    fn fork_creates_new_session_from_entry_point() {
        let state = make_app_state();
        let user = crate::session::SessionEntry::new_user("user", serde_json::json!("fork here"));
        let entry_id = user.id.clone();
        save_via(
            &state,
            "default",
            "mock",
            vec![
                user,
                crate::session::SessionEntry::new_assistant(serde_json::json!("reply"), vec![]),
            ],
        );

        let mut cmd = make_cmd("fork");
        cmd.entry_id = entry_id;
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        let fork_id = resp["data"]["sessionId"].as_str().unwrap().to_string();
        assert!(!fork_id.is_empty());
        assert!(state.get_session(&fork_id).is_some());
        // Forked history was loaded into memory so a later save cannot
        // truncate it.
        let session = state.get_session(&fork_id).unwrap();
        assert!(!session.read().messages.read().is_empty());
    }

    #[test]
    fn fork_from_explicit_parent_session() {
        let state = make_app_state();
        let user = crate::session::SessionEntry::new_user("user", serde_json::json!("parent msg"));
        let entry_id = user.id.clone();
        save_via(&state, "parent-disk", "mock", vec![user]);

        let mut cmd = make_cmd("fork");
        cmd.entry_id = entry_id;
        cmd.parent_session = "parent-disk".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        let fork_id = resp["data"]["sessionId"].as_str().unwrap();
        assert!(state.get_session(fork_id).is_some());
    }

    #[test]
    fn clone_rejects_empty_session() {
        let state = make_app_state();
        let resp = parse_response(&handle_command_internal(&state, make_cmd("clone")));
        assert_eq!(resp["success"], false);
        assert!(resp["error"].as_str().unwrap().contains("Nothing to clone"));
    }

    #[test]
    fn clone_fails_when_disk_session_missing() {
        let state = make_app_state();
        {
            let session = state.get_session("default").unwrap();
            session
                .read()
                .messages
                .write()
                .push(crate::types::AgentMessage::new_user(
                    "user",
                    serde_json::json!("in-memory only"),
                ));
        }
        let resp = parse_response(&handle_command_internal(&state, make_cmd("clone")));
        assert_eq!(resp["success"], false);
        assert!(resp["error"]
            .as_str()
            .unwrap()
            .contains("not found on disk"));
    }

    #[test]
    fn clone_rejects_disk_session_with_idless_last_entry() {
        let state = make_app_state();
        {
            let session = state.get_session("default").unwrap();
            session
                .read()
                .messages
                .write()
                .push(crate::types::AgentMessage::new_user(
                    "user",
                    serde_json::json!("in-memory only"),
                ));
        }
        // A disk session whose last entry carries no id -> the clone leaf id
        // resolves empty and hits the "no messages found" arm.
        let mut entry = crate::session::SessionEntry::new_user("user", serde_json::json!("legacy"));
        entry.id = String::new();
        save_via(&state, "default", "mock", vec![entry]);
        let resp = parse_response(&handle_command_internal(&state, make_cmd("clone")));
        assert_eq!(resp["success"], false);
        assert!(resp["error"]
            .as_str()
            .unwrap()
            .contains("no messages found"));
    }

    #[test]
    fn clone_succeeds_from_leaf_entry() {
        let state = make_app_state();
        {
            let session = state.get_session("default").unwrap();
            session
                .read()
                .messages
                .write()
                .push(crate::types::AgentMessage::new_user(
                    "user",
                    serde_json::json!("clone me"),
                ));
        }
        save_via(
            &state,
            "default",
            "mock",
            vec![
                // A session_info entry with a model makes the forked model
                // non-empty, so clone also syncs it into the new session's
                // agent loop.
                crate::session::SessionEntry::session_info(
                    serde_json::json!({"cwd": "/tmp", "model": "mock"}),
                    "mock".to_string(),
                    "high".to_string(),
                ),
                crate::session::SessionEntry::new_user("user", serde_json::json!("clone me")),
                crate::session::SessionEntry::new_assistant(serde_json::json!("reply"), vec![]),
            ],
        );
        let resp = parse_response(&handle_command_internal(&state, make_cmd("clone")));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["cancelled"], false);
    }

    // ── coverage batch 1: reload_config ─────────────────────────────────────

    #[test]
    fn reload_config_without_context_file_returns_empty_list() {
        let state = make_app_state();
        let resp = parse_response(&handle_command_internal(&state, make_cmd("reload_config")));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["contextFiles"], serde_json::json!([]));
    }

    #[test]
    fn reload_config_picks_up_context_file() {
        let state = make_app_state();
        let cwd = state.welcome_cwd.clone();
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::write(std::path::Path::new(&cwd).join("CLAUDE.md"), "# context").unwrap();

        let resp = parse_response(&handle_command_internal(&state, make_cmd("reload_config")));
        assert_eq!(resp["success"], true);
        assert_eq!(
            resp["data"]["contextFiles"],
            serde_json::json!(["CLAUDE.md"])
        );
        assert_eq!(
            state.welcome_context.read().as_slice(),
            &["# context".to_string()]
        );
        let _ = std::fs::remove_dir_all(&cwd);
    }

    // ── coverage batch 2: error-path arms ───────────────────────────────────

    #[test]
    fn session_scoped_command_requires_known_session() {
        let state = make_app_state();
        let resp = parse_response(&handle_command_internal(
            &state,
            make_cmd_for("get_messages", "ghost"),
        ));
        assert_eq!(resp["success"], false);
        assert!(resp["error"]
            .as_str()
            .unwrap()
            .contains("session not found"));
    }

    #[test]
    fn prompt_reject_if_busy_reports_active_run_details() {
        let state = make_app_state();
        let session = state.get_session("default").unwrap();
        session
            .read()
            .runtime
            .begin(Some("run-active"), Some("request-active"))
            .unwrap();

        let mut cmd = make_cmd("prompt");
        cmd.message = "rejected".to_string(); // default busy policy: reject_if_busy
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert_eq!(resp["error_code"], "busy");
        assert_eq!(resp["error_data"]["active_run_id"], "run-active");
    }

    #[test]
    fn prompt_supersede_replaces_queued_run() {
        let state = make_app_state();
        let session = state.get_session("default").unwrap();
        session
            .read()
            .runtime
            .begin(Some("run-active"), Some("request-active"))
            .unwrap();
        // A queued entry makes the scheduler busy.
        let mut cmd = make_cmd("prompt");
        cmd.message = "queued".to_string();
        cmd.busy_policy = "enqueue_if_busy".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);

        let mut cmd = make_cmd("prompt");
        cmd.message = "supersede".to_string();
        cmd.busy_policy = "supersede_session".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        // The superseded queued run is gone; the new request is queued behind
        // the still-active run.
        let queued = session.read().scheduler.queued().clone();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].payload["message"], "supersede");
    }

    #[test]
    fn prompt_duplicate_run_id_maps_to_scheduler_error() {
        let state = make_app_state();
        // Plant a journal for run-dupe so the id is rejected as reused.
        let run_data = state.session_manager.run_data_path("default");
        std::fs::create_dir_all(&run_data).unwrap();
        std::fs::write(run_data.join("run-dupe.jsonl"), "").unwrap();

        let mut cmd = make_cmd("prompt");
        cmd.message = "dupe".to_string();
        cmd.busy_policy = "enqueue_if_busy".to_string();
        cmd.requested_run_id = "run-dupe".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert_eq!(resp["error_code"], "scheduler_error");
    }

    #[test]
    fn prompt_reports_attachment_unavailable() {
        let state = make_app_state();
        let mut cmd = make_cmd("prompt");
        cmd.message = "with attachment".to_string();
        cmd.busy_policy = "enqueue_if_busy".to_string();
        cmd.attachments = vec![crate::types::Attachment {
            path: "/definitely/not/a/real/file.pdf".to_string(),
            kind: "file".to_string(),
            ..Default::default()
        }];
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert_eq!(resp["error_code"], "attachment_unavailable");
        assert_eq!(
            resp["error_data"]["path"],
            "/definitely/not/a/real/file.pdf"
        );
    }

    #[test]
    fn prompt_reports_persistence_unavailable() {
        // A regular file where the run-events dir should be makes journal
        // configuration fail, which enqueue reports as persistence_unavailable.
        let dir = test_session_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".run-events"), "not a dir").unwrap();
        let state =
            make_app_state_with(dir, Arc::new(crate::runtime::GlobalQueueBudget::defaults()));

        let mut cmd = make_cmd("prompt");
        cmd.message = "hello".to_string();
        cmd.busy_policy = "enqueue_if_busy".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert_eq!(resp["error_code"], "persistence_unavailable");
    }

    #[test]
    fn prompt_reports_session_queue_full() {
        let state = make_app_state();
        let session = state.get_session("default").unwrap();
        session
            .read()
            .runtime
            .begin(Some("run-active"), Some("request-active"))
            .unwrap();

        // Fill the session queue to capacity (128).
        for i in 0..crate::runtime::DEFAULT_SESSION_QUEUE_CAPACITY {
            let mut cmd = make_cmd("prompt");
            cmd.message = format!("queued {i}");
            cmd.busy_policy = "enqueue_if_busy".to_string();
            let resp = parse_response(&handle_command_internal(&state, cmd));
            assert_eq!(resp["success"], true, "enqueue {i}");
        }
        let mut cmd = make_cmd("prompt");
        cmd.message = "one too many".to_string();
        cmd.busy_policy = "enqueue_if_busy".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert_eq!(resp["error_code"], "queue_full");
        assert_eq!(
            resp["error_data"]["limit"],
            crate::runtime::DEFAULT_SESSION_QUEUE_CAPACITY as u64
        );
    }

    #[test]
    fn prompt_reports_request_too_large() {
        let state = make_app_state();
        let mut cmd = make_cmd("prompt");
        cmd.message = "x".repeat(crate::runtime::DEFAULT_REQUEST_BYTES + 1);
        cmd.busy_policy = "enqueue_if_busy".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert_eq!(resp["error_code"], "attachment_too_large");
    }

    #[test]
    fn prompt_reports_global_queue_full() {
        let state = make_app_state_with(
            test_session_dir(),
            Arc::new(crate::runtime::GlobalQueueBudget::new(0, usize::MAX)),
        );
        let mut cmd = make_cmd("prompt");
        cmd.message = "hello".to_string();
        cmd.busy_policy = "enqueue_if_busy".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert_eq!(resp["error_code"], "queue_full");
    }

    #[test]
    fn prompt_reports_global_queue_bytes_exceeded() {
        let state = make_app_state_with(
            test_session_dir(),
            Arc::new(crate::runtime::GlobalQueueBudget::new(usize::MAX, 1)),
        );
        let mut cmd = make_cmd("prompt");
        cmd.message = "more than one byte".to_string();
        cmd.busy_policy = "enqueue_if_busy".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert_eq!(resp["error_code"], "attachment_too_large");
    }

    #[test]
    fn cancel_queued_run_requires_run_id() {
        let state = make_app_state();
        let resp = parse_response(&handle_command_internal(
            &state,
            make_cmd("cancel_queued_run"),
        ));
        assert_eq!(resp["success"], false);
        assert_eq!(resp["error_code"], "run_not_queued");
    }

    #[test]
    fn cancel_queued_run_unknown_run_errors() {
        let state = make_app_state();
        let mut cmd = make_cmd("cancel_queued_run");
        cmd.run_id = "run-ghost".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert_eq!(resp["error_code"], "run_not_queued");
    }

    #[test]
    fn prune_run_events_rejects_active_run() {
        let state = make_app_state();
        let session = state.get_session("default").unwrap();
        session
            .read()
            .runtime
            .begin(Some("run-blocker"), Some("request-blocker"))
            .unwrap();
        let mut cmd = make_cmd("prompt");
        cmd.message = "queued".to_string();
        cmd.busy_policy = "enqueue_if_busy".to_string();
        cmd.requested_run_id = "run-scheduled".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        session.read().scheduler.start_next(1).unwrap();

        let mut cmd = make_cmd("prune_run_events");
        cmd.run_id = "run-scheduled".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert_eq!(resp["error_code"], "run_active");
    }

    #[test]
    fn prune_run_events_reports_io_error() {
        let state = make_app_state();
        // A directory where the journal file should be makes remove_file fail.
        let run_data = state.session_manager.run_data_path("default");
        std::fs::create_dir_all(run_data.join("run-dir.jsonl")).unwrap();

        let mut cmd = make_cmd("prune_run_events");
        cmd.run_id = "run-dir".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert_eq!(resp["error_code"], "prune_failed");
        let _ = std::fs::remove_dir_all(run_data.join("run-dir.jsonl"));
    }

    #[test]
    fn retry_persistence_recovers_degraded_run() {
        let state = make_app_state();
        // The transcript must exist for the recovery append to land.
        save_via(
            &state,
            "default",
            "mock",
            vec![crate::session::SessionEntry::session_info(
                serde_json::json!({"cwd": state.welcome_cwd, "model": "mock"}),
                "mock".to_string(),
                "low".to_string(),
            )],
        );
        let session = state.get_session("default").unwrap();
        let lease = session
            .read()
            .runtime
            .begin(Some("run-degraded"), Some("request-degraded"))
            .unwrap();
        assert!(session
            .read()
            .runtime
            .mark_persistence_degraded(&lease, "disk full"));

        let resp = parse_response(&handle_command_internal(
            &state,
            make_cmd("retry_persistence"),
        ));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["run_id"], "run-degraded");
        assert_eq!(resp["data"]["state"], "interrupted");
        assert_eq!(resp["data"]["recovered"], true);
    }

    #[test]
    fn get_session_events_since_returns_events_then_journal_error() {
        let state = make_app_state();
        // Broadcast a session-level event so the journal has content.
        let mut cmd = make_cmd("set_model");
        cmd.model_id = "mock".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);

        let mut cmd = make_cmd("get_session_events_since");
        cmd.since_idx = -1;
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        let events = resp["data"]["events"].as_array().unwrap();
        assert!(!events.is_empty());
        assert_eq!(events[0]["type"], "model_changed");

        // A directory where the journal file should be breaks reads.
        let journal = state
            .session_manager
            .run_data_path("default")
            .join("_session.jsonl");
        std::fs::remove_file(&journal).unwrap();
        std::fs::create_dir_all(&journal).unwrap();
        let resp = parse_response(&handle_command_internal(
            &state,
            make_cmd("get_session_events_since"),
        ));
        assert_eq!(resp["success"], false);
        let _ = std::fs::remove_dir_all(&journal);
    }

    #[test]
    fn set_model_fails_while_loop_is_locked() {
        let state = make_app_state();
        let session = state.get_session("default").unwrap();
        let agent_loop = session.read().agent_loop.clone();
        let _guard = agent_loop.try_write().unwrap();

        let mut cmd = make_cmd("set_model");
        cmd.model_id = "mock".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert!(resp["error"].as_str().unwrap().contains("busy"));
    }

    #[test]
    fn shell_fails_with_missing_cwd() {
        let state = make_app_state(); // test_workspace() is never created
        let mut cmd = make_cmd("shell");
        cmd.command = "echo hi".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
    }

    #[test]
    fn set_session_name_survives_persist_error() {
        let state = make_app_state();
        save_via(
            &state,
            "default",
            "mock",
            vec![crate::session::SessionEntry::session_info(
                serde_json::json!({"cwd": state.welcome_cwd, "model": "mock"}),
                "mock".to_string(),
                "low".to_string(),
            )],
        );
        // Break the on-disk file so update_info fails (logged, still ok).
        let path = state.session_manager.find("default").unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir_all(&path).unwrap();

        let mut cmd = make_cmd("set_session_name");
        cmd.name = "still works".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn set_cwd_survives_persist_error() {
        let state = make_app_state();
        save_via(
            &state,
            "default",
            "mock",
            vec![crate::session::SessionEntry::session_info(
                serde_json::json!({"cwd": state.welcome_cwd, "model": "mock"}),
                "mock".to_string(),
                "low".to_string(),
            )],
        );
        let path = state.session_manager.find("default").unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir_all(&path).unwrap();

        let mut cmd = make_cmd("set_cwd");
        cmd.cwd = "/tmp/new-cwd".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["cwd"], "/tmp/new-cwd");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn cycle_model_fails_while_loop_is_locked() {
        let home = TestHome::new();
        let state = make_app_state();
        let providers: Vec<String> = {
            let registry = state.model_registry.read();
            let mut providers: Vec<String> = registry
                .all_models()
                .iter()
                .map(|m| m.provider.clone())
                .collect();
            providers.sort();
            providers.dedup();
            providers.truncate(2);
            providers
        };
        let mut auth = serde_json::json!({});
        for provider in &providers {
            auth[provider] = serde_json::json!({"type": "api_key", "key": "k"});
        }
        let auth_path = home.auth_path();
        std::fs::create_dir_all(auth_path.parent().unwrap()).unwrap();
        std::fs::write(&auth_path, serde_json::to_string_pretty(&auth).unwrap()).unwrap();

        let session = state.get_session("default").unwrap();
        let agent_loop = session.read().agent_loop.clone();
        let _guard = agent_loop.try_write().unwrap();
        let resp = parse_response(&handle_command_internal(&state, make_cmd("cycle_model")));
        assert_eq!(resp["success"], false);
    }

    #[test]
    fn set_sandbox_policy_applies_tier() {
        let state = make_app_state();
        let mut cmd = make_cmd("set_sandbox_policy");
        cmd.sandbox_policy = Some(crate::sandbox::SandboxPolicy {
            tier: crate::sandbox::SandboxTier::Off,
        });
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["tier"], "off");
        assert!(resp["data"]["sandboxAvailable"].is_boolean());
    }

    #[test]
    fn list_models_sorts_and_includes_builtin_providers() {
        let home = TestHome::new();
        let state = make_app_state();
        let providers: Vec<String> = {
            let registry = state.model_registry.read();
            let mut providers: Vec<String> = registry
                .all_models()
                .iter()
                .map(|m| m.provider.clone())
                .collect();
            providers.sort();
            providers.dedup();
            providers.truncate(2);
            providers
        };
        assert!(providers.len() >= 2, "catalog has multiple providers");
        let mut auth = serde_json::json!({});
        for provider in &providers {
            auth[provider] = serde_json::json!({"type": "api_key", "key": "k"});
        }
        let auth_path = home.auth_path();
        std::fs::create_dir_all(auth_path.parent().unwrap()).unwrap();
        std::fs::write(&auth_path, serde_json::to_string_pretty(&auth).unwrap()).unwrap();

        let mut cmd = make_cmd("list_models");
        cmd.include_builtin_providers = true;
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        let models = resp["data"]["models"].as_array().unwrap();
        assert!(models.len() >= 2);
        assert!(models.iter().all(|m| m["label"].is_string()));
        assert!(resp["data"]["builtinProviders"].is_object());
    }

    #[test]
    fn set_auth_reports_mutation_error() {
        let home = TestHome::new();
        let state = make_app_state();
        let auth_path = home.auth_path();
        std::fs::create_dir_all(auth_path.parent().unwrap()).unwrap();
        std::fs::write(&auth_path, "{corrupt").unwrap();

        let mut cmd = make_cmd("set_auth");
        cmd.auth_update = Some(crate::config::providers::AuthMutation {
            provider: "custom".to_string(),
            key: Some("k".to_string()),
            ..Default::default()
        });
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
    }

    #[test]
    fn list_models_uses_id_label_for_unnamed_model() {
        let _home = TestHome::new();
        let state = make_app_state();
        // The file loaders normalize an absent name to the model id, so a
        // genuinely empty name only exists via the verbatim test seam.
        state
            .model_registry
            .write()
            .test_insert(crate::models::Model {
                id: "unnamed-model".to_string(),
                name: String::new(),
                provider: "custom".to_string(),
                api_key: "k".to_string(),
                output: vec!["text".to_string()],
                ..Default::default()
            });
        let resp = parse_response(&handle_command_internal(&state, make_cmd("list_models")));
        let models = resp["data"]["models"].as_array().unwrap();
        let model = models
            .iter()
            .find(|m| m["id"] == "unnamed-model")
            .expect("unnamed model listed");
        assert_eq!(model["label"], "unnamed-model");
    }

    #[test]
    fn upsert_provider_rejects_no_change_and_builtin_ids() {
        let _home = TestHome::new();
        let state = make_app_state();

        // id only, no change fields.
        let mut cmd = make_cmd("upsert_provider");
        cmd.provider_config = Some(crate::config::providers::ProviderUpsertSpec {
            id: "custom".to_string(),
            ..Default::default()
        });
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert!(resp["error"].as_str().unwrap().contains("no change"));

        // A pure base-URL override defines no custom provider (name/api/models/
        // key all absent), so it is legitimately allowed even under a custom
        // id. This also exercises every short-circuit arm of the
        // defines-custom-provider guard (all four operands evaluate false).
        let mut cmd = make_cmd("upsert_provider");
        cmd.provider_config = Some(crate::config::providers::ProviderUpsertSpec {
            id: "custom".to_string(),
            base_url: Some("https://override.example.com".to_string()),
            ..Default::default()
        });
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);

        // A built-in id cannot be redefined with a name.
        let builtin = {
            let registry = state.model_registry.read();
            let mut ids: Vec<String> = registry.builtin_provider_ids().into_iter().collect();
            ids.sort();
            ids.first().expect("builtin catalog").clone()
        };
        let mut cmd = make_cmd("upsert_provider");
        cmd.provider_config = Some(crate::config::providers::ProviderUpsertSpec {
            id: builtin.clone(),
            name: Some("Hijacked".to_string()),
            ..Default::default()
        });
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert!(resp["error"].as_str().unwrap().contains("reserved"));
    }

    #[test]
    fn delete_provider_rejects_builtin_and_reports_storage_errors() {
        let home = TestHome::new();
        let state = make_app_state();

        let builtin = {
            let registry = state.model_registry.read();
            let mut ids: Vec<String> = registry.builtin_provider_ids().into_iter().collect();
            ids.sort();
            ids.first().expect("builtin catalog").clone()
        };
        let mut cmd = make_cmd("delete_provider");
        cmd.provider_config = Some(crate::config::providers::ProviderUpsertSpec {
            id: builtin,
            ..Default::default()
        });
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert!(resp["error"].as_str().unwrap().contains("reserved"));

        // Corrupt models.json → the delete write path reports an error.
        let models_path = home.models_path();
        std::fs::create_dir_all(models_path.parent().unwrap()).unwrap();
        std::fs::write(&models_path, "{corrupt").unwrap();
        let mut cmd = make_cmd("delete_provider");
        cmd.provider_config = Some(crate::config::providers::ProviderUpsertSpec {
            id: "custom-provider".to_string(),
            ..Default::default()
        });
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
    }

    #[test]
    fn list_sessions_reports_enumeration_error() {
        // A regular file where the session dir should be breaks read_dir.
        let dir = test_session_dir();
        std::fs::create_dir_all(dir.parent().unwrap()).unwrap();
        std::fs::write(&dir, "not a dir").unwrap();
        let state =
            make_app_state_with(dir, Arc::new(crate::runtime::GlobalQueueBudget::defaults()));

        let resp = parse_response(&handle_command_internal(&state, make_cmd("list_sessions")));
        assert_eq!(resp["success"], false);
        assert!(resp["error"]
            .as_str()
            .unwrap()
            .contains("enumerate sessions"));

        let resp = parse_response(&handle_command_internal(
            &state,
            make_cmd("list_session_ids"),
        ));
        assert_eq!(resp["success"], false);
        assert!(resp["error"]
            .as_str()
            .unwrap()
            .contains("enumerate session files"));
    }

    #[test]
    fn delete_session_with_active_run_returns_deleting() {
        let state = make_app_state();
        let session = state.get_session("default").unwrap();
        session
            .read()
            .runtime
            .begin(Some("run-active"), Some("request-active"))
            .unwrap();

        let resp = parse_response(&handle_command_internal(&state, make_cmd("delete_session")));
        assert_eq!(resp["success"], false);
        assert_eq!(resp["error_code"], "deleting");
        assert_eq!(resp["error_data"]["active_run_id"], "run-active");
        assert_eq!(resp["error_data"]["retryable"], true);
        // The session stays live behind the deletion fence.
        assert!(state.get_session("default").is_some());
    }

    #[test]
    fn get_commands_lists_discovered_skills() {
        let home = TestHome::new();
        let skill_dir = home.path().join(".future/agent/skills/cov-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: cov-skill\ndescription: coverage fixture\n---\n# body\n",
        )
        .unwrap();
        // A second skill guarantees the sort_by comparator runs (it is
        // skipped for a 0/1-element list).
        let skill_dir_b = home.path().join(".future/agent/skills/aaa-skill");
        std::fs::create_dir_all(&skill_dir_b).unwrap();
        std::fs::write(
            skill_dir_b.join("SKILL.md"),
            "---\nname: aaa-skill\ndescription: sorts before cov-skill\n---\n# body\n",
        )
        .unwrap();
        crate::skills::invalidate_skills_cache();

        let state = make_app_state();
        let resp = parse_response(&handle_command_internal(&state, make_cmd("get_commands")));
        assert_eq!(resp["success"], true);
        let commands = resp["data"]["commands"].as_array().unwrap();
        assert!(commands.iter().any(|c| c["name"] == "cov-skill"));
        // aaa-skill sorts before cov-skill, proving the comparator ran.
        let names: Vec<&str> = commands
            .iter()
            .filter_map(|c| c["name"].as_str())
            .filter(|n| n.ends_with("-skill"))
            .collect();
        assert!(
            names.windows(2).all(|w| w[0] <= w[1]),
            "{names:?} not sorted"
        );
        crate::skills::invalidate_skills_cache();
    }

    #[test]
    fn new_session_applies_user_settings() {
        let home = TestHome::new();
        let settings_path = home.settings_path();
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        std::fs::write(&settings_path, r#"{"defaultPermissionLevel": "workspace"}"#).unwrap();

        let state = make_app_state();
        let cmd = make_cmd_for("new_session", "ns-settings");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        let session = state.get_session("ns-settings").unwrap();
        assert_eq!(session.read().get_permission_level(), "workspace");
    }

    #[test]
    fn new_session_restores_entries_without_disk_model() {
        let state = make_app_state();
        // No session_info entry → disk model resolves empty and the effective
        // model falls back to the session's default.
        save_via(
            &state,
            "ns-nomodel",
            "mock",
            vec![crate::session::SessionEntry::new_user(
                "user",
                serde_json::json!("hi"),
            )],
        );
        let cmd = make_cmd_for("new_session", "ns-nomodel");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        let session = state.get_session("ns-nomodel").unwrap();
        assert_eq!(session.read().messages.read().len(), 1);
    }

    #[test]
    fn get_session_entries_handles_empty_tool_and_rich_meta() {
        let state = make_app_state();
        let mut assistant = crate::session::SessionEntry::new_assistant(
            serde_json::json!("with tools"),
            vec![crate::types::ToolCall {
                id: "call-1".to_string(),
                call_type: "function".to_string(),
                function: crate::types::ToolCallFn {
                    name: "read".to_string(),
                    arguments: serde_json::json!({"path": "x"}),
                },
            }],
        );
        assistant.meta = Some(serde_json::json!({"attachments": []}));
        let empty_tool = crate::session::SessionEntry::new_tool("call-1", "");
        save_via(&state, "default", "mock", vec![assistant, empty_tool]);

        let resp = parse_response(&handle_command_internal(
            &state,
            make_cmd("get_session_entries"),
        ));
        assert_eq!(resp["success"], true);
        let entries = resp["data"]["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries[0]["tool_calls"].is_array());
        assert!(entries[0]["meta"].is_object());
        assert_eq!(entries[1]["content"], "");
    }

    #[test]
    fn fork_inherits_parent_disk_model() {
        let state = make_app_state();
        let user = crate::session::SessionEntry::new_user("user", serde_json::json!("fork me"));
        let entry_id = user.id.clone();
        save_via(
            &state,
            "default",
            "mock",
            vec![
                crate::session::SessionEntry::session_info(
                    serde_json::json!({"cwd": state.welcome_cwd, "model": "disk/model-y"}),
                    "disk/model-y".to_string(),
                    "low".to_string(),
                ),
                user,
            ],
        );

        let mut cmd = make_cmd("fork");
        cmd.entry_id = entry_id;
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        let fork_id = resp["data"]["sessionId"].as_str().unwrap();
        let fork = state.get_session(fork_id).unwrap();
        assert_eq!(fork.read().model, "disk/model-y");
    }

    #[cfg(unix)]
    #[test]
    fn fork_and_clone_report_save_errors() {
        let state = make_app_state();
        let user = crate::session::SessionEntry::new_user("user", serde_json::json!("fork me"));
        let entry_id = user.id.clone();
        save_via(&state, "default", "mock", vec![user]);
        {
            let session = state.get_session("default").unwrap();
            session
                .read()
                .messages
                .write()
                .push(crate::types::AgentMessage::new_user(
                    "user",
                    serde_json::json!("clone me"),
                ));
        }
        // Read-only session dir → the forked/clone save fails. (Windows ignores
        // the readonly bit on directories, hence cfg(unix).)
        let dir = state.session_manager.run_data_path("default");
        let sess_dir = dir.parent().unwrap().parent().unwrap().to_path_buf();
        let mut perms = std::fs::metadata(&sess_dir).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&sess_dir, perms.clone()).unwrap();

        let mut cmd = make_cmd("fork");
        cmd.entry_id = entry_id;
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert!(resp["error"]
            .as_str()
            .unwrap()
            .contains("failed to save forked"));

        let resp = parse_response(&handle_command_internal(&state, make_cmd("clone")));
        assert_eq!(resp["success"], false);
        assert!(resp["error"]
            .as_str()
            .unwrap()
            .contains("failed to save cloned"));

        let mut perms = std::fs::metadata(&sess_dir).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        std::fs::set_permissions(&sess_dir, perms).unwrap();
    }

    #[test]
    fn empty_provider_yields_an_empty_stream() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            use tokio_stream::StreamExt;
            let provider = EmptyProvider;
            let mut stream = provider
                .stream_chat("mock".to_string(), vec![], vec![], String::new())
                .await
                .unwrap();
            assert!(stream.next().await.is_none());
        });
    }

    // ── coverage batch 24: per-line residuals ─────────────────────────────

    #[test]
    fn prompt_reports_session_queue_bytes_exceeded() {
        let state = make_app_state();
        // A tiny per-session queue-byte limit (smaller than the request limit,
        // so the payload passes RequestTooLarge and trips QueueBytesExceeded).
        let small = Arc::new(crate::runtime::InMemoryRunQueue::with_limits_and_global(
            "default",
            1,
            crate::runtime::DEFAULT_SESSION_QUEUE_CAPACITY,
            8,
            1024,
            256,
            Arc::new(crate::runtime::GlobalQueueBudget::defaults()),
        ));
        state.get_session("default").unwrap().write().scheduler = small;

        let mut cmd = make_cmd("prompt");
        cmd.message = "hello there".to_string();
        cmd.busy_policy = "enqueue_if_busy".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert_eq!(resp["error_code"], "attachment_too_large");
    }

    #[test]
    fn prompt_reports_busy_configuration_error() {
        let state = make_app_state();
        let agent_loop = state
            .get_session("default")
            .unwrap()
            .read()
            .agent_loop
            .clone();
        let _guard = agent_loop.try_write().unwrap();

        let mut cmd = make_cmd("prompt");
        cmd.message = "hi".to_string();
        cmd.busy_policy = "enqueue_if_busy".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert!(resp["error"]
            .as_str()
            .unwrap()
            .contains("run configuration is busy"));
    }

    #[test]
    fn get_events_since_returns_projection_over_truncated_ring() {
        let state = make_app_state();
        {
            let session = state.get_session("default").unwrap();
            let mut sess = session.write();
            // No journal configured → a cursor older than the in-memory ring
            // returns a compressed projection instead of a partial tail.
            sess.broadcaster = Arc::new(SseBroadcaster::new());
            sess.broadcaster.start_run("run-ring".to_string(), 1);
            for i in 0..2100 {
                sess.broadcaster
                    .broadcast(SseEvent::new("text_chunk", serde_json::json!({"i": i})));
            }
        }
        let mut cmd = make_cmd("get_events_since");
        cmd.run_id = "run-ring".to_string();
        cmd.since_idx = 0;
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert!(!resp["data"]["projection"].is_null());
    }

    #[test]
    fn export_html_reports_write_failure() {
        let _gate = EXPORT_TEST_LOCK.lock();
        let _guard = ExportDirGuard::new(std::path::PathBuf::from(
            "/definitely/not/a/real/export/dir",
        ));
        let state = make_app_state();
        let resp = parse_response(&handle_command_internal(&state, make_cmd("export_html")));
        assert_eq!(resp["success"], false);
        assert!(resp["error"]
            .as_str()
            .unwrap()
            .contains("failed to write file"));
    }

    #[test]
    fn set_cwd_persists_successfully_on_disk_session() {
        let state = make_app_state();
        save_via(
            &state,
            "default",
            "mock",
            vec![crate::session::SessionEntry::session_info(
                serde_json::json!({"cwd": state.welcome_cwd, "model": "mock"}),
                "mock".to_string(),
                "low".to_string(),
            )],
        );
        let mut cmd = make_cmd("set_cwd");
        cmd.cwd = "/tmp/persisted-cwd".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["cwd"], "/tmp/persisted-cwd");
    }

    #[test]
    fn delete_session_reports_close_failure() {
        let state = make_app_state();
        state
            .get_session("default")
            .unwrap()
            .read()
            .persistence
            .fail_next_close();
        let resp = parse_response(&handle_command_internal(&state, make_cmd("delete_session")));
        assert_eq!(resp["success"], false);
        assert_eq!(resp["error_code"], "delete_failed");
    }

    #[test]
    fn new_session_legacy_provenance_invalid_json() {
        let state = make_app_state();
        let mut cmd = make_cmd_for("new_session", "ns-bad-json");
        cmd.custom_instructions = "not valid json".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        let session = state.get_session("ns-bad-json").unwrap();
        assert_eq!(session.read().created_by, "tui");
    }

    #[test]
    fn new_session_legacy_provenance_with_typed_source_meta() {
        let state = make_app_state();
        let mut cmd = make_cmd_for("new_session", "ns-typed-meta");
        cmd.source_meta = "{\"chat\":\"c1\"}".to_string();
        cmd.custom_instructions = "{\"createdBy\":\"legacy\"}".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        let session = state.get_session("ns-typed-meta").unwrap();
        assert_eq!(session.read().created_by, "legacy");
    }

    #[test]
    fn get_session_entries_skips_orphan_terminal_marker() {
        let state = make_app_state();
        // A run_terminal with no preceding assistant marker (orphan terminal)
        // must not fabricate run stats.
        save_via(
            &state,
            "default",
            "mock",
            vec![
                crate::session::SessionEntry::run_terminal(
                    "run-1",
                    crate::session::RUN_STATE_COMPLETED,
                    0,
                    0,
                    None,
                ),
                crate::session::SessionEntry::new_user("user", serde_json::json!("hi")),
            ],
        );
        let resp = parse_response(&handle_command_internal(
            &state,
            make_cmd("get_session_entries"),
        ));
        assert_eq!(resp["success"], true);
    }

    #[test]
    fn fork_warns_when_model_sync_fails() {
        let state = make_app_state();
        let user = crate::session::SessionEntry::new_user("user", serde_json::json!("fork here"));
        let entry_id = user.id.clone();
        // A unique parent id gates the hook against parallel tests that fork
        // the "default" session. A session_info entry makes the forked model
        // non-empty, so the fork reaches the model-sync block (and consumes
        // the hook).
        save_via(
            &state,
            "fork-warn-parent",
            "mock",
            vec![
                crate::session::SessionEntry::session_info(
                    serde_json::json!({"cwd": state.welcome_cwd, "model": "mock"}),
                    "mock".to_string(),
                    "high".to_string(),
                ),
                user,
            ],
        );
        *MODEL_SYNC_FAIL_HOOK.lock() = Some((
            "fork-warn-parent".to_string(),
            Box::new(|sess: &mut ServerSession| {
                // Closing persistence makes the subsequent model-sync
                // `update_info` fail, exercising the warn arm.
                let _ = sess.persistence.close();
            }),
        ));

        let mut cmd = make_cmd("fork");
        cmd.entry_id = entry_id;
        cmd.parent_session = "fork-warn-parent".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
    }

    #[test]
    fn clone_warns_when_model_sync_fails() {
        let state = make_app_state();
        // A dedicated session id gates the hook against parallel tests that
        // clone the "default" session.
        let _ = parse_response(&handle_command_internal(
            &state,
            make_cmd_for("new_session", "clone-warn"),
        ));
        {
            let session = state.get_session("clone-warn").unwrap();
            session
                .read()
                .messages
                .write()
                .push(crate::types::AgentMessage::new_user(
                    "user",
                    serde_json::json!("clone me"),
                ));
        }
        save_via(
            &state,
            "clone-warn",
            "mock",
            vec![
                crate::session::SessionEntry::session_info(
                    serde_json::json!({"cwd": "/tmp", "model": "mock"}),
                    "mock".to_string(),
                    "high".to_string(),
                ),
                crate::session::SessionEntry::new_user("user", serde_json::json!("clone me")),
                crate::session::SessionEntry::new_assistant(serde_json::json!("reply"), vec![]),
            ],
        );
        *MODEL_SYNC_FAIL_HOOK.lock() = Some((
            "clone-warn".to_string(),
            Box::new(|sess: &mut ServerSession| {
                let _ = sess.persistence.close();
            }),
        ));

        let resp = parse_response(&handle_command_internal(
            &state,
            make_cmd_for("clone", "clone-warn"),
        ));
        assert_eq!(resp["success"], true);
    }

    #[test]
    fn clone_with_empty_disk_model_completes() {
        let state = make_app_state();
        {
            let session = state.get_session("default").unwrap();
            session
                .read()
                .messages
                .write()
                .push(crate::types::AgentMessage::new_user(
                    "user",
                    serde_json::json!("clone me"),
                ));
        }
        // A disk session with messages but no session_info/model_change → the
        // forked model resolves empty, skipping the model-sync block.
        save_via(
            &state,
            "default",
            "mock",
            vec![
                crate::session::SessionEntry::new_user("user", serde_json::json!("clone me")),
                crate::session::SessionEntry::new_assistant(serde_json::json!("reply"), vec![]),
            ],
        );
        let resp = parse_response(&handle_command_internal(&state, make_cmd("clone")));
        assert_eq!(resp["success"], true);
    }

    #[test]
    fn new_session_with_invalid_settings_file_uses_defaults() {
        let home = TestHome::new();
        let settings_path = home.settings_path();
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        std::fs::write(&settings_path, "not valid json").unwrap();

        let state = make_app_state();
        let cmd = make_cmd_for("new_session", "ns-bad-settings");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
    }
}
