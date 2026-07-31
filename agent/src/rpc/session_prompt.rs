use anyhow::Result;
use std::sync::Arc;

use super::prompt_helpers::{
    approve_tool_path_if_present, build_user_message, canonical_stream_event,
    prepare_session_tool_call, stream_event_to_sse_data,
};
use super::ServerSession;

impl ServerSession {
    pub fn prompt(
        &mut self,
        msg: &str,
        images: &[crate::types::ImageContent],
        attachments: &[crate::types::Attachment],
        requested_run_id: Option<&str>,
        client_request_id: Option<&str>,
    ) -> Result<crate::runtime::RunLease> {
        crate::utils::ensure_workspace_accessible(std::path::Path::new(&self.cwd))?;
        let (system_prompt, verbose, mut run_loop) = {
            let mut shared = self
                .agent_loop
                .try_write()
                .map_err(|_| anyhow::anyhow!("session run configuration is busy"))?;
            // Apply session-level settings at the run boundary. Updates made
            // after this snapshot are intentionally deferred to the next run
            // and cannot partially affect the accepted run.
            let thinking_budget = match self.thinking_level.as_str() {
                "minimal" => 2_000,
                "low" => 4_000,
                "medium" => 8_000,
                "high" => 16_000,
                "xhigh" => 24_000,
                _ => 0,
            };
            shared.config.thinking_budget = thinking_budget;
            shared
                .provider
                .update_thinking(&self.thinking_level, thinking_budget);
            shared.steering_queue.mode = self.steering_mode.clone();
            shared.follow_up_queue.mode = self.follow_up_mode.clone();
            self.swap_token_counters_into_loop(&mut shared);
            self.wire_auto_compaction(&mut shared);
            let system_prompt = self.build_system_prompt(shared.tools.clone());
            shared.system_prompt = system_prompt.clone();
            shared.config.system_prompt = system_prompt.clone();

            // Freeze provider/tools/config in the same short critical section.
            // The shared Loop remains the next-run control plane and can be
            // updated immediately while this independent snapshot streams.
            let mut snapshot = shared.independent_copy();
            // Auto-compaction callbacks report through these
            // shared per-session cells; keep the snapshot wired to the same
            // cells instead of the fresh defaults from independent_copy().
            snapshot.last_compaction_result = shared.last_compaction_result.clone();
            snapshot.compaction_failed = shared.compaction_failed.clone();
            snapshot.compaction_occurred = shared.compaction_occurred.clone();
            (system_prompt, shared.verbose, snapshot)
        };
        run_loop.cumulative_input_tokens = self.tokens_in.clone();
        run_loop.cumulative_output_tokens = self.tokens_out.clone();
        run_loop.cumulative_cache_read_tokens = self.tokens_cache_r.clone();
        run_loop.cumulative_cache_write_tokens = self.tokens_cache_w.clone();
        run_loop.cumulative_cost = self.cumulative_cost.clone();
        run_loop.last_prompt_tokens = self.last_prompt_tokens.clone();
        run_loop.steering_queue.set_mode(&self.steering_mode);
        run_loop.follow_up_queue.set_mode(&self.follow_up_mode);
        self.steering_tx = run_loop.steering_queue.sender();
        self.follow_up_tx = run_loop.follow_up_queue.sender();

        // Whether the active model accepts image input (catalog modalities).
        // Uses the cached registry from ServerSession to avoid ~15% CPU overhead
        // from re-deserialising the full model catalog on every prompt.
        let model_supports_images =
            crate::models::model_accepts_images_with(&self.model_registry.read(), &self.model);
        // Images are read + (down)encoded to base64 here, on the agent, from the
        // local path the GUI sent — the base64 never crosses the wire.
        let mut user_message = build_user_message(
            msg,
            images,
            attachments,
            model_supports_images,
            &crate::utils::image_data_url_for_model,
        );

        // NOTE: No content-based dedup here. Idempotency for transport-level
        // retries is enforced atomically by SessionRuntime::begin.
        // A text-based dedup at this point only ever fires when NOT streaming —
        // i.e. exactly the cases that must run: retrying after a failed run,
        // or deliberately repeating a message ("continue", "yes", same text
        // with different attachments).
        let user_text = user_message.text();
        let user_display_text = user_message.display_text();

        // Accept the run before mutating messages or persistence. This is the
        // sole Idle -> Starting transition, so abort -> resend cannot create a
        // second task while the cancelled task is still unwinding.
        let run_lease = self.runtime.begin(requested_run_id, client_request_id)?;
        self.broadcaster
            .start_run(run_lease.run_id.clone(), run_lease.epoch as i64);
        // This run starts with a clean persistence-error and compaction state so
        // the run-end commit decision (append-only vs healing rewrite) reflects
        // only this run. Runs are serialized per session, so no concurrent run
        // can observe the reset.
        self.persistence.reset_error();
        run_loop
            .compaction_occurred
            .store(false, std::sync::atomic::Ordering::SeqCst);
        run_loop
            .stream_incomplete
            .store(false, std::sync::atomic::Ordering::SeqCst);
        user_message
            .metadata
            .get_or_insert_with(serde_json::Map::new)
            .insert(
                "run_id".to_string(),
                serde_json::Value::String(run_lease.run_id.clone()),
            );
        self.messages.write().push(user_message);

        // Log the user message so the run log shows the question alongside
        // the answer (thinking/output blocks already land via eprint_log!).
        if verbose {
            tracing::info!("[user] {user_text}");
        }

        // Persist immediately so the GUI can see the user message (and any
        // tool entries from prior turns) during streaming. Without this, a
        // thread switch mid-stream loses the question until the run settles
        // because get_session_entries reads from disk.
        // Ephemeral sessions (--no-session) skip persistence entirely.
        if !self.ephemeral {
            if let Err(error) = self.persist_user_message(&run_lease) {
                self.messages.write().pop();
                let _ = self.runtime.begin_finalizing(&run_lease);
                let full_error = format!("Failed to persist accepted user message: {error:#}");
                self.broadcaster.broadcast(crate::rpc::SseEvent {
                    event_type: "error".to_string(),
                    data: serde_json::json!({"error": &full_error}).to_string(),
                    ..Default::default()
                });
                self.broadcaster.broadcast(crate::rpc::SseEvent {
                    event_type: "agent_end".to_string(),
                    data: serde_json::json!({
                        "type": "agent_end",
                        "error": &full_error,
                    })
                    .to_string(),
                    ..Default::default()
                });
                let _ = self.runtime.finish(&run_lease);
                return Err(error);
            }
        }

        // Broadcast only after the durability boundary succeeds. Use
        // display_text (first text block only): text() also joins the
        // agent-injected attachment manifest, which observers would render as
        // a bogus extra bubble.
        self.broadcaster.broadcast(crate::rpc::SseEvent::new(
            "user_message",
            serde_json::json!({"text": user_display_text}),
        ));

        // Clone shared state for the background task
        let messages_arc = self.messages.clone();
        let initial_messages = messages_arc.read().clone();
        let broadcaster = self.broadcaster.clone();
        let runtime = self.runtime.clone();
        let task_lease = run_lease.clone();
        let session_manager = self.session_manager.clone();
        let session_persistence = self.persistence.clone();
        let session_id = self.session_id.clone();
        let session_cwd = self.cwd.clone();
        let session_model = self.model.clone();
        let session_thinking = self.thinking_level.clone();
        let tokens_in = self.tokens_in.clone();
        let tokens_out = self.tokens_out.clone();
        let tokens_cache_r = self.tokens_cache_r.clone();
        let tokens_cache_w = self.tokens_cache_w.clone();
        let cumulative_cost = self.cumulative_cost.clone();
        let last_prompt = self.last_prompt_tokens.clone();
        let session_name = self.session_name.clone();
        let created_by = self.created_by.clone();
        let source_meta = self.source_meta.clone();
        let auto_compaction = self.auto_compaction;
        let approval_gate = self.approval_gate.clone();
        let is_ephemeral = self.ephemeral;
        // Shared with the next-run control plane (re-shared in independent_copy),
        // so the run task can read at finalize whether compaction diverged the
        // in-memory history from the appended JSONL.
        let compaction_occurred = run_loop.compaction_occurred.clone();

        // Resolve the sandbox boundary once per run: canonicalized writable
        // roots + platform availability. Shared by the approval closure (pre-
        // execution decisions), the shell wrapper, and write/edit boundary
        // checks so all layers agree. No explicit policy (every non-GUI client)
        // → dormant sandbox = legacy behavior. Session rules are cleared at run
        // start and shared into the sandbox so same-run "allow in this
        // workspace" injections take effect immediately (APPROVAL_PLAN §6.2).
        self.session_rules.lock().clear();
        let sandbox = Arc::new(match &self.sandbox_policy {
            Some(policy) => crate::sandbox::ResolvedSandbox::resolve_with_session(
                policy,
                &self.cwd,
                self.session_rules.clone(),
            ),
            None => crate::sandbox::ResolvedSandbox::disabled(&self.cwd),
        });

        // Build per-session StreamContext (callbacks) — these are session-
        // specific closures and must NOT be stored on the shared Loop.
        let tool_event_cb: Option<Arc<dyn Fn(crate::types::StreamEvent) + Send + Sync>> = {
            let bt = broadcaster.clone();
            Some(Arc::new(move |event: crate::types::StreamEvent| {
                bt.broadcast(crate::rpc::SseEvent {
                    event_type: event.event_type.clone(),
                    data: stream_event_to_sse_data(&event),
                    ..Default::default()
                });
            }))
        };
        let save_messages = messages_arc.clone();
        let save_persistence = self.persistence.clone();
        let persisted_run_id = run_lease.run_id.clone();
        let save_closure: crate::agent::PersistCallback =
            Arc::new(move |msg: &crate::types::AgentMessage| {
                if is_ephemeral {
                    return;
                }
                let mut persisted = msg.clone();
                if persisted.role == "assistant" {
                    persisted
                        .metadata
                        .get_or_insert_with(serde_json::Map::new)
                        .insert(
                            "run_id".to_string(),
                            serde_json::Value::String(persisted_run_id.clone()),
                        );
                }
                save_messages.write().push(persisted.clone());
                let entry = crate::session::agent_message_to_entry(&persisted);
                if let Err(error) = save_persistence.append(vec![entry]) {
                    tracing::error!("Failed to enqueue session entry: {error}");
                }
            });
        // Steering can reorder a user message ahead of the current context,
        // while follow-ups append one inside the same canonical run. Persist
        // both immediately for crash visibility, then heal their exact
        // in-memory ordering with a rewrite at the terminal boundary.
        //
        // TODO(runtime-persistence): split steering and follow-up callbacks so
        // append-only follow-ups can avoid the rewrite while reordered steering
        // records an explicit journal transform.
        let in_run_user_message_seen = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let user_msg_cb: crate::agent::PersistCallback = {
            let b = broadcaster.clone();
            let persistence = self.persistence.clone();
            let rewrite_required = in_run_user_message_seen.clone();
            Arc::new(move |msg: &crate::types::AgentMessage| {
                rewrite_required.store(true, std::sync::atomic::Ordering::SeqCst);
                let entry = crate::session::agent_message_to_entry(msg);
                if let Err(error) = persistence.append(vec![entry]) {
                    tracing::error!("Failed to enqueue in-run user message: {error}");
                }
                b.broadcast(crate::rpc::SseEvent::new(
                    "user_message",
                    serde_json::json!({"text": msg.display_text()}),
                ));
            })
        };
        let stream_ctx = crate::agent::StreamContext {
            // Use the bare model ID from the Loop — the LLM API expects just
            // the model name, not the "provider/model" display format stored
            // on ServerSession.
            model: run_loop.model.clone(),
            system_prompt,
            on_tool_result: Some(save_closure.clone()),
            save_callback: Some(save_closure),
            tool_event_callback: tool_event_cb,
            on_user_message: Some(user_msg_cb),
        };

        // Set approval/sandbox hooks on this session's Loop config (these
        // are not callbacks — they're tool-execution hooks on AgentConfig).
        let approval_gate_hook = approval_gate.clone();
        let approval_broadcaster = broadcaster.clone();
        let approval_session_id = session_id.clone();
        let approval_cwd = session_cwd.clone();
        let approval_sandbox = sandbox.clone();
        let permission_level = self.permission_level.clone();
        run_loop.config.before_tool_call =
            Some(Arc::new(
                move |tool_name, tool_id, arguments| match permission_level.as_str() {
                    "all" => {
                        approve_tool_path_if_present(&approval_cwd, tool_name, arguments);
                        None
                    }
                    "none" => Some(crate::types::ToolCallResult {
                        result: format!(
                            "Tool call `{tool_name}` denied: permission level is set to 'none'."
                        ),
                        is_error: true,
                    }),
                    _ => approval_gate_hook.request(
                        &approval_broadcaster,
                        &approval_session_id,
                        &approval_cwd,
                        tool_name,
                        tool_id,
                        arguments,
                        &approval_sandbox,
                    ),
                },
            ));
        let prepare_cwd = session_cwd.clone();
        run_loop.config.prepare_tool_call = Some(Arc::new(move |tool_name, arguments| {
            prepare_session_tool_call(&prepare_cwd, tool_name, arguments)
        }));

        // agent_start is now emitted inside run_streaming_with_messages via on_event,
        // for both initial prompts and follow-up turns.

        // Clear any stale interrupt flag copied from the next-run control
        // plane. Each run owns its snapshot after this boundary.
        run_loop.clear_interrupt();
        let shared_interrupt_flag = run_loop.interrupt_flag();

        // Create interrupt channel so steer()/abort() can stop the current stream
        let (interrupt_tx, interrupt_rx) = tokio::sync::mpsc::channel::<()>(1);
        // Capture both cancellation paths in the run-scoped lease. Abort can
        // now signal the task without making the session appear idle.
        let cancellation_installed = self.runtime.install_cancellation(
            &run_lease,
            interrupt_tx,
            shared_interrupt_flag.clone(),
        );
        debug_assert!(cancellation_installed);

        // Post-hoc escalation channel (SANDBOX_PLAN.md §2.6): lets run_shell
        // raise a `sandbox_escalation` approval after a sandbox denial without
        // the tools layer touching RPC internals. Blocks until the user decides.
        let escalation: crate::sandbox::EscalationRequester = {
            let gate = approval_gate.clone();
            let escalation_broadcaster = broadcaster.clone();
            let escalation_session_id = session_id.clone();
            let escalation_sandbox = sandbox.clone();
            Arc::new(move |request: &crate::sandbox::EscalationRequest| {
                gate.request_escalation(
                    &escalation_broadcaster,
                    &escalation_session_id,
                    request,
                    &escalation_sandbox,
                )
            })
        };

        // Sandboxed-execution notifier → `tool_sandboxed` run event (Run
        // Inspect shows which commands ran inside the OS sandbox).
        let on_sandboxed: crate::tools::SandboxedNotifier = {
            let sandbox_broadcaster = broadcaster.clone();
            Arc::new(move |command: &str| {
                sandbox_broadcaster.broadcast(crate::rpc::SseEvent {
                    event_type: "tool_sandboxed".to_string(),
                    data: serde_json::json!({
                        "type": "tool_sandboxed",
                        "command": command,
                    })
                    .to_string(),
                    ..Default::default()
                });
            })
        };

        // Build the run future; SessionRuntime owns spawning, monitoring, and
        // the task slot. TODO(runtime-persistence): once terminal persistence
        // is an ordered writer command, move the remaining execution-input
        // assembly into a dedicated RunExecution value.
        let perm = self.permission_level.clone();
        let scope_sandbox = sandbox.clone();
        let run_task = async move {
            // Anchors for per-reply metadata written at the save site: wall-clock
            // start and the session-cumulative output-token count before this
            // prompt ran. The delta/elapsed are attributed to the final assistant
            // entry so the GUI can show "time · N tokens" when reloading history.
            let run_start = std::time::Instant::now();
            let out_start = tokens_out.load(std::sync::atomic::Ordering::Relaxed);
            let result = crate::tools::with_tool_scope(
                crate::tools::ScopeOptions {
                    workspace: session_cwd.clone(),
                    permission_level: perm,
                    interrupt_flag: shared_interrupt_flag,
                    sandbox: scope_sandbox,
                    escalation: Some(escalation),
                    on_sandboxed: Some(on_sandboxed),
                },
                async {
                    let mut current_messages = initial_messages;
                    let mut current_interrupt_rx = Some(interrupt_rx);

                    loop {
                        let bt = broadcaster.clone();
                        let be = broadcaster.clone();
                        match run_loop
                            .run_streaming_with_messages(
                                current_messages,
                                &stream_ctx,
                                move |text| {
                                    bt.broadcast(crate::rpc::SseEvent {
                                        event_type: "text_chunk".to_string(),
                                        data: serde_json::json!({"text": text}).to_string(),
                                        ..Default::default()
                                    });
                                },
                                move |event| {
                                    if let Some(event) = canonical_stream_event(event) {
                                        be.broadcast(crate::rpc::SseEvent {
                                            event_type: event.event_type.clone(),
                                            data: stream_event_to_sse_data(&event),
                                            ..Default::default()
                                        });
                                    }
                                },
                                current_interrupt_rx.take(),
                            )
                            .await
                        {
                            Ok((_, final_messages)) => {
                                current_messages = final_messages;

                                let follow_ups = run_loop.follow_up_queue.drain();

                                if follow_ups.is_empty() {
                                    return Ok(current_messages);
                                }
                                for msg in follow_ups {
                                    let follow_up = crate::types::AgentMessage::new_user(
                                        "user",
                                        serde_json::json!([{"type": "text", "text": msg}]),
                                    );
                                    if let Some(ref on_user_message) = stream_ctx.on_user_message {
                                        on_user_message(&follow_up);
                                    }
                                    current_messages.push(follow_up);
                                }
                                // No interrupt channel for follow-up re-runs
                                current_interrupt_rx = None;
                            }
                            Err(e) => return Err(e),
                        }
                    }
                },
            )
            .await;

            // This check fences shared messages, disk commits, and terminal
            // events. A completion from an obsolete epoch is never allowed to
            // mutate a newer run.
            let was_cancelled = runtime.snapshot().is_some_and(|active| {
                active.run_id == task_lease.run_id
                    && active.epoch == task_lease.epoch
                    && matches!(
                        active.phase,
                        crate::runtime::RunPhase::Cancelling
                            | crate::runtime::RunPhase::CancellationStuck
                    )
            });
            let stream_incomplete = run_loop
                .stream_incomplete
                .load(std::sync::atomic::Ordering::SeqCst);
            if !runtime.begin_finalizing(&task_lease) {
                return;
            }

            let run_error = match result {
                Ok(mut final_messages) => {
                    if let Some(last_assistant) = final_messages
                        .iter_mut()
                        .rev()
                        .find(|message| message.role == "assistant")
                    {
                        last_assistant
                            .metadata
                            .get_or_insert_with(serde_json::Map::new)
                            .insert(
                                "run_id".to_string(),
                                serde_json::Value::String(task_lease.run_id.clone()),
                            );
                    }
                    // Update shared messages so next prompt includes the full context
                    *messages_arc.write() = final_messages;
                    None
                }
                Err(error) => Some(format!("{error:#}")),
            };
            let run_output_tokens =
                (tokens_out.load(std::sync::atomic::Ordering::Relaxed) - out_start).max(0);
            let run_duration_ms = run_start.elapsed().as_millis() as i64;

            // Every terminal path reaches the same durability boundary before
            // the runtime may return to Idle.
            //
            // Append-only is the fast path: the run's user/assistant/tool
            // entries were already appended during the run, so we only add a
            // run_terminal marker plus a refreshed session_info snapshot, then
            // commit at a durable boundary — O(this run), not O(full history).
            //
            // A full rewrite is reserved for the two cases that make the
            // in-memory history diverge from the appended JSONL: compaction
            // replacing the message list, or a mid-run append failure. commit_run
            // reports the latter by refusing to commit, and we then heal via a
            // full rewrite (which also applies the compacted history).
            let compaction_happened = compaction_occurred.load(std::sync::atomic::Ordering::SeqCst);
            let history_rewrite_required =
                in_run_user_message_seen.load(std::sync::atomic::Ordering::SeqCst);
            let terminal_state = if was_cancelled {
                crate::session::RUN_STATE_CANCELLED
            } else if run_error.is_some() {
                crate::session::RUN_STATE_ERROR
            } else if stream_incomplete {
                crate::session::RUN_STATE_INCOMPLETE
            } else {
                crate::session::RUN_STATE_COMPLETED
            };
            // Clones for the blocking commit task: run_error and task_lease are
            // still needed after the task completes (terminal event dispatch).
            let commit_run_id = task_lease.run_id.clone();
            let commit_run_error = run_error.clone();
            let persistence_task = tokio::task::spawn_blocking(move || {
                if is_ephemeral {
                    return anyhow::Ok(());
                }
                use std::sync::atomic::Ordering;

                // Preserve parent_session_id from the existing session on disk.
                let parent_session_id = session_manager
                    .load(&session_id)
                    .map(|s| s.parent_session_id)
                    .unwrap_or_default();

                // Build the authoritative session_info snapshot that records the
                // session's cumulative metadata at this run boundary. Auto-
                // generate session_name from the first user message when not set
                // explicitly (matches first_message in list_sessions).
                let resolved_name = if session_name.is_empty() {
                    messages_arc
                        .read()
                        .iter()
                        .find(|m| m.role == "user")
                        .map(|m| m.display_text())
                        .map(|s| crate::session::truncate_visible(s.trim(), 40))
                        .unwrap_or_default()
                } else {
                    session_name
                };
                let total_cost = *cumulative_cost.lock();
                let mut info = serde_json::json!({
                    "cwd": session_cwd,
                    "tokens_in": tokens_in.load(Ordering::Relaxed),
                    "tokens_out": tokens_out.load(Ordering::Relaxed),
                    "tokens_cache_r": tokens_cache_r.load(Ordering::Relaxed),
                    "tokens_cache_w": tokens_cache_w.load(Ordering::Relaxed),
                    "last_prompt_tokens": last_prompt.load(Ordering::Relaxed),
                    "total_cost": total_cost,
                    "session_name": resolved_name,
                    "auto_compaction": auto_compaction,
                    "parent_session_id": parent_session_id,
                    "thinking_level": session_thinking,
                    "model": session_model,
                });
                if !created_by.is_empty() {
                    info["created_by"] = serde_json::Value::String(created_by);
                }
                if !source_meta.is_null() {
                    info["source_meta"] = source_meta;
                }
                let info_entry = crate::session::SessionEntry::session_info(
                    info,
                    session_model.clone(),
                    session_thinking.clone(),
                );
                let run_started =
                    crate::session::SessionEntry::run_started(&commit_run_id, task_lease.epoch);
                let terminal = crate::session::SessionEntry::run_terminal(
                    &commit_run_id,
                    terminal_state,
                    run_output_tokens,
                    run_duration_ms,
                    commit_run_error.as_deref(),
                );

                if compaction_happened || history_rewrite_required {
                    // Compaction replaced the in-memory history, so the appended
                    // JSONL no longer matches. In-run steering/follow-up can
                    // likewise change message ordering. Rewrite the authoritative
                    // snapshot and retain its lifecycle journal.
                    let messages = messages_arc.read().clone();
                    let session = Self::build_rewrite_snapshot(
                        &session_manager,
                        &session_id,
                        &session_cwd,
                        &session_model,
                        &resolved_name,
                        &parent_session_id,
                        &messages,
                        info_entry,
                        run_output_tokens,
                        run_duration_ms,
                        tokens_in.load(Ordering::Relaxed),
                        tokens_out.load(Ordering::Relaxed),
                        run_started,
                        terminal,
                    );
                    session_persistence.rewrite_run_snapshot(session)?;
                    return anyhow::Ok(());
                }

                // Append-only fast path: terminal marker + refreshed session_info,
                // committed at a durable (fsync) boundary. commit_run is ordered
                // after every mid-run append, so it refuses if any of them failed;
                // in that case heal with a full rewrite.
                // Keep the terminal marker last. A crash before the fsync may
                // leave a missing/partial tail, but cannot expose a terminal
                // marker followed by an incomplete metadata record.
                match session_persistence.commit_run(vec![info_entry.clone(), terminal.clone()]) {
                    Ok(()) => anyhow::Ok(()),
                    Err(commit_error) => {
                        tracing::warn!(
                            run_id = %commit_run_id,
                            "append-only run commit refused ({commit_error}); healing with full rewrite"
                        );
                        let messages = messages_arc.read().clone();
                        let session = Self::build_rewrite_snapshot(
                            &session_manager,
                            &session_id,
                            &session_cwd,
                            &session_model,
                            &resolved_name,
                            &parent_session_id,
                            &messages,
                            info_entry,
                            run_output_tokens,
                            run_duration_ms,
                            tokens_in.load(Ordering::Relaxed),
                            tokens_out.load(Ordering::Relaxed),
                            run_started,
                            terminal,
                        );
                        session_persistence.rewrite_run_snapshot(session)?;
                        anyhow::Ok(())
                    }
                }
            });
            let persistence_error = match persistence_task.await {
                Ok(Ok(())) => None,
                Ok(Err(error)) => Some(format!("{error:#}")),
                Err(error) => Some(format!("persistence task failed: {error}")),
            };
            if let Some(error) = persistence_error {
                let _ = runtime.mark_persistence_degraded(&task_lease, &error);
                tracing::error!(
                    run_id = %task_lease.run_id,
                    "Session persistence commit failed: {error}"
                );
                broadcaster.broadcast(crate::rpc::SseEvent {
                    event_type: "persistence_error".to_string(),
                    data: serde_json::json!({"error": &error}).to_string(),
                    ..Default::default()
                });
                broadcaster.broadcast(crate::rpc::SseEvent {
                    event_type: "agent_end".to_string(),
                    data: serde_json::json!({
                        "type": "agent_end",
                        "state": crate::session::RUN_STATE_ERROR,
                        "error": format!("Session persistence failed: {error}"),
                        "usage": { "output_tokens": run_output_tokens },
                        "duration_ms": run_duration_ms
                    })
                    .to_string(),
                    ..Default::default()
                });
                return;
            }

            match run_error {
                None => {
                    // Carry this run's output-token total, wall-clock duration,
                    // and terminal state on the event so clients (notably the IM
                    // channel bridges and remote mobile/web clients) can show the
                    // stats and distinguish a cancellation from a clean completion
                    // the instant the run settles — without depending on having
                    // seen every streamed event (a late-joining client may only
                    // have the tail of the run's event ring).
                    let mut data = serde_json::json!({
                        "type": "agent_end",
                        "state": terminal_state,
                        "usage": { "output_tokens": run_output_tokens },
                        "duration_ms": run_duration_ms
                    });
                    if stream_incomplete {
                        data["reason"] = serde_json::Value::String("incomplete".to_string());
                    }
                    broadcaster.broadcast(crate::rpc::SseEvent {
                        event_type: "agent_end".to_string(),
                        data: data.to_string(),
                        ..Default::default()
                    });
                }
                Some(full_error) => {
                    tracing::error!("Agent loop error: {}", full_error);
                    broadcaster.broadcast(crate::rpc::SseEvent {
                        event_type: "error".to_string(),
                        data: serde_json::json!({"error": &full_error}).to_string(),
                        ..Default::default()
                    });
                    broadcaster.broadcast(crate::rpc::SseEvent {
                        event_type: "agent_end".to_string(),
                        data: serde_json::json!({
                            "type": "agent_end",
                            "state": terminal_state,
                            "error": &full_error,
                            "usage": { "output_tokens": run_output_tokens },
                            "duration_ms": run_duration_ms
                        })
                        .to_string(),
                        ..Default::default()
                    });
                }
            }
        };
        if let Err(error) = self.runtime.spawn(run_lease.clone(), run_task) {
            let _ = self.runtime.begin_finalizing(&run_lease);
            let full_error = format!("Failed to start accepted run task: {error:#}");
            self.broadcaster.broadcast(crate::rpc::SseEvent {
                event_type: "error".to_string(),
                data: serde_json::json!({"error": &full_error}).to_string(),
                ..Default::default()
            });
            self.broadcaster.broadcast(crate::rpc::SseEvent {
                event_type: "agent_end".to_string(),
                data: serde_json::json!({
                    "type": "agent_end",
                    "error": &full_error,
                })
                .to_string(),
                ..Default::default()
            });
            let _ = self.runtime.finish(&run_lease);
            return Err(error);
        }

        Ok(run_lease)
    }
    /// Build this turn's system prompt: project context (CLAUDE.md/AGENTS.md/
    /// GEMINI.md), workspace memory (FUTURE.md), discovered skills, and the
    /// write/memory guidelines. Read fresh each turn (cwd-scoped).
    /// Point the agent loop's cumulative token/cost counters at this session's
    /// shared atomics so streaming updates are tracked per-session.
    fn swap_token_counters_into_loop(&self, r#loop: &mut crate::agent::Loop) {
        r#loop.cumulative_input_tokens = self.tokens_in.clone();
        r#loop.cumulative_output_tokens = self.tokens_out.clone();
        r#loop.cumulative_cache_read_tokens = self.tokens_cache_r.clone();
        r#loop.cumulative_cache_write_tokens = self.tokens_cache_w.clone();
        r#loop.cumulative_cost = self.cumulative_cost.clone();
        r#loop.last_prompt_tokens = self.last_prompt_tokens.clone();
    }

