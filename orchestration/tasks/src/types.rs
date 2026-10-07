//! Core types for `future-tasks`.
//!
//! All types are serde-friendly and host-agnostic. The `Task` is the durable
//! definition; `TaskRun` is one execution; `TaskDep`/`TaskDepState` model the
//! dependency graph and join progress.

use serde::{Deserialize, Serialize};

/// Stable identifier prefixes.
pub const TASK_ID_PREFIX: &str = "tsk_";
pub const RUN_ID_PREFIX: &str = "trn_";
pub const REVISION_ID_PREFIX: &str = "rev_";

/// Envelope schema version; bump when the `<task ...>` block changes shape.
pub const TASK_ENVELOPE_SCHEMA_VERSION: &str = "task-v1";

/// Trigger kind (0 or 1 own schedule; deps are orthogonal).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TriggerKind {
    Manual,
    Schedule,
}

/// Join policy for a task's dependency edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DepJoin {
    All,
    Any,
}

/// Condition a dependency edge fires on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DepOn {
    Success,
    Failure,
    Completed,
}

/// Session reuse policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionPolicy {
    New,
    Existing,
}

/// What kind of conversation a task opens.
///
/// `Workspace` anchors the conversation to the task's working directory (the
/// default, and what a task that touches files needs). `Chat` is the temporary
/// -workspace conversation a user reaches from "new chat": the agent session
/// still runs *in* the task's `cwd`, but the conversation is filed under Chat
/// rather than under that directory in the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConversationMode {
    Chat,
    Workspace,
}

/// Reflection cadence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Reflection {
    Off,
    Ask,
    Auto,
}

/// Run kind (who asked for it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunKind {
    Main,
    Manual,
    Chain,
    Reflection,
}

/// Run origin (where the request came from).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunOrigin {
    Schedule,
    Ui,
    Cli,
    Chain,
    Reflection,
}

/// Run status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Running,
    Completed,
    Failed,
    Skipped,
}

/// A task definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub prompt: String,
    pub prompt_version: i64,
    pub cwd: String,
    pub model_id: Option<String>,
    pub thinking_level: Option<String>,
    pub session_policy: SessionPolicy,
    /// What kind of conversation this task opens (see [`ConversationMode`]).
    #[serde(default = "default_conversation_mode")]
    pub conversation_mode: ConversationMode,
    /// Host-side conversation binding (lazy when `existing`).
    pub thread_id: Option<String>,
    pub trigger_kind: TriggerKind,
    pub trigger_json: serde_json::Value,
    pub dep_join: DepJoin,
    /// Next due (epoch ms); `None` = no longer scheduled.
    pub next_due_at: Option<i64>,
    pub last_run_at: Option<i64>,
    /// Pending explicit/chain trigger (tick consumes).
    pub pending_request_at: Option<i64>,
    pub pending_origin: Option<RunOrigin>,
    pub pending_actor: Option<String>,
    pub reflection: Reflection,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
}

/// One execution of a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRun {
    pub id: String,
    pub task_id: String,
    pub kind: RunKind,
    pub origin: RunOrigin,
    pub actor: Option<String>,
    /// Scheduled due time; only for `kind=main`.
    pub due_at: Option<i64>,
    pub status: RunStatus,
    pub thread_id: Option<String>,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub prompt_version: Option<i64>,
    /// Truncated final answer (head…tail, 2000 chars).
    pub result_summary: Option<String>,
    pub feedback: Option<String>,
    pub feedback_note: Option<String>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub error_message: Option<String>,
}

/// A dependency edge (downstream task → upstream task).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDep {
    pub task_id: String,
    pub upstream_task_id: String,
    pub on: DepOn,
}

/// Join progress for one edge (used when `dep_join=all`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDepState {
    pub task_id: String,
    pub upstream_task_id: String,
    pub satisfied_run_id: Option<String>,
    pub satisfied_at: Option<i64>,
}

/// A prompt revision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptRevision {
    pub id: String,
    pub task_id: String,
    pub version: i64,
    pub prompt: String,
    pub source: String,
    pub status: String,
    pub reason: Option<String>,
    pub confidence: Option<f64>,
    pub source_run_id: Option<String>,
    pub created_at: i64,
}

/// Generate a task id.
pub fn new_task_id() -> String {
    format!("{}{}", TASK_ID_PREFIX, uuid::Uuid::new_v4().simple())
}

/// Generate a run id.
pub fn new_run_id() -> String {
    format!("{}{}", RUN_ID_PREFIX, uuid::Uuid::new_v4().simple())
}

/// Generate a revision id.
pub fn new_revision_id() -> String {
    format!("{}{}", REVISION_ID_PREFIX, uuid::Uuid::new_v4().simple())
}

/// The conversation mode a task has when it does not declare one: the
/// behaviour every task had before the setting existed.
fn default_conversation_mode() -> ConversationMode {
    ConversationMode::Workspace
}
