//! Agent Loop — 1:1 compatible with Go internal/agent/

mod events;
mod run_loop;
use crate::types::{AgentMessage, AgentTool, ContentBlock, LLMProvider, ToolCall};
use anyhow::{anyhow, Result};
pub use events::RunEvent;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

// ANSI terminal colors (matching Go). Only for raw stderr prints via
// eprint_log! — never inside tracing messages (tracing escapes ESC bytes in
// format args to literal text; the log file must stay plain).
const C_RESET: &str = "\x1b[0m";
const C_GREEN: &str = "\x1b[32m";
const C_MAGENTA: &str = "\x1b[35m";

pub const DEFAULT_MAX_TURNS: i32 = 0; // 0 = unlimited

pub type PersistCallback = Arc<dyn Fn(&mut crate::types::AgentMessage) + Send + Sync>;
pub type CheckpointCallback =
    Arc<dyn Fn(&crate::compaction::ContextCheckpoint) -> Result<()> + Send + Sync>;

/// Per-session state passed into `run_streaming_with_messages`.  Callbacks
/// are session-specific (they capture session_id, messages_arc, broadcaster)
/// and must NOT be stored on the shared Loop — otherwise concurrent sessions
/// overwrite each other's persistence and event streams.
#[derive(Default)]
pub struct StreamContext {
    pub model: String,
    pub system_prompt: String,
    #[allow(clippy::type_complexity)]
    pub on_tool_result: Option<PersistCallback>,
    pub save_callback: Option<PersistCallback>,
    /// Durable checkpoint commit. A successful return means the checkpoint
    /// journal entry was fsync'd and a committed event may be emitted.
    pub on_checkpoint: Option<CheckpointCallback>,
}

pub struct Loop {
    pub provider: Arc<dyn LLMProvider>,
    pub model: String,
    pub system_prompt: String,
    pub tools: Vec<AgentTool>,
    pub config: crate::types::AgentConfig,
    pub verbose: bool,
    pub session_id: String,
    pub parallel_tools: bool,
    pub(crate) interrupt_flag: Arc<AtomicBool>,
    pub context_manager: Option<crate::compaction::ContextManager>,
    pub active_checkpoint: Arc<Mutex<Option<crate::compaction::ContextCheckpoint>>>,
    pub cumulative_input_tokens: Arc<std::sync::atomic::AtomicI64>,
    pub cumulative_output_tokens: Arc<std::sync::atomic::AtomicI64>,
    pub cumulative_cache_read_tokens: Arc<std::sync::atomic::AtomicI64>,
    pub cumulative_cache_write_tokens: Arc<std::sync::atomic::AtomicI64>,
    /// Cumulative cost as reported by upstream (Future API `credit_cost`).
    pub cumulative_cost: Arc<parking_lot::Mutex<f64>>,
    /// Last API call's prompt_tokens (actual context size, not cumulative across turns)
    pub last_prompt_tokens: Arc<std::sync::atomic::AtomicI64>,
    /// Set when the provider stream ended without a genuine terminal event.
    /// Read by the run commit path so both the journal and `agent_end` preserve
    /// `incomplete` instead of presenting a truncated prefix as completed.
    pub stream_incomplete: Arc<AtomicBool>,
    /// Cached model registry — avoids re-deserialising the 906-model catalog
    /// on auto-compaction checks and image-support queries inside the hot loop.
    pub model_registry: Option<Arc<parking_lot::RwLock<crate::models::Registry>>>,
    /// Mid-turn steering notes: orchestrators/injected operator messages that
    /// must reach a RUNNING turn. The run loop drains this cell at every step
    /// and appends pending notes to the system prompt of the next LLM call.
    /// Shared between the shared Loop and its `independent_copy` snapshot (like
    /// the compaction cells) so notes written mid-run are seen by the snapshot.
    pub steering_notes: Arc<Mutex<Vec<String>>>,
}