    /// Install the pre-turn auto-compaction transform on the agent loop (a
    /// no-op when auto-compaction is off), compacting context once usage
    /// crosses ~90% of the model's window.
    fn wire_auto_compaction(&self, r#loop: &mut crate::agent::Loop) {
        if self.auto_compaction {
            let comp_tokens = self.last_prompt_tokens.clone();
            let comp_result = r#loop.last_compaction_result.clone();
            let comp_failed = r#loop.compaction_failed.clone();
            let comp_occurred = r#loop.compaction_occurred.clone();
            // Resolve context_window once — reuse cached registry
            // to avoid re-deserialising the model catalog.
            let context_window = self
                .model_registry
                .read()
                .resolve(&self.model)
                .map(|m| m.context_window)
                .unwrap_or(1_000_000); // Modern default: 1M (was 200K — too low for 1M models)
            r#loop.config.transform_context = Some(Arc::new(move |msgs, _| {
                use std::sync::atomic::Ordering;
                let api_tokens = comp_tokens.load(Ordering::Relaxed) as i32;
                // Fall back to heuristic estimate when API doesn't report usage.
                let context_tokens = if api_tokens > 0 {
                    api_tokens
                } else {
                    crate::compaction::estimate_context_tokens(&msgs)
                };
                if context_tokens == 0 {
                    return msgs; // Truly empty — nothing to compact
                }
                // Compact when context usage exceeds 90% (10% reserve, min 16K).
                // Keep more history: 50% of context window so the model retains
                // substantial conversation continuity after compaction.
                let reserve_tokens = ((context_window as f64 * 0.1) as i32).max(16384);
                let keep_tokens = ((context_window as f64 * 0.2) as i32).max(reserve_tokens);
                let needs_compact = context_tokens > context_window - reserve_tokens;
                let (compacted, result) = crate::compaction::compact(
                    msgs,
                    &crate::compaction::CompactOptions {
                        reserve_tokens,
                        keep_recent_tokens: keep_tokens,
                        context_window,
                        tokens_before: context_tokens,
                    },
                );
                if let Some(r) = result {
                    *comp_result.lock() = Some(r);
                    // Signal the run-end persistence path that compaction
                    // replaced the in-memory history, so it diverges from the
                    // appended JSONL and must be persisted via a full rewrite
                    // rather than an append-only commit.
                    comp_occurred.store(true, Ordering::SeqCst);
                    compacted
                } else if needs_compact {
                    // Compaction was needed but compact() returned no result,
                    // meaning it found no valid cut point. Signal failure so
                    // the run loop can report an error instead of silently
                    // proceeding with full (overflowing) context.
                    tracing::error!(
                        tokens = context_tokens,
                        window = context_window,
                        "auto-compaction needed but failed"
                    );
                    comp_failed.store(true, Ordering::SeqCst);
                    compacted
                } else {
                    compacted
                }
            }));
        }
    }

    fn build_system_prompt(&self, tools: Vec<crate::types::AgentTool>) -> String {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();

        // Discover skills so they appear in the system prompt's <available_skills> block.
        // Global user-level dirs only — identical for every session/cwd, which
        // keeps the skills cache correct regardless of which session refreshes it.
        let skills = crate::skills::discover_skills_cached(&crate::skills::global_skill_dirs());

        // Load project context (AGENTS.md / CLAUDE.md / GEMINI.md)
        let mut agent_content = String::new();
        for fname in &["AGENTS.md", "CLAUDE.md", "GEMINI.md"] {
            let p = std::path::Path::new(&self.cwd).join(fname);
            if p.exists() {
                if let Ok(content) = std::fs::read_to_string(&p) {
                    agent_content = content;
                    break;
                }
            }
        }

        // Load workspace memory (FUTURE.md) — a separate layer from project
        // context, read fresh each turn (cwd only; workspace-scoped).
        let memory_path = std::path::Path::new(&self.cwd).join("FUTURE.md");
        let memory_content = std::fs::read_to_string(&memory_path).unwrap_or_default();

        crate::prompt::build_prompt(&crate::prompt::PromptOptions {
            working_directory: self.cwd.replace('\\', "/"),
            date: today,
            tools,
            skills,
            agent_content,
            memory_content,
            session_id: self.session_id.clone(),
            prompt_guidelines: vec![
                // The write-via-shell prohibition is platform-neutral, but its
                // examples must name the redirection forms the host's shell
                // actually has, or a PowerShell model won't map "don't use
                // `cat > file`" onto "don't use Out-File".
                {
                    #[cfg(not(target_os = "windows"))]
                    let forms = "`>`, `>>`, tee, heredocs, `cat > file`";
                    #[cfg(target_os = "windows")]
                    let forms = "`>`, `>>`, Out-File, Set-Content, Add-Content";
                    format!("When asked to create, save, write, or modify a file, ALWAYS use the write or edit tool — including for absolute paths and paths outside the current working directory (both tools accept any path). Do NOT use shell redirection ({forms}) to write files: shell file writes bypass file tracking and the approval flow. Reserve shell redirection for piping between commands, not for creating files. Only describe file changes after the tool succeeds.")
                },
            ],
            ..Default::default()
        })
    }

    /// Persist the just-pushed user message so the GUI sees it mid-stream.
    /// Uses append-only when the session file already exists (avoids a full
    /// rewrite); falls back to full save for a brand-new session that has no
    /// JSONL yet.  The session_info line (token counts, model, name) stays
    /// at its last-completed-run values — the final save at run end refreshes
    /// it. A failure rejects StartRun so memory and JSONL cannot diverge before
    /// the model begins producing side effects.
    fn persist_user_message(&self, run_lease: &crate::runtime::RunLease) -> Result<()> {
        // Use in-memory parent_session_id — avoids reading the entire session
        // file from disk just to get one field.
        let parent_session_id = self.parent_session_id.clone();
        let msgs = self.messages.read();
        // The run_started marker is persisted together with the user message so
        // the journal durably records that this run began. A run_started with no
        // matching run_terminal identifies a run interrupted by crash/restart.
        let run_started =
            crate::session::SessionEntry::run_started(&run_lease.run_id, run_lease.epoch);

        // Fast path: an existing session atomically closes any run left open by
        // a prior process restart and appends this run's user/start records.
        // Refuse the new run if that durability boundary fails: allowing the new
        // run_started through would hide the older open marker.
        if let Some(last_msg) = msgs.last() {
            let entry = crate::session::agent_message_to_entry(last_msg);
            if self.session_manager.find(&self.session_id).is_some() {
                self.session_manager.append_run_start(
                    &self.session_id,
                    entry,
                    run_started.clone(),
                )?;
                return Ok(());
            }
        }

        // Slow path: full save for a brand-new session.
        let mut entries: Vec<crate::session::SessionEntry> = msgs
            .iter()
            .map(crate::session::agent_message_to_entry)
            .collect();
        // Prepend session_info so token counts and other metadata survive
        // a crash — without this, a restarted session starts with zeroed
        // token counters and may skip needed compaction.
        {
            use std::sync::atomic::Ordering;
            let session_name = if !self.session_name.is_empty() {
                self.session_name.clone()
            } else {
                entries
                    .iter()
                    .find(|e| e.role == "user")
                    .and_then(|e| e.content.as_ref())
                    .map(|c| {
                        if let Some(arr) = c.as_array() {
                            arr.iter()
                                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                                .next()
                                .unwrap_or("")
                                .to_string()
                        } else {
                            c.as_str().unwrap_or("").to_string()
                        }
                    })
                    .map(|s| crate::session::truncate_visible(s.trim(), 40))
                    .unwrap_or_default()
            };
            let mut info = serde_json::json!({
                "cwd": self.cwd,
                "tokens_in": self.tokens_in.load(Ordering::Relaxed),
                "tokens_out": self.tokens_out.load(Ordering::Relaxed),
                "tokens_cache_r": self.tokens_cache_r.load(Ordering::Relaxed),
                "tokens_cache_w": self.tokens_cache_w.load(Ordering::Relaxed),
                "last_prompt_tokens": self.last_prompt_tokens.load(Ordering::Relaxed),
                "total_cost": *self.cumulative_cost.lock(),
                "session_name": session_name,
                "auto_compaction": self.auto_compaction,
                "parent_session_id": parent_session_id,
                "thinking_level": self.thinking_level.clone(),
                "model": self.model.clone(),
            });
            if !self.created_by.is_empty() {
                info["created_by"] = serde_json::Value::String(self.created_by.clone());
            }
            if !self.source_meta.is_null() {
                info["source_meta"] = self.source_meta.clone();
            }
            let info_entry = crate::session::SessionEntry::session_info(
                info,
                self.model.clone(),
                self.thinking_level.clone(),
            );
            entries.insert(0, info_entry);
        }
        // Record the run_started marker after the user message so the brand-new
        // session's journal also bounds this run (matches the fast path).
        entries.push(run_started);
        let session = crate::session::Session::snapshot(
            self.session_id.clone(),
            self.cwd.clone(),
            self.model.clone(),
            self.session_name.clone(),
            parent_session_id,
            entries,
        );
        self.session_manager.save(&session)?;
        Ok(())
    }

    /// Rebuild the complete session snapshot from the in-memory message list for
    /// a full rewrite. This is the O(history) path, reserved for runs where
    /// compaction replaced the message history (diverging from the appended
    /// JSONL) or where a mid-run append failed and the file must be healed.
    ///
    /// `info_entry` is the authoritative session_info snapshot (prepended).
    /// On-disk timestamps and prior-run token stats are preserved by index so a
    /// rewrite doesn't reset every message to "just now"; this run's output
    /// tokens and duration are attached to the final assistant entry.
    #[allow(clippy::too_many_arguments)]
    fn build_rewrite_snapshot(
        session_manager: &crate::session::Manager,
        session_id: &str,
        session_cwd: &str,
        session_model: &str,
        resolved_name: &str,
        parent_session_id: &str,
        messages: &[crate::types::AgentMessage],
        info_entry: crate::session::SessionEntry,
        run_output_tokens: i64,
        run_duration_ms: i64,
        tokens_in: i64,
        tokens_out: i64,
        run_started: crate::session::SessionEntry,
        terminal: crate::session::SessionEntry,
    ) -> crate::session::Session {
        use crate::session::SessionEntry;
        let mut entries: Vec<SessionEntry> = messages
            .iter()
            .map(crate::session::agent_message_to_entry)
            .collect();

        // The whole session is rebuilt from the in-memory message list, and
        // agent_message_to_entry re-stamps `now()` with zero token/duration.
        // Without preserving them, every reload shows all messages at the
        // current time ("just now") and drops earlier replies' token counts.
        // Messages only grow by appending, so the on-disk message entries align
        // by index with this prefix. Filter the old side to message entries only
        // (matching what the rebuild produces) so interleaved label/model_change
        // /run-marker entries can't shift the alignment.
        let is_message_entry = |t: &str| {
            matches!(
                t,
                crate::session::ENTRY_TYPE_USER
                    | crate::session::ENTRY_TYPE_ASSISTANT
                    | crate::session::ENTRY_TYPE_TOOL
                    | crate::session::ENTRY_TYPE_SYSTEM
            )
        };
        let old_session = session_manager.load(session_id).ok();
        let old_msg_entries: Vec<SessionEntry> = old_session
            .as_ref()
            .map(|session| {
                session
                    .entries
                    .iter()
                    .filter(|e| is_message_entry(&e.entry_type))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        for (new_entry, old_entry) in entries.iter_mut().zip(old_msg_entries.iter()) {
            new_entry.timestamp = old_entry.timestamp;
            // Preserve run stats from the old entry's content.
            if let Some(ref old_content) = old_entry.content {
                if let Some(obj) = new_entry.content.as_mut().and_then(|c| c.as_object_mut()) {
                    if let Some(v) = old_content.get("run_tokens") {
                        obj.insert("run_tokens".to_string(), v.clone());
                    }
                    if let Some(v) = old_content.get("run_duration_ms") {
                        obj.insert("run_duration_ms".to_string(), v.clone());
                    }
                }
            }
        }

        // Attach this run's output tokens + wall-clock duration to the final
        // assistant entry (the reply just made). It sits beyond the preserved
        // prefix, so earlier replies are untouched.
        if let Some(last_assistant) = entries
            .iter_mut()
            .rev()
            .find(|e| e.entry_type == crate::session::ENTRY_TYPE_ASSISTANT)
        {
            if let Some(ref mut content) = last_assistant.content {
                if let Some(obj) = content.as_object_mut() {
                    obj.insert(
                        "run_tokens".to_string(),
                        serde_json::json!(run_output_tokens),
                    );
                    obj.insert(
                        "run_duration_ms".to_string(),
                        serde_json::json!(run_duration_ms),
                    );
                }
            }
        }

        // If the first user message is a compaction marker, replace it with a
        // proper compaction entry so the JSONL records the compaction point.
        if let Some(idx) = entries.iter().position(|e| {
            e.role == "user"
                && e.content
                    .as_ref()
                    .and_then(|c| c.as_array())
                    .and_then(|arr| {
                        arr.first()
                            .and_then(|b| b.get("text"))
                            .and_then(|t| t.as_str())
                    })
                    .is_some_and(|t| t.starts_with("[Context compaction:"))
        }) {
            if let Some(marker) = entries.get(idx) {
                let summary = marker
                    .content
                    .as_ref()
                    .and_then(|c| c.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|b| b.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                let mut comp_entry = marker.clone();
                comp_entry.id = crate::utils::generate_id();
                comp_entry.entry_type = crate::session::ENTRY_TYPE_COMPACTION.to_string();
                comp_entry.role = "system".to_string();
                comp_entry.content = Some(serde_json::json!({
                    "summary": summary,
                    "tokens_in": tokens_in,
                    "tokens_out": tokens_out,
                }));
                entries.insert(idx + 1, comp_entry);
                entries.remove(idx);
            }
        }

        // A rewrite is a journal compaction/repair, not permission to erase run
        // history. Preserve existing lifecycle markers, ensure the current
        // start marker exists, and make the current terminal marker the final
        // durable record. Markers remain projection-invisible.
        let current_run_id = run_started
            .content
            .as_ref()
            .and_then(|content| content.get("run_id"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let mut markers: Vec<SessionEntry> = old_session
            .map(|session| {
                session
                    .entries
                    .into_iter()
                    .filter(|entry| crate::session::is_run_marker(&entry.entry_type))
                    .filter(|entry| {
                        entry.entry_type != crate::session::ENTRY_TYPE_RUN_TERMINAL
                            || entry
                                .content
                                .as_ref()
                                .and_then(|content| content.get("run_id"))
                                .and_then(serde_json::Value::as_str)
                                != Some(current_run_id)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let has_current_start = markers.iter().any(|entry| {
            entry.entry_type == crate::session::ENTRY_TYPE_RUN_STARTED
                && entry
                    .content
                    .as_ref()
                    .and_then(|content| content.get("run_id"))
                    .and_then(serde_json::Value::as_str)
                    == Some(current_run_id)
        });
        if !has_current_start {
            markers.push(run_started);
        }

        entries.insert(0, info_entry);
        entries.extend(markers);
        entries.push(terminal);

        crate::session::Session::snapshot(
            session_id.to_string(),
            session_cwd.to_string(),
            session_model.to_string(),
            resolved_name.to_string(),
            parent_session_id.to_string(),
            entries,
        )
    }
}

#[cfg(test)]
mod build_user_message_tests;
#[cfg(test)]
mod tests;
