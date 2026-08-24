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
    CompactionCommitted {
        checkpoint: crate::compaction::ContextCheckpoint,
    },
    CompactionFailed {
        trigger: crate::compaction::CompactionTrigger,
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
    },
}