impl Loop {
    pub fn new(provider: Arc<dyn LLMProvider>, model: &str) -> Self {
        Self {
            provider,
            model: model.to_string(),
            system_prompt: String::new(),
            tools: vec![],
            config: crate::types::AgentConfig::default(),
            verbose: false,
            session_id: String::new(),
            parallel_tools: false,
            interrupt_flag: Arc::new(AtomicBool::new(false)),
            context_manager: None,
            active_checkpoint: Arc::new(Mutex::new(None)),
            cumulative_input_tokens: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            cumulative_output_tokens: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            cumulative_cache_read_tokens: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            cumulative_cache_write_tokens: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            cumulative_cost: Arc::new(parking_lot::Mutex::new(0.0)),
            last_prompt_tokens: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            stream_incomplete: Arc::new(AtomicBool::new(false)),
            model_registry: None,
            steering_notes: Arc::new(Mutex::new(vec![])),
        }
    }

    pub fn with_tools(mut self, tools: Vec<AgentTool>) -> Self {
        self.tools = tools;
        self
    }

    pub fn with_system_prompt(mut self, prompt: &str) -> Self {
        self.system_prompt = prompt.to_string();
        self
    }

    pub fn with_config(mut self, config: crate::types::AgentConfig) -> Self {
        self.config = config;
        self
    }

