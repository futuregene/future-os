use crate::llm::schema::ModelStreamEvent;
use serde_json::Value;

/// Typed events emitted by one agent run before they are projected onto the
/// public RPC event vocabulary.
#[derive(Debug, Clone)]
pub enum RunEvent {
    AgentStart {
        started_at_ms: u64,
    },
    Model(ModelStreamEvent),
    MessageStart {
        entry_id: String,
    },
    MessageEnd {
        entry_id: String,
        complete: bool,
    },
    StreamRetry {
        attempt: usize,
        max_retries: usize,
        delay_ms: u64,
    },
    StreamResumed,
    /// The model's output budget ran out and this run is writing the rest of
    /// the answer instead of ending at the cut.
    ///
    /// Distinct from [`RunEvent::StreamRetry`]: nothing is being reconnected,
    /// and the reply keeps growing. `thinking_level` is the level the
    /// continuation will run at when the cut produced no visible output at all
    /// (the entire budget went to reasoning) and the loop therefore stepped
    /// down; `None` means the level was left alone.
    OutputStreamContinuing {
        attempt: usize,
        max_continuations: usize,
        thinking_level: Option<String>,
    },
    CompactionStarted {
        operation_id: String,
        trigger: crate::compaction::CompactionTrigger,
        phase: crate::compaction::CompactionPhase,
    },
    CompactionCommitted {
        operation_id: String,
        checkpoint: crate::compaction::ContextCheckpoint,
    },
    CompactionFailed {
        operation_id: String,
        trigger: crate::compaction::CompactionTrigger,
        phase: crate::compaction::CompactionPhase,
        error: String,
    },
    ToolExecutionStarted {
        id: String,
        name: String,
        arguments: Value,
    },
    ToolExecutionFinished {
        id: String,
        name: String,
        output: String,
        error: Option<String>,
        exit_code: Option<i32>,
        is_soft_fail: Option<bool>,
        target_path: Option<String>,
        shell_result: Option<future_rpc::shell_result::ShellResult>,
    },
}
