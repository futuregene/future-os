use anyhow::Result;
use std::sync::Arc;

use super::prompt_helpers::{
    approve_tool_path_if_present, build_user_message, prepare_session_tool_call,
    stream_event_to_sse_data,
};
use super::ServerSession;

impl ServerSession {
    pub fn prompt(
        &mut self,
        msg: &str,
        images: &[crate::types::ImageContent],
        attachments: &[crate::types::Attachment],
    ) -> Result<()> {
        std::fs::create_dir_all(&self.cwd)?;
        let system_prompt;
        let verbose = if let Ok(mut r#loop) = self.agent_loop.try_write() {
            let sp = self.build_system_prompt(r#loop.tools.clone());
            r#loop.system_prompt = sp.clone();
            r#loop.config.system_prompt = sp.clone();
            system_prompt = sp;
            r#loop.verbose
        } else {
            system_prompt = String::new();
            false
        };

        // Whether the active model accepts image input (catalog modalities).
        // Uses the cached registry from ServerSession to avoid ~15% CPU overhead
        // from re-deserialising the full model catalog on every prompt.
        let model_supports_images = crate::models::model_accepts_images_with(
            &self.model_registry.read(),
            &self.model,
        );
        // Images are read + (down)encoded to base64 here, on the agent, from the
        // local path the GUI sent — the base64 never crosses the wire.
        let user_message = build_user_message(
            msg,
            images,
            attachments,
            model_supports_images,
            &crate::utils::image_data_url_for_model,
        );

        // NOTE: No content-based dedup here. Idempotency for transport-level
        // retries is enforced atomically by the RPC layer (commands.rs rejects
        // a second "prompt" while is_streaming, under the session write lock).
        // A text-based dedup at this point only ever fires when NOT streaming —
        // i.e. exactly the cases that must run: retrying after a failed run,
        // or deliberately repeating a message ("continue", "yes", same text
        // with different attachments).
        let user_text = user_message.text();
        let user_display_text = user_message.display_text();
        self.messages.write().push(user_message);

        // Log the user message so the run log shows the question alongside
        // the answer (thinking/output blocks already land via eprint_log!).
        if verbose {
            tracing::info!("[user] {user_text}");
        }

        // Broadcast the user message to all connected clients so a second TUI
        // observing the same session sees the question alongside the answer.
        // Use display_text (first text block only): text() also joins the
        // agent-injected attachment manifest, which observers would render
        // as a bogus extra bubble.
        self.broadcaster.broadcast(crate::rpc::SseEvent::new(
            "user_message",
            serde_json::json!({"text": user_display_text}),
        ));

        // Persist immediately so the GUI can see the user message (and any
        // tool entries from prior turns) during streaming. Without this, a
        // thread switch mid-stream loses the question until the run settles
        // because get_session_entries reads from disk.
        // Ephemeral sessions (--no-session) skip persistence entirely.
        if !self.ephemeral {
            self.persist_user_message();
        }

        // Set streaming flag + start a new run. P1: run_id is assigned once per
        // user run at the is_streaming false→true edge (resets idx + event buffer);
        // every event this run then carries the same run_id.
        self.is_streaming
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.broadcaster.start_run(crate::utils::generate_id());

        // Swap per-session token counters into the agent loop so updates are tracked per-session
        self.swap_token_counters_into_loop();

        // Wire auto-compaction transform (checked before each turn)
        self.wire_auto_compaction();

        // Clone shared state for the background task
        let messages_arc = self.messages.clone();
        let initial_messages = messages_arc.read().clone();
        let agent_loop = self.agent_loop.clone();
        let broadcaster = self.broadcaster.clone();
        let is_streaming = self.is_streaming.clone();
        let session_manager = self.session_manager.clone();
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
        let save_mgr = session_manager.clone();
        let save_sid = session_id.clone();
        let save_closure: crate::agent::PersistCallback =
            Arc::new(move |msg: &crate::types::AgentMessage| {
                if is_ephemeral {
                    return;
                }
                save_messages.write().push(msg.clone());
                let entry = crate::session::agent_message_to_entry(msg);
                if let Err(e) = save_mgr.append_entries(&save_sid, &[entry]) {
                    tracing::error!("Failed to append entry: {}", e);
                }
            });
        let user_msg_cb: crate::agent::PersistCallback = {
            let b = broadcaster.clone();
            Arc::new(move |msg: &crate::types::AgentMessage| {
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
            model: agent_loop
                .try_read()
                .ok()
                .map(|l| l.model.clone())
                .unwrap_or_else(|| session_model.clone()),
            system_prompt,
            on_tool_result: Some(save_closure.clone()),
            save_callback: Some(save_closure),
            tool_event_callback: tool_event_cb,
            on_user_message: Some(user_msg_cb),
        };

        // Set approval/sandbox hooks on this session's Loop config (these
        // are not callbacks — they're tool-execution hooks on AgentConfig).
        if let Ok(mut r#loop) = agent_loop.try_write() {
            let approval_gate_hook = approval_gate.clone();
            let approval_broadcaster = broadcaster.clone();
            let approval_session_id = session_id.clone();
            let approval_cwd = session_cwd.clone();
            let approval_sandbox = sandbox.clone();
            let permission_level = self.permission_level.clone();
            r#loop.config.before_tool_call = Some(Arc::new(
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
            r#loop.config.prepare_tool_call = Some(Arc::new(move |tool_name, arguments| {
                prepare_session_tool_call(&prepare_cwd, tool_name, arguments)
            }));
        }

        // agent_start is now emitted inside run_streaming_with_messages via on_event,
        // for both initial prompts and follow-up turns.

        // Clear any stale interrupt flag left by a previous abort().
        // abort() sets interrupt_flag=true on this session's own agent_loop.
        // Without clearing it, the spawned task's first loop iteration would
        // exit immediately without calling the LLM.
        let shared_interrupt_flag = if let Ok(r#loop) = self.agent_loop.try_read() {
            r#loop.clear_interrupt();
            r#loop.interrupt_flag()
        } else {
            // Fallback: if we can't acquire the lock, create a fresh flag.
            // This path is unlikely in practice because we hold no concurrent writers here.
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false))
        };

        // Create interrupt channel so steer()/abort() can stop the current stream
        let (interrupt_tx, interrupt_rx) = tokio::sync::mpsc::channel::<()>(1);
        self.interrupt_tx = Some(interrupt_tx);
        // Capture the loop's interrupt flag so abort() can set it without the
        // agent_loop lock (which the spawned run below holds for its duration).
        self.interrupt_flag = Some(shared_interrupt_flag.clone());

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

        // Spawn background task to run agent loop
        let perm = self.permission_level.clone();
        let scope_sandbox = sandbox.clone();
        tokio::spawn(async move {
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
                        let r#loop = agent_loop.read().await;

                        match r#loop
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
                                    be.broadcast(crate::rpc::SseEvent {
                                        event_type: event.event_type.clone(),
                                        data: stream_event_to_sse_data(&event),
                                        ..Default::default()
                                    });
                                },
                                current_interrupt_rx.take(),
                            )
                            .await
                        {
                            Ok((_, final_messages)) => {
                                current_messages = final_messages;

                                let follow_ups = r#loop.follow_up_queue.drain();
                                drop(r#loop);

                                if follow_ups.is_empty() {
                                    return Ok(current_messages);
                                }
                                for msg in follow_ups {
                                    let text = msg.clone();
                                    current_messages.push(crate::types::AgentMessage::new_user(
                                        "user",
                                        serde_json::json!([{"type": "text", "text": text}]),
                                    ));
                                    // Broadcast the follow-up so observing TUIs see it
                                    // alongside the assistant's response (same as prompt()).
                                    broadcaster.broadcast(crate::rpc::SseEvent::new(
                                        "user_message",
                                        serde_json::json!({"text": msg}),
                                    ));
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

            match result {
                Ok(final_messages) => {
                    // Update shared messages so next prompt includes the full context
                    *messages_arc.write() = final_messages;
                    // Save session to disk
                    {
                        let msgs = messages_arc.read();
                        let mut entries: Vec<crate::session::SessionEntry> = msgs
                            .iter()
                            .map(crate::session::agent_message_to_entry)
                            .collect();

                        // The whole session is rebuilt from the in-memory message
                        // list on every save, and agent_message_to_entry re-stamps
                        // `now()` with zero token/duration. Without preserving them,
                        // every reload shows all messages at the current time
                        // ("just now") and drops earlier replies' token counts.
                        // Messages only grow by appending, so the on-disk message
                        // entries align by index with this prefix. Filter the old
                        // side to message entries only (matching what the rebuild
                        // produces) so any interleaved label/model_change entries
                        // can't shift the alignment.
                        {
                            let is_message_entry = |t: &str| {
                                matches!(
                                    t,
                                    crate::session::ENTRY_TYPE_USER
                                        | crate::session::ENTRY_TYPE_ASSISTANT
                                        | crate::session::ENTRY_TYPE_TOOL
                                        | crate::session::ENTRY_TYPE_SYSTEM
                                )
                            };
                            let old_msg_entries: Vec<crate::session::SessionEntry> =
                                session_manager
                                    .load(&session_id)
                                    .map(|s| {
                                        s.entries
                                            .into_iter()
                                            .filter(|e| is_message_entry(&e.entry_type))
                                            .collect()
                                    })
                                    .unwrap_or_default();
                            for (new_entry, old_entry) in
                                entries.iter_mut().zip(old_msg_entries.iter())
                            {
                                new_entry.timestamp = old_entry.timestamp;
                                // Preserve run stats from old entry's content
                                if let Some(ref old_content) = old_entry.content {
                                    if let Some(obj) =
                                        new_entry.content.as_mut().and_then(|c| c.as_object_mut())
                                    {
                                        if let Some(v) = old_content.get("run_tokens") {
                                            obj.insert("run_tokens".to_string(), v.clone());
                                        }
                                        if let Some(v) = old_content.get("run_duration_ms") {
                                            obj.insert("run_duration_ms".to_string(), v.clone());
                                        }
                                    }
                                }
                            }

                            // Attach this run's output tokens + wall-clock duration
                            // to the final assistant entry (the reply just made). It
                            // sits beyond the preserved prefix, so earlier replies
                            // are untouched.
                            let run_output_tokens =
                                (tokens_out.load(std::sync::atomic::Ordering::Relaxed) - out_start)
                                    .max(0);
                            let run_duration_ms = run_start.elapsed().as_millis() as i64;
                            if let Some(last_assistant) = entries
                                .iter_mut()
                                .rev()
                                .find(|e| e.entry_type == crate::session::ENTRY_TYPE_ASSISTANT)
                            {
                                // Store run stats in the last assistant's content JSON
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
                        }

                        // Prepend session_info entry with metadata
                        use std::sync::atomic::Ordering;
                        // Preserve parent_session_id from existing session on disk
                        let parent_session_id = session_manager
                            .load(&session_id)
                            .map(|s| s.parent_session_id)
                            .unwrap_or_default();

                        // Auto-generate session_name from the first user message
                        // if not explicitly set (matches first_message in list_sessions).
                        let session_name = if session_name.is_empty() {
                            entries
                                .iter()
                                .find(|e| e.role == "user")
                                .and_then(|e| e.content.as_ref())
                                .map(|c| {
                                    if let Some(arr) = c.as_array() {
                                        // Only the user's own text (first text block);
                                        // a later text block is the agent-injected
                                        // attachment-path list, which must not leak
                                        // into the auto-generated session name.
                                        arr.iter()
                                            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                                            .next()
                                            .unwrap_or("")
                                            .to_string()
                                    } else {
                                        c.as_str().unwrap_or("").to_string()
                                    }
                                })
                                .map(|s| {
                                    let trimmed = s.trim();
                                    crate::session::truncate_visible(trimmed, 40)
                                })
                                .unwrap_or_default()
                        } else {
                            session_name
                        };

                        let total_cost = cumulative_cost.lock();
                        let mut info = serde_json::json!({
                            "cwd": session_cwd,
                            "tokens_in": tokens_in.load(Ordering::Relaxed),
                            "tokens_out": tokens_out.load(Ordering::Relaxed),
                            "tokens_cache_r": tokens_cache_r.load(Ordering::Relaxed),
                            "tokens_cache_w": tokens_cache_w.load(Ordering::Relaxed),
                            "last_prompt_tokens": last_prompt.load(Ordering::Relaxed),
                            "total_cost": *total_cost,
                            "session_name": session_name,
                            "auto_compaction": auto_compaction,
                            "parent_session_id": parent_session_id,
                            "thinking_level": session_thinking.clone(),
                            "model": session_model.clone(),
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
                        entries.insert(0, info_entry);

                        // If the first user message is a compaction marker, replace
                        // it with a proper compaction entry so the JSONL records
                        // the compaction point explicitly.
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
                                // Build a clean compaction entry — keep the summary
                                // text but convert from message array to a simple
                                // JSON object.
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
                                comp_entry.entry_type =
                                    crate::session::ENTRY_TYPE_COMPACTION.to_string();
                                comp_entry.role = "system".to_string();
                                comp_entry.content = Some(serde_json::json!({
                                    "summary": summary,
                                    "tokens_in": tokens_in.load(Ordering::Relaxed),
                                    "tokens_out": tokens_out.load(Ordering::Relaxed),
                                }));
                                entries.insert(idx + 1, comp_entry);
                                entries.remove(idx);
                            }
                        }

                        if !is_ephemeral {
                            let session = crate::session::Session::snapshot(
                                session_id.clone(),
                                session_cwd.clone(),
                                session_model.clone(),
                                session_name.clone(),
                                parent_session_id,
                                entries,
                            );
                            if let Err(e) = session_manager.save(&session) {
                                tracing::error!("Failed to save session: {}", e);
                            }
                        }
                    }
                    // Carry this run's output-token total on the terminal event so
                    // the client can show the token stat the instant the run settles.
                    // The run loop's rich usage (usage events + agent_end usage) only
                    // reaches the internal EventBus, which is never bridged to this SSE
                    // broadcaster — so without this the count is 0 until a reload reads
                    // it from the session entry. Same figure persisted at line ~574.
                    let run_output_tokens =
                        (tokens_out.load(std::sync::atomic::Ordering::Relaxed) - out_start).max(0);
                    broadcaster.broadcast(crate::rpc::SseEvent {
                        event_type: "agent_end".to_string(),
                        data: serde_json::json!({
                            "type": "agent_end",
                            "usage": { "output_tokens": run_output_tokens }
                        })
                        .to_string(),
                        ..Default::default()
                    });
                    is_streaming.store(false, std::sync::atomic::Ordering::Relaxed);
                }
                Err(e) => {
                    let full_error = format!("{:#}", e);
                    tracing::error!("Agent loop error: {}", full_error);
                    broadcaster.broadcast(crate::rpc::SseEvent {
                        event_type: "error".to_string(),
                        data: serde_json::json!({"error": &full_error}).to_string(),
                        ..Default::default()
                    });
                    let run_output_tokens =
                        (tokens_out.load(std::sync::atomic::Ordering::Relaxed) - out_start).max(0);
                    broadcaster.broadcast(crate::rpc::SseEvent {
                        event_type: "agent_end".to_string(),
                        data: serde_json::json!({
                            "type": "agent_end",
                            "error": &full_error,
                            "usage": { "output_tokens": run_output_tokens }
                        })
                        .to_string(),
                        ..Default::default()
                    });
                    is_streaming.store(false, std::sync::atomic::Ordering::Relaxed);
                }
            }
        });

        Ok(())
    }
    /// Build this turn's system prompt: project context (CLAUDE.md/AGENTS.md/
    /// GEMINI.md), workspace memory (FUTURE.md), discovered skills, and the
    /// write/memory guidelines. Read fresh each turn (cwd-scoped).
    /// Point the agent loop's cumulative token/cost counters at this session's
    /// shared atomics so streaming updates are tracked per-session.
    fn swap_token_counters_into_loop(&self) {
        if let Ok(mut r#loop) = self.agent_loop.try_write() {
            r#loop.cumulative_input_tokens = self.tokens_in.clone();
            r#loop.cumulative_output_tokens = self.tokens_out.clone();
            r#loop.cumulative_cache_read_tokens = self.tokens_cache_r.clone();
            r#loop.cumulative_cache_write_tokens = self.tokens_cache_w.clone();
            r#loop.cumulative_cost = self.cumulative_cost.clone();
            r#loop.last_prompt_tokens = self.last_prompt_tokens.clone();
        }
    }

    /// Install the pre-turn auto-compaction transform on the agent loop (a
    /// no-op when auto-compaction is off), compacting context once usage
    /// crosses ~90% of the model's window.
    fn wire_auto_compaction(&self) {
        if self.auto_compaction {
            if let Ok(mut r#loop) = self.agent_loop.try_write() {
                let comp_tokens = self.last_prompt_tokens.clone();
                let comp_result = r#loop.last_compaction_result.clone();
                let comp_failed = r#loop.compaction_failed.clone();
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
    }

    fn build_system_prompt(&self, tools: Vec<crate::types::AgentTool>) -> String {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();

        // Discover skills so they appear in the system prompt's <available_skills> block.
        let skill_dirs = vec![
            crate::skills::APP_SKILLS_DIR.to_string(),
            format!("{}/{}", self.cwd, crate::skills::PROJECT_SKILLS_DIR),
            crate::skills::AGENTS_SKILLS_DIR.to_string(),
        ];
        let skills = crate::skills::discover_skills(&skill_dirs).unwrap_or_default();

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

    /// Persist the current session snapshot (entries + prepended session_info)
    /// so the GUI sees the just-pushed user message mid-stream. Best-effort:
    /// a save failure is logged, not propagated.
    fn persist_user_message(&self) {
        let msgs = self.messages.read();
        let mut entries: Vec<crate::session::SessionEntry> = msgs
            .iter()
            .map(crate::session::agent_message_to_entry)
            .collect();
        let parent_session_id = self
            .session_manager
            .load(&self.session_id)
            .map(|s| s.parent_session_id)
            .unwrap_or_default();
        // Prepend session_info so token counts and other metadata survive
        // a crash — without this, a restarted session starts with zeroed
        // token counters and may skip needed compaction.
        {
            use std::sync::atomic::Ordering;
            // Derive session_name: prefer the explicitly-set name; fall back
            // to the first user message so the mid-stream save doesn't write
            // an empty name that would leak into a subsequent fork.
            let session_name = if !self.session_name.is_empty() {
                self.session_name.clone()
            } else {
                // Same auto-generation logic as the final save below.
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
        let session = crate::session::Session::snapshot(
            self.session_id.clone(),
            self.cwd.clone(),
            self.model.clone(),
            self.session_name.clone(),
            parent_session_id,
            entries,
        );
        if let Err(e) = self.session_manager.save(&session) {
            tracing::error!("Failed to persist user message: {}", e);
        }
    }
}

#[cfg(test)]
mod build_user_message_tests;
#[cfg(test)]
mod tests;