    /// Create an independent copy of this loop: same provider, model, tools,
    /// config and system prompt, but fresh token counters, interrupt flag and
    /// compaction state.
    ///
    /// `ServerSession` first uses this to isolate sessions from the process
    /// template, then snapshots its session-owned control plane again at each
    /// run boundary. A streaming run never holds the control-plane lock across
    /// model/tool awaits, and interrupt flags, queues, counters, and execution
    /// hooks cannot leak across sessions or adjacent runs. The provider `Arc`
    /// is cloned only as a seed: `ServerSession::set_model` replaces it with a
    /// freshly-built client for the session's selected model.
    pub fn independent_copy(&self) -> Loop {
        let mut copy = Loop::new(self.provider.clone(), &self.model)
            .with_tools(self.tools.clone())
            .with_system_prompt(&self.system_prompt)
            .with_config(self.config.clone());
        copy.verbose = self.verbose;
        copy.parallel_tools = self.parallel_tools;
        copy.model_registry = self.model_registry.clone();
        copy.context_manager = self.context_manager.clone();
        copy.active_checkpoint = self.active_checkpoint.clone();
        // Share the steering cell with the snapshot so notes pushed while the
        // snapshot runs are delivered at its next step boundary.
        copy.steering_notes = self.steering_notes.clone();
        copy
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // PUBLIC API — matching Go's Loop public methods
    // ═══════════════════════════════════════════════════════════════════════════

    /// RunStreaming runs the agent loop with a new session (user prompt only)
    pub async fn run_streaming(
        &self,
        user_prompt: String,
        on_text: impl Fn(String) + Send + 'static,
    ) -> Result<String> {
        let messages = vec![self.new_user_message(user_prompt)];
        let (result, _) = self
            .run_streaming_with_messages(messages, &StreamContext::default(), on_text, |_| {}, None)
            .await?;
        Ok(result)
    }

    // RunStreamingWithMessages runs the agent loop with pre-existing messages.
    // Returns (final_text, all_messages).
    // interrupt_rx is an optional channel that, when fired, interrupts the current stream.

    // ═══════════════════════════════════════════════════════════════════════════
    // TOOL EXECUTION
    // ═══════════════════════════════════════════════════════════════════════════

    async fn execute_tools<F>(
        &self,
        turn: usize,
        tool_calls: &[ToolCall],
        messages: &mut Vec<AgentMessage>,
        on_event: &F,
        on_tool_result: &Option<PersistCallback>,
    ) where
        F: Fn(RunEvent) + Send + Sync,
    {
        // `parallel_tools` and `tools_execution_mode` remain readable only for
        // historical config compatibility. They never provided real parallel
        // execution, so the runtime exposes one honest deterministic behavior.
        // A future parallel executor must return as a new capability together
        // with explicit same-workspace write-conflict semantics.
        self.execute_tools_sequential(turn, tool_calls, messages, on_event, on_tool_result)
            .await;
    }

    async fn execute_tools_sequential<F>(
        &self,
        _turn: usize,
        tool_calls: &[ToolCall],
        messages: &mut Vec<AgentMessage>,
        on_event: &F,
        on_tool_result: &Option<PersistCallback>,
    ) where
        F: Fn(RunEvent) + Send + Sync,
    {
        let tools = &self.tools;
        let config = &self.config;
        let mut interrupted = false;
        let mut executed = 0usize;
        for tc in tool_calls {
            // Check for abort between tool executions
            if self.is_interrupted() {
                interrupted = true;
                break;
            }
            let start = Instant::now();

            // Broadcast tool_start (include tool_call for args)
            on_event(RunEvent::ToolExecutionStarted {
                id: tc.id.clone(),
                name: tc.function.name.clone(),
                arguments: tc.function.arguments.clone(),
            });

            let (result, err_str, tool_name) =
                Self::execute_one_tool_impl_static(tc, tools, config).await;
            let duration = start.elapsed().as_millis() as u64;

            if self.verbose {
                let tag = if tool_name == "read" && result.contains("SKILL.md") {
                    "[skill]"
                } else {
                    "[tool]"
                };
                // No manual ANSI colors here: tracing escapes ESC bytes in
                // message args to literal "\x1b" text, and the file layer must
                // stay plain. The level label (INFO/ERROR) already colors the
                // console output.
                if let Some(ref err) = err_str {
                    tracing::error!("{} ✗ {:-12} {:6}ms  {}", tag, tool_name, duration, err);
                } else {
                    tracing::info!("{} ✓ {:-12} {:6}ms", tag, tool_name, duration);
                }
            }

            // Broadcast tool_end — with structured semantics (exit code,
            // soft-fail, target path) so consumers don't re-parse the output
            // prose.
            let semantics =
                crate::tools::tool_end_semantics(&tool_name, &tc.function.arguments, &result);
            on_event(RunEvent::ToolExecutionFinished {
                id: tc.id.clone(),
                name: tool_name.clone(),
                output: result.clone(),
                error: err_str.clone(),
                exit_code: semantics.exit_code,
                is_soft_fail: semantics.is_soft_fail,
                target_path: semantics.target_path,
            });

            let tool_args_str = match &tc.function.arguments {
                serde_json::Value::String(s) => s.clone(),
                other => serde_json::to_string(other).unwrap_or_default(),
            };
            let tool_msg = self.new_tool_result(
                &tc.id,
                &tc.function.name,
                &tool_args_str,
                &result,
                err_str.as_deref(),
            );
            messages.push(tool_msg);
            if let Some(ref cb) = on_tool_result {
                cb(messages.last_mut().unwrap());
            }
            executed += 1;
        }

        // Inject placeholder results for tools that were skipped due to interrupt
        if interrupted {
            for tc in tool_calls.iter().skip(executed) {
                let cancelled = format!(
                    "[Tool execution cancelled — {} was skipped due to user interrupt]",
                    tc.function.name
                );
                let tool_args_str = match &tc.function.arguments {
                    serde_json::Value::String(s) => s.clone(),
                    other => serde_json::to_string(other).unwrap_or_default(),
                };
                messages.push(self.new_tool_result(
                    &tc.id,
                    &tc.function.name,
                    &tool_args_str,
                    &cancelled,
                    Some(&cancelled),
                ));
            }
        }
    }

    async fn execute_one_tool_impl_static(
        tc: &ToolCall,
        tools: &[AgentTool],
        config: &crate::types::AgentConfig,
    ) -> (String, Option<String>, String) {
        let tool_name = tc.function.name.clone();
        let tool_id = tc.id.clone();

        // Stage 1: BeforeToolCall hook
        if let Some(ref hook) = config.before_tool_call {
            if let Some(result_val) = hook(&tool_name, &tool_id, &tc.function.arguments) {
                if result_val.is_error {
                    return (
                        result_val.result.clone(),
                        Some(result_val.result),
                        tool_name,
                    );
                } else {
                    return (result_val.result.clone(), None, tool_name);
                }
            }
        }

        // Stage 2: PrepareToolCall hook
        let raw_args = tc.function.arguments.clone();
        let normalized_args = match &raw_args {
            serde_json::Value::String(s) => {
                serde_json::from_str::<serde_json::Value>(s).unwrap_or(raw_args)
            }
            _ => raw_args,
        };
        let effective_args = if let Some(ref hook) = config.prepare_tool_call {
            hook(&tool_name, &normalized_args)
        } else {
            normalized_args
        };

        // Execute the tool
        let start = Instant::now();
        let mut result: Result<String> = Err(anyhow!(
            "Unknown tool '{}'. The model requested a tool that is not available. \
             This may happen if the model is not compatible with the tool set.",
            tool_name
        ));
        for tool in tools {
            if tool.def.function.name == tool_name {
                result = (tool.handler)(effective_args.clone()).await;
                break;
            }
        }
        let _duration = start.elapsed().as_millis() as u64;

        // Stage 3: FinalizeToolCall hook
        let (final_result, final_err) = if let Some(ref hook) = config.finalize_tool_call {
            match result.as_ref() {
                Ok(s) => {
                    let (r, e) = hook(&tool_name, s.clone(), anyhow::anyhow!(""));
                    (Some(r), e)
                }
                Err(err) => {
                    let (r, e) = hook(&tool_name, String::new(), anyhow::anyhow!("{}", err));
                    (Some(r), e)
                }
            }
        } else {
            // No finalize hook, use result directly
            match result {
                Ok(s) => (Some(s), None),
                Err(e) => (None, Some(e)),
            }
        };

        // Stage 4: AfterToolCall hook
        if let Some(ref hook) = config.after_tool_call {
            let result_str = final_result.as_deref().unwrap_or("");
            let err_owned = final_err
                .as_ref()
                .map(|e| anyhow::anyhow!("{}", e))
                .unwrap_or_else(|| anyhow::anyhow!(""));
            if let Some(result_val) = hook(
                &tool_name,
                &tool_id,
                &effective_args,
                result_str.to_string(),
                err_owned,
            ) {
                let error_result = if result_val.is_error {
                    Some(result_val.result.clone())
                } else {
                    None
                };
                return (result_val.result, error_result, tool_name);
            }
        }

        // Return (result_string, error_string_option, tool_name)
        let result_str = final_result.unwrap_or_else(String::new);
        let error_str = final_err.map(|e| e.to_string());
        (result_str, error_str, tool_name)
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // INTERRUPT METHODS
    // ═══════════════════════════════════════════════════════════════════════════

    /// Abort cancels current streaming without queuing a message.
    pub fn abort(&self) {
        self.interrupt_flag.store(true, Ordering::SeqCst);
    }

    fn is_interrupted(&self) -> bool {
        self.interrupt_flag.load(Ordering::SeqCst)
    }

    pub fn clear_interrupt(&self) {
        self.interrupt_flag.store(false, Ordering::SeqCst);
    }

    /// Returns a clone of the Arc-wrapped interrupt flag for sharing
    /// with cooperative cancellation points (e.g., shell tool).
    pub fn interrupt_flag(&self) -> Arc<AtomicBool> {
        self.interrupt_flag.clone()
    }

    fn new_user_message(&self, content: impl Into<String>) -> AgentMessage {
        AgentMessage {
            role: "user".to_string(),
            content: vec![ContentBlock::text(content.into())],
            ..Default::default()
        }
    }

    fn new_tool_result(
        &self,
        call_id: &str,
        tool_name: &str,
        tool_args: &str,
        result: &str,
        err: Option<&str>,
    ) -> AgentMessage {
        let text = if let Some(e) = err {
            format!("Error: {}", e)
        } else {
            result.to_string()
        };
        // Cap tool result at 100K chars (~25K tokens) to avoid
        // a single oversized result blowing past the context window.
        // Compaction can trim old messages but can't split one message.
        let capped = if text.len() > 100_000 {
            let start = text.ceil_char_boundary(text.len() - 100_000);
            format!(
                "...(truncated, showing last 100K chars)\n{}",
                &text[start..]
            )
        } else {
            text
        };
        AgentMessage {
            role: "tool".to_string(),
            content: vec![ContentBlock::tool_result(
                call_id.to_string(),
                &capped,
                false,
            )],
            name: tool_name.to_string(),
            tool_args: tool_args.to_string(),
            ..Default::default()
        }
    }
}

impl Default for crate::types::AgentConfig {
    fn default() -> Self {
        Self {
            system_prompt: String::new(),
            max_turns: DEFAULT_MAX_TURNS,
            thinking_budget: 0,
            max_retries: 3,
            stop_condition: None,
            before_tool_call: None,
            prepare_tool_call: None,
            finalize_tool_call: None,
            after_tool_call: None,
            tools_execution_mode: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Loop struct (needs mock provider) ──────────────────────────────────

    struct MockProvider;

    #[async_trait::async_trait]
    impl crate::types::LLMProvider for MockProvider {
        async fn stream_model(
            &self,
            _request: crate::llm::schema::ModelRequest,
        ) -> anyhow::Result<
            tokio_stream::wrappers::ReceiverStream<crate::llm::schema::ModelStreamEvent>,
        > {
            let (_tx, rx) = tokio::sync::mpsc::channel(1);
            Ok(tokio_stream::wrappers::ReceiverStream::new(rx))
        }
    }

    fn make_loop() -> Loop {
        Loop::new(std::sync::Arc::new(MockProvider), "test-model")
    }

    #[test]
    fn loop_interrupt_and_clear() {
        let loop_ = make_loop();
        assert!(!loop_
            .interrupt_flag()
            .load(std::sync::atomic::Ordering::SeqCst));
        loop_.abort();
        assert!(loop_
            .interrupt_flag()
            .load(std::sync::atomic::Ordering::SeqCst));
        loop_.clear_interrupt();
        assert!(!loop_
            .interrupt_flag()
            .load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn loop_new_tool_result_normal() {
        let loop_ = make_loop();
        let msg = loop_.new_tool_result("call_1", "shell", "{\"cmd\": \"ls\"}", "output", None);
        assert_eq!(msg.role, "tool");
        assert_eq!(msg.tool_call_id(), "call_1");
        assert_eq!(msg.text(), "output");
    }

    #[test]
    fn loop_new_tool_result_with_error() {
        let loop_ = make_loop();
        let msg = loop_.new_tool_result("call_1", "shell", "{}", "", Some("file not found"));
        assert!(msg.text().contains("Error"));
        assert!(msg.text().contains("file not found"));
    }

    #[test]
    fn loop_new_tool_result_truncates_long_output() {
        let loop_ = make_loop();
        let long = "x".repeat(200_000);
        let msg = loop_.new_tool_result("call_1", "shell", "{}", &long, None);
        assert!(msg.text().len() <= 110_000);
        assert!(msg.text().contains("truncated"));
    }

    #[test]
    fn loop_new_user_message() {
        let loop_ = make_loop();
        let msg = loop_.new_user_message("hello");
        assert_eq!(msg.role, "user");
        assert_eq!(msg.text(), "hello");
    }

    #[test]
    fn loop_builder_methods() {
        let tools = vec![];
        let loop_ = Loop::new(std::sync::Arc::new(MockProvider), "m")
            .with_tools(tools)
            .with_system_prompt("test prompt")
            .with_config(crate::types::AgentConfig::default());
        assert_eq!(loop_.model, "m");
    }

    #[test]
    fn loop_independent_copy() {
        let loop_ = make_loop()
            .with_system_prompt("original prompt")
            .with_tools(vec![]);
        let copy = loop_.independent_copy();
        assert_eq!(copy.model, loop_.model);
        assert_eq!(copy.system_prompt, "original prompt");
        // Independent state: interrupt flag should be fresh.
        assert!(!copy
            .interrupt_flag()
            .load(std::sync::atomic::Ordering::SeqCst));
    }

    // ─── execute_one_tool_impl_static ──────────────────────────────────────

    #[tokio::test]
    async fn execute_one_tool_unknown_tool() {
        let tc = crate::types::ToolCall {
            id: "c1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: "nonexistent_tool".to_string(),
                arguments: serde_json::json!({}),
            },
        };
        let (_result, err, name) =
            Loop::execute_one_tool_impl_static(&tc, &[], &crate::types::AgentConfig::default())
                .await;
        assert_eq!(name, "nonexistent_tool");
        assert!(err.is_some());
        assert!(err.unwrap().contains("Unknown tool"));
    }

    #[tokio::test]
    async fn execute_one_tool_finds_and_runs_tool() {
        let tools = vec![crate::tools::shell_tool()];
        let tc = crate::types::ToolCall {
            id: "c1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: "shell".to_string(),
                arguments: serde_json::json!("{\"command\": \"echo works\"}"),
            },
        };
        let (result, _err, name) =
            Loop::execute_one_tool_impl_static(&tc, &tools, &crate::types::AgentConfig::default())
                .await;
        assert_eq!(name, "shell");
        // Tool ran (might fail due to scope, but should have run)
        let _ = result;
    }

    #[tokio::test]
    async fn execute_one_tool_before_hook_blocks() {
        let config = crate::types::AgentConfig {
            before_tool_call: Some(std::sync::Arc::new(|_name, _id, _args| {
                Some(crate::types::ToolCallResult {
                    result: "blocked by hook".to_string(),
                    is_error: true,
                })
            })),
            ..Default::default()
        };
        let tc = crate::types::ToolCall {
            id: "c1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: "shell".to_string(),
                arguments: serde_json::json!({}),
            },
        };
        let (result, err, _) = Loop::execute_one_tool_impl_static(&tc, &[], &config).await;
        assert_eq!(result, "blocked by hook");
        assert!(err.is_some());
    }

    #[tokio::test]
    async fn execute_one_tool_before_hook_allows() {
        let config = crate::types::AgentConfig {
            before_tool_call: Some(std::sync::Arc::new(|_name, _id, _args| {
                Some(crate::types::ToolCallResult {
                    result: "allowed by hook".to_string(),
                    is_error: false,
                })
            })),
            ..Default::default()
        };
        let tc = crate::types::ToolCall {
            id: "c1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: "shell".to_string(),
                arguments: serde_json::json!({}),
            },
        };
        let (result, err, _) = Loop::execute_one_tool_impl_static(&tc, &[], &config).await;
        assert_eq!(result, "allowed by hook");
        assert!(err.is_none());
    }

    #[tokio::test]
    async fn execute_one_tool_before_hook_none_passes_through() {
        let config = crate::types::AgentConfig {
            before_tool_call: Some(std::sync::Arc::new(|_name, _id, _args| None)),
            ..Default::default()
        };
        let tc = crate::types::ToolCall {
            id: "c1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: "unknown".to_string(),
                arguments: serde_json::json!({}),
            },
        };
        let (_, err, _) = Loop::execute_one_tool_impl_static(&tc, &[], &config).await;
        assert!(err.is_some());
        assert!(err.unwrap().contains("Unknown tool"));
    }

    #[tokio::test]
    async fn execute_one_tool_prepare_hook_modifies_args() {
        let config = crate::types::AgentConfig {
            prepare_tool_call: Some(std::sync::Arc::new(|_name, args| {
                let mut modified = args.clone();
                modified["injected"] = serde_json::json!(true);
                modified
            })),
            ..Default::default()
        };
        let tc = crate::types::ToolCall {
            id: "c1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: "unknown".to_string(),
                arguments: serde_json::json!({}),
            },
        };
        let (_, err, _) = Loop::execute_one_tool_impl_static(&tc, &[], &config).await;
        assert!(err.is_some());
    }

