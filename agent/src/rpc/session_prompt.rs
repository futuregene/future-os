use anyhow::Result;
use std::{
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use super::ServerSession;

impl ServerSession {
    pub fn prompt(
        &mut self,
        msg: &str,
        images: &[crate::types::ImageContent],
        _behavior: &str,
    ) -> Result<()> {
        std::fs::create_dir_all(&self.cwd)?;
        if let Ok(mut r#loop) = self.agent_loop.try_write() {
            let today = chrono::Local::now().format("%Y-%m-%d").to_string();

            // Discover skills so they appear in the system prompt's <available_skills> block.
            let skill_dirs = vec![
                crate::skills::APP_SKILLS_DIR.to_string(),
                format!("{}/{}", self.cwd, crate::skills::PROJECT_SKILLS_DIR),
                crate::skills::AGENTS_SKILLS_DIR.to_string(),
            ];
            let skills = crate::skills::discover_skills(&skill_dirs).unwrap_or_default();

            // Load project context (CLAUDE.md / AGENTS.md / GEMINI.md)
            let mut agent_content = String::new();
            for fname in &["CLAUDE.md", "AGENTS.md", "GEMINI.md"] {
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

            let system_prompt = crate::prompt::build_prompt(&crate::prompt::PromptOptions {
                working_directory: self.cwd.clone(),
                date: today,
                tools: r#loop.tools.clone(),
                skills,
                agent_content,
                memory_content,
                prompt_guidelines: vec![
                    "When asked to create, save, write, or modify a normal file, prefer the write or edit tool. Bash redirection, heredocs, tee, or cat > file remain available when they are the better fit. Only describe file changes after the tool succeeds.".to_string(),
                    "You maintain a workspace memory file named FUTURE.md in the working directory. Record a memory when the user explicitly asks you to remember something, and also proactively when you learn a durable, high-value fact about this workspace: a verified build/test/run/lint command, a stated user preference, a correction the user made (especially a repeated one), or a stable project convention. Do not record one-off task details, transient state, secrets, unverified guesses, or anything already derivable from the repo. Use the write or edit tool; keep entries short and grouped under markdown headers; update or remove stale entries instead of duplicating; keep the file concise (aim under ~200 lines). Whenever you write to memory, tell the user in one short line what you recorded. Memory may only be written to FUTURE.md — never to CLAUDE.md, AGENTS.md, or GEMINI.md.".to_string(),
                ],
                ..Default::default()
            });
            r#loop.system_prompt = system_prompt.clone();
            r#loop.config.system_prompt = system_prompt;
        }

        // Build content blocks: text + images
        let mut content: Vec<serde_json::Value> = Vec::new();
        content.push(serde_json::json!({"type": "text", "text": msg}));
        for img in images {
            let url = img.data.as_deref().unwrap_or("");
            if !url.is_empty() {
                content.push(serde_json::json!({
                    "type": "image_url",
                    "image_url": {"url": url}
                }));
            }
        }
        self.messages
            .write()
            .unwrap()
            .push(crate::types::AgentMessage::new_user(
                "user",
                serde_json::Value::Array(content),
            ));

        // Set streaming flag
        self.is_streaming
            .store(true, std::sync::atomic::Ordering::Relaxed);

        // Swap per-session token counters into the agent loop so updates are tracked per-session
        {
            if let Ok(mut r#loop) = self.agent_loop.try_write() {
                r#loop.cumulative_input_tokens = self.tokens_in.clone();
                r#loop.cumulative_output_tokens = self.tokens_out.clone();
                r#loop.cumulative_cache_read_tokens = self.tokens_cache_r.clone();
                r#loop.cumulative_cache_write_tokens = self.tokens_cache_w.clone();
                r#loop.last_prompt_tokens = self.last_prompt_tokens.clone();
            }
        }

        // Wire auto-compaction transform (checked before each turn)
        if self.auto_compaction {
            if let Ok(mut r#loop) = self.agent_loop.try_write() {
                let comp_tokens = self.last_prompt_tokens.clone();
                let comp_model = self.model.clone();
                let comp_result = r#loop.last_compaction_result.clone();
                r#loop.config.transform_context = Some(Arc::new(move |msgs, _| {
                    use std::sync::atomic::Ordering;
                    let context_tokens = comp_tokens.load(Ordering::Relaxed) as i32;
                    if context_tokens == 0 {
                        return msgs; // No API call made yet, nothing to compact
                    }
                    let context_window = crate::models::Registry::new()
                        .resolve(&comp_model)
                        .map(|m| m.context_window)
                        .unwrap_or(200000);
                    // Compact when context usage exceeds 90% (10% reserve, min 16K)
                    let reserve_tokens = ((context_window as f64 * 0.1) as i32).max(16384);
                    let (compacted, result) = crate::compaction::compact(
                        msgs,
                        &crate::compaction::CompactOptions {
                            reserve_tokens,
                            keep_recent_tokens: reserve_tokens,
                            context_window,
                            tokens_before: context_tokens,
                        },
                    );
                    if let Some(r) = result {
                        *comp_result.lock().unwrap() = Some(r);
                        compacted
                    } else {
                        compacted
                    }
                }));
            }
        }

        // Clone shared state for the background task
        let messages_arc = self.messages.clone();
        let initial_messages = messages_arc.read().unwrap().clone();
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
        let last_prompt = self.last_prompt_tokens.clone();
        let session_name = self.session_name.clone();
        let auto_compaction = self.auto_compaction;
        let approval_gate = self.approval_gate.clone();

        // Set tool event callback so tool_start/tool_end reach the TUI
        {
            let broadcaster_tool = broadcaster.clone();
            if let Ok(mut r#loop) = agent_loop.try_write() {
                r#loop.tool_event_callback =
                    Some(Arc::new(move |event: crate::types::StreamEvent| {
                        let mut data = serde_json::Map::new();
                        data.insert("type".to_string(), serde_json::json!(&event.event_type));
                        if !event.tool_name.is_empty() {
                            data.insert(
                                "tool_name".to_string(),
                                serde_json::json!(&event.tool_name),
                            );
                        }
                        if !event.tool_id.is_empty() {
                            data.insert("tool_id".to_string(), serde_json::json!(&event.tool_id));
                        }
                        if !event.text.is_empty() {
                            data.insert("text".to_string(), serde_json::json!(&event.text));
                        }
                        if !event.error_text.is_empty() {
                            data.insert("error".to_string(), serde_json::json!(&event.error_text));
                        }
                        if let Some(ref tc) = event.tool_call {
                            data.insert("tool_args".to_string(), tc.function.arguments.clone());
                        }
                        broadcaster_tool.broadcast(crate::rpc::SseEvent {
                            event_type: event.event_type.clone(),
                            data: serde_json::to_string(&data).unwrap_or_default(),
                        });
                    }));
                let approval_gate = approval_gate.clone();
                let approval_broadcaster = broadcaster.clone();
                let approval_session_id = session_id.clone();
                let approval_cwd = session_cwd.clone();
                let permission_level = self.permission_level.clone();
                r#loop.config.before_tool_call = Some(Arc::new(
                    move |tool_name, tool_id, arguments| {
                        match permission_level.as_str() {
                            "all" => {
                                approve_tool_path_if_present(
                                    &approval_cwd,
                                    tool_name,
                                    arguments,
                                );
                                None
                            }
                            "none" => Some(crate::types::ToolCallResult {
                                result: format!("Tool call `{tool_name}` denied: permission level is set to 'none'."),
                                is_error: true,
                            }),
                            _ => approval_gate.request(
                                &approval_broadcaster,
                                &approval_session_id,
                                &approval_cwd,
                                tool_name,
                                tool_id,
                                arguments,
                            ),
                        }
                    },
                ));
                let prepare_cwd = session_cwd.clone();
                r#loop.config.prepare_tool_call = Some(Arc::new(move |tool_name, arguments| {
                    prepare_session_tool_call(&prepare_cwd, tool_name, arguments)
                }));
            }
        }

        // agent_start is now emitted inside run_streaming_with_messages via on_event,
        // for both initial prompts and follow-up turns.

        // Clear any stale interrupt flag left by a previous abort().
        // Ctrl+C / abort sets interrupt_flag=true on the shared agent_loop.
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

        // Spawn background task to run agent loop
        let perm = self.permission_level.clone();
        tokio::spawn(async move {
            let result = crate::tools::with_workspace_scope_with_interrupt(
                session_cwd.clone(),
                perm,
                shared_interrupt_flag,
                async {
                    let mut current_messages = initial_messages;
                    let mut current_interrupt_rx = Some(interrupt_rx);

                    loop {
                        let bt = broadcaster.clone();
                        let be = broadcaster.clone();
                        let r#loop = agent_loop.write().await;

                        match r#loop
                            .run_streaming_with_messages(
                                current_messages,
                                move |text| {
                                    bt.broadcast(crate::rpc::SseEvent {
                                        event_type: "text_chunk".to_string(),
                                        data: serde_json::json!({"text": text}).to_string(),
                                    });
                                },
                                move |event| {
                                    let mut data = serde_json::Map::new();
                                    data.insert(
                                        "type".to_string(),
                                        serde_json::json!(&event.event_type),
                                    );
                                    if !event.text.is_empty() {
                                        data.insert(
                                            "text".to_string(),
                                            serde_json::json!(&event.text),
                                        );
                                    }
                                    if !event.tool_name.is_empty() {
                                        data.insert(
                                            "tool_name".to_string(),
                                            serde_json::json!(&event.tool_name),
                                        );
                                    }
                                    if !event.tool_id.is_empty() {
                                        data.insert(
                                            "tool_id".to_string(),
                                            serde_json::json!(&event.tool_id),
                                        );
                                    }
                                    if !event.error_text.is_empty() {
                                        data.insert(
                                            "error".to_string(),
                                            serde_json::json!(&event.error_text),
                                        );
                                    }
                                    if !event.stop_reason.is_empty() {
                                        data.insert(
                                            "stopReason".to_string(),
                                            serde_json::json!(&event.stop_reason),
                                        );
                                    }
                                    if let Some(usage) = &event.usage {
                                        data.insert("usage".to_string(), serde_json::json!(usage));
                                    }
                                    if let Some(ref tc) = event.tool_call {
                                        data.insert(
                                            "tool_args".to_string(),
                                            tc.function.arguments.clone(),
                                        );
                                    }
                                    be.broadcast(crate::rpc::SseEvent {
                                        event_type: event.event_type.clone(),
                                        data: serde_json::to_string(&data).unwrap_or_default(),
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
                                    current_messages.push(crate::types::AgentMessage::new_user(
                                        "user",
                                        serde_json::json!([{"type": "text", "text": msg}]),
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
                    match messages_arc.write() {
                        Ok(mut msgs) => {
                            *msgs = final_messages;
                        }
                        Err(e) => {
                            let mut msgs = e.into_inner();
                            *msgs = final_messages;
                        }
                    }
                    // Save session to disk
                    {
                        let msgs = messages_arc.read().unwrap();
                        let mut entries: Vec<crate::session::SessionEntry> = msgs
                            .iter()
                            .map(crate::session::agent_message_to_entry)
                            .collect();

                        // Prepend session_info entry with metadata
                        use std::sync::atomic::Ordering;
                        // Preserve parent_session_id from existing session on disk
                        let parent_session_id = session_manager
                            .load(&session_id)
                            .map(|s| s.parent_session_id)
                            .unwrap_or_default();
                        let info = serde_json::json!({
                            "cwd": session_cwd,
                            "model": session_model,
                            "thinking_level": session_thinking,
                            "tokens_in": tokens_in.load(Ordering::Relaxed),
                            "tokens_out": tokens_out.load(Ordering::Relaxed),
                            "tokens_cache_r": tokens_cache_r.load(Ordering::Relaxed),
                            "tokens_cache_w": tokens_cache_w.load(Ordering::Relaxed),
                            "last_prompt_tokens": last_prompt.load(Ordering::Relaxed),
                            "session_name": session_name,
                            "auto_compaction": auto_compaction,
                            "parent_session_id": parent_session_id,
                        });
                        let info_entry = crate::session::SessionEntry {
                            id: crate::utils::generate_entry_id(),
                            parent_id: String::new(),
                            entry_type: crate::session::ENTRY_TYPE_SESSION_INFO.to_string(),
                            role: "system".to_string(),
                            content: Some(info),
                            tool_calls: vec![],
                            timestamp: chrono::Local::now(),
                            summary: String::new(),
                            model: session_model.clone(),
                            label: String::new(),
                            thinking_level: session_thinking.clone(),
                            branch_summary: None,
                            custom_type: String::new(),
                            custom_data: None,
                            display: String::new(),
                            provider: String::new(),
                            tool_call_id: String::new(),
                            name: String::new(),
                            tool_args: String::new(),
                            thinking: String::new(),
                        };
                        entries.insert(0, info_entry);

                        let session = crate::session::Session {
                            id: session_id.clone(),
                            version: crate::session::CURRENT_SESSION_VERSION,
                            cwd: session_cwd.clone(),
                            model: session_model.clone(),
                            base_url: String::new(),
                            name: String::new(),
                            parent_session_id,
                            leaf_id: String::new(),
                            entries,
                            created_at: chrono::Local::now(),
                            updated_at: chrono::Local::now(),
                        };
                        if let Err(e) = session_manager.save(&session) {
                            eprintln!("Failed to save session: {}", e);
                        }
                    }
                    broadcaster.broadcast(crate::rpc::SseEvent {
                        event_type: "agent_end".to_string(),
                        data: serde_json::json!({"type": "agent_end"}).to_string(),
                    });
                    is_streaming.store(false, std::sync::atomic::Ordering::Relaxed);
                }
                Err(e) => {
                    let full_error = format!("{:#}", e);
                    eprintln!("Agent loop error: {}", full_error);
                    broadcaster.broadcast(crate::rpc::SseEvent {
                        event_type: "error".to_string(),
                        data: serde_json::json!({"error": &full_error}).to_string(),
                    });
                    broadcaster.broadcast(crate::rpc::SseEvent {
                        event_type: "agent_end".to_string(),
                        data: serde_json::json!({"type": "agent_end", "error": &full_error})
                            .to_string(),
                    });
                    is_streaming.store(false, std::sync::atomic::Ordering::Relaxed);
                }
            }
        });

        Ok(())
    }
}

fn prepare_session_tool_call(
    cwd: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
) -> serde_json::Value {
    let mut normalized = match arguments {
        serde_json::Value::String(raw) => {
            serde_json::from_str::<serde_json::Value>(raw).unwrap_or(arguments.clone())
        }
        _ => arguments.clone(),
    };

    match tool_name {
        "read" | "write" | "edit" | "ls" => {
            rewrite_path_field(cwd, &mut normalized, "path");
        }
        "grep" => {
            rewrite_path_field(cwd, &mut normalized, "path");
        }
        _ => {}
    }

    normalized
}

fn approve_tool_path_if_present(cwd: &str, tool_name: &str, arguments: &serde_json::Value) {
    if !matches!(tool_name, "write" | "edit") {
        return;
    }

    let Some(path) = argument_path(arguments) else {
        return;
    };

    crate::tools::approve_outside_path(&resolve_workspace_path(cwd, &path));
}

fn argument_path(arguments: &serde_json::Value) -> Option<String> {
    let normalized = match arguments {
        serde_json::Value::String(raw) => {
            serde_json::from_str::<serde_json::Value>(raw).unwrap_or(arguments.clone())
        }
        _ => arguments.clone(),
    };

    ["path", "file_path", "filePath"]
        .iter()
        .find_map(|key| normalized.get(*key).and_then(|value| value.as_str()))
        .map(str::to_string)
}

fn rewrite_path_field(cwd: &str, arguments: &mut serde_json::Value, key: &str) {
    let Some(path) = arguments.get(key).and_then(|value| value.as_str()) else {
        return;
    };
    arguments[key] = serde_json::Value::String(resolve_workspace_path(cwd, path));
}

fn resolve_workspace_path(cwd: &str, path: &str) -> String {
    let workspace = PathBuf::from(cwd);
    let candidate = if let Some(relative) = path.strip_prefix("~/") {
        workspace.join(relative)
    } else if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        workspace.join(path)
    };
    normalize_path(&candidate).to_string_lossy().to_string()
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agent::Loop,
        tools::coding_tools,
        types::{AgentMessage, LLMProvider, Message, StreamEvent, ToolCall, ToolCallFn, ToolDef},
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::mpsc;
    use tokio_stream::wrappers::ReceiverStream;

    struct MockWriteProvider {
        calls: AtomicUsize,
        outside_path: String,
    }

    #[async_trait::async_trait]
    impl LLMProvider for MockWriteProvider {
        async fn stream_chat(
            &self,
            _model: String,
            _messages: Vec<Message>,
            _tools: Vec<ToolDef>,
            _system_prompt: String,
        ) -> anyhow::Result<ReceiverStream<StreamEvent>> {
            let (tx, rx) = mpsc::channel(8);
            let call_index = self.calls.fetch_add(1, Ordering::SeqCst);
            let outside_path = self.outside_path.clone();

            tokio::spawn(async move {
                if call_index == 0 {
                    let arguments = serde_json::json!({
                        "path": outside_path,
                        "content": "should not leave workspace"
                    });
                    let _ = tx
                        .send(event_with_tool_call(
                            "toolcall_start",
                            "call_test",
                            "write",
                            arguments,
                        ))
                        .await;
                    let _ = tx.send(simple_event("toolcall_end")).await;
                } else {
                    let _ = tx.send(text_event("done")).await;
                    let _ = tx.send(simple_event("stop")).await;
                }
            });

            Ok(ReceiverStream::new(rx))
        }
    }

    fn simple_event(event_type: &str) -> StreamEvent {
        StreamEvent {
            event_type: event_type.to_string(),
            text: String::new(),
            tool_call: None,
            tool_name: String::new(),
            tool_id: String::new(),
            usage: None,
            stop_reason: String::new(),
            error_text: String::new(),
        }
    }

    fn text_event(text: &str) -> StreamEvent {
        StreamEvent {
            event_type: "text_delta".to_string(),
            text: text.to_string(),
            ..simple_event("text_delta")
        }
    }

    fn event_with_tool_call(
        event_type: &str,
        tool_id: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> StreamEvent {
        StreamEvent {
            event_type: event_type.to_string(),
            tool_name: tool_name.to_string(),
            tool_id: tool_id.to_string(),
            tool_call: Some(ToolCall {
                id: tool_id.to_string(),
                call_type: "function".to_string(),
                function: ToolCallFn {
                    name: tool_name.to_string(),
                    arguments,
                },
            }),
            ..simple_event(event_type)
        }
    }

    fn test_path(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("futureos-session-{name}-{stamp}"))
    }

    #[tokio::test]
    async fn loop_workspace_scope_blocks_unapproved_absolute_write_from_model_tool_call() {
        let workspace = test_path("workspace");
        let outside = test_path("outside.txt");
        std::fs::create_dir_all(&workspace).unwrap();

        let provider = Arc::new(MockWriteProvider {
            calls: AtomicUsize::new(0),
            outside_path: outside.to_string_lossy().to_string(),
        });
        let agent_loop = Loop::new(provider, "mock").with_tools(coding_tools());

        crate::tools::with_workspace_scope(
            workspace.to_string_lossy().to_string(),
            "workspace".to_string(),
            async {
                let _ = agent_loop
                    .run_streaming_with_messages(
                        vec![AgentMessage::new_user(
                            "user",
                            serde_json::json!([{"type": "text", "text": "write outside"}]),
                        )],
                        |_| {},
                        |_| {},
                        None,
                    )
                    .await;
            },
        )
        .await;

        assert!(!outside.exists());
    }
}