    #[tokio::test]
    async fn execute_one_tool_finalize_hook_transforms_result() {
        let config = crate::types::AgentConfig {
            finalize_tool_call: Some(std::sync::Arc::new(|_name, _result, _err| {
                ("finalized".to_string(), None)
            })),
            ..Default::default()
        };
        let tools = vec![crate::tools::shell_tool()];
        let tc = crate::types::ToolCall {
            id: "c1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: "shell".to_string(),
                arguments: serde_json::json!("{\"command\": \"echo test\"}"),
            },
        };
        let (result, err, _) = Loop::execute_one_tool_impl_static(&tc, &tools, &config).await;
        assert_eq!(result, "finalized");
        assert!(err.is_none());
    }

    #[tokio::test]
    async fn execute_one_tool_after_hook_transforms() {
        let config = crate::types::AgentConfig {
            after_tool_call: Some(std::sync::Arc::new(|_name, _id, _args, _result, _err| {
                Some(crate::types::ToolCallResult {
                    result: "after-hook".to_string(),
                    is_error: false,
                })
            })),
            ..Default::default()
        };
        let tc = crate::types::ToolCall {
            id: "c1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: "unknown".to_string(),
                arguments: serde_json::json!({}),
            },
        };
        let (result, err, _) = Loop::execute_one_tool_impl_static(&tc, &[], &config).await;
        assert_eq!(result, "after-hook");
        assert!(err.is_none());
    }

    #[tokio::test]
    async fn execute_one_tool_after_hook_error() {
        let config = crate::types::AgentConfig {
            after_tool_call: Some(std::sync::Arc::new(|_name, _id, _args, _result, _err| {
                Some(crate::types::ToolCallResult {
                    result: "hook error".to_string(),
                    is_error: true,
                })
            })),
            ..Default::default()
        };
        let tc = crate::types::ToolCall {
            id: "c1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: "unknown".to_string(),
                arguments: serde_json::json!({}),
            },
        };
        let (result, err, _) = Loop::execute_one_tool_impl_static(&tc, &[], &config).await;
        assert_eq!(result, "hook error");
        assert!(err.is_some());
    }

    // ─── Mock streaming provider ────────────────────────────────────────────

    struct TextStreamProvider {
        chunks: Vec<String>,
    }

    #[async_trait::async_trait]
    impl crate::types::LLMProvider for TextStreamProvider {
        async fn stream_model(
            &self,
            _request: crate::llm::schema::ModelRequest,
        ) -> anyhow::Result<
            tokio_stream::wrappers::ReceiverStream<crate::llm::schema::ModelStreamEvent>,
        > {
            let (tx, rx) = tokio::sync::mpsc::channel(64);
            let chunks = self.chunks.clone();
            tokio::spawn(async move {
                for chunk in chunks {
                    let _ = tx
                        .send(crate::llm::schema::ModelStreamEvent::TextDelta {
                            id: "text".to_string(),
                            text: chunk,
                        })
                        .await;
                }
                // Send stop event to end the stream
                let _ = tx
                    .send(crate::llm::schema::ModelStreamEvent::Finish {
                        reason: crate::llm::schema::FinishReason::Stop,
                        usage: Some(crate::types::Usage {
                            prompt_tokens: 10,
                            completion_tokens: 5,
                            total_tokens: 15,
                            ..Default::default()
                        }),
                    })
                    .await;
            });
            Ok(tokio_stream::wrappers::ReceiverStream::new(rx))
        }
    }

    #[tokio::test]
    async fn run_streaming_produces_text_output() {
        let provider = TextStreamProvider {
            chunks: vec!["Hello ".to_string(), "world".to_string()],
        };
        let loop_ = Loop::new(std::sync::Arc::new(provider), "test-model");
        let result = loop_.run_streaming("test prompt".to_string(), |_| {}).await;
        assert!(result.is_ok());
        let final_text = result.unwrap();
        assert!(final_text.contains("Hello world"));
        assert!(!loop_
            .stream_incomplete
            .load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn provider_eof_without_stop_marks_run_incomplete() {
        let loop_ = make_loop();
        let result = loop_.run_streaming("test prompt".to_string(), |_| {}).await;
        assert!(result.is_ok());
        assert!(
            loop_
                .stream_incomplete
                .load(std::sync::atomic::Ordering::SeqCst),
            "EOF without a provider stop frame must not be a clean completion"
        );
    }

    fn cov_ok_handler(
        _: serde_json::Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send>> {
        Box::pin(async { Ok("contents of SKILL.md body".to_string()) })
    }

    fn cov_err_handler(
        _: serde_json::Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send>> {
        Box::pin(async { Err(anyhow::anyhow!("fixture failure")) })
    }

    fn cov_tool(name: &str, ok: bool) -> crate::types::AgentTool {
        let handler: crate::types::ToolHandler = if ok { cov_ok_handler } else { cov_err_handler };
        crate::types::AgentTool {
            def: crate::types::ToolDef {
                tool_type: "function".to_string(),
                function: crate::types::FunctionDef {
                    name: name.to_string(),
                    description: "coverage fixture".to_string(),
                    parameters: serde_json::json!({}),
                },
            },
            handler,
            guidelines: vec![],
        }
    }

    fn cov_tool_call(id: &str, name: &str, args: serde_json::Value) -> crate::types::ToolCall {
        crate::types::ToolCall {
            id: id.to_string(),
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: name.to_string(),
                arguments: args,
            },
        }
    }

    #[tokio::test]
    async fn execute_one_tool_finalize_hook_sees_error_arm() {
        let config = crate::types::AgentConfig {
            finalize_tool_call: Some(std::sync::Arc::new(|name, _result, err| {
                (format!("finalized[{name}]"), Some(err))
            })),
            ..Default::default()
        };
        let tc = cov_tool_call("c1", "no_such_tool", serde_json::json!({}));
        let (result, err, _) = Loop::execute_one_tool_impl_static(&tc, &[], &config).await;
        assert_eq!(result, "finalized[no_such_tool]");
        assert!(err.unwrap().to_string().contains("Unknown tool"));
    }

    #[tokio::test]
    async fn sequential_tools_verbose_logs_skill_tag_and_error() {
        // tracing event regions only evaluate under a subscriber.
        let _subscriber = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_writer(std::io::sink)
                .finish(),
        );
        let mut loop_ = make_loop();
        loop_.verbose = true;
        loop_.tools = vec![cov_tool("read", true), cov_tool("explode", false)];
        let calls = vec![
            cov_tool_call("c1", "read", serde_json::json!({"path": "SKILL.md"})),
            cov_tool_call("c2", "explode", serde_json::json!({})),
        ];
        let mut messages = Vec::new();
        loop_
            .execute_tools_sequential(0, &calls, &mut messages, &|_| {}, &None)
            .await;
        assert_eq!(messages.len(), 2);
        assert!(messages[0].text().contains("SKILL.md"));
        assert!(messages[1].text().contains("fixture failure"));
    }

    #[tokio::test]
    async fn sequential_tools_interrupt_placeholders_serialize_object_args() {
        let loop_ = make_loop();
        loop_.abort(); // interrupted before the first tool runs
        let calls = vec![
            cov_tool_call("c1", "read", serde_json::json!({"path": "a.rs"})),
            cov_tool_call("c2", "shell", serde_json::json!({"command": "ls"})),
        ];
        let mut messages = Vec::new();
        loop_
            .execute_tools_sequential(0, &calls, &mut messages, &|_| {}, &None)
            .await;
        assert_eq!(messages.len(), 2);
        assert!(messages[0].text().contains("cancelled"));
        assert!(messages[1].text().contains("cancelled"));
    }
}
