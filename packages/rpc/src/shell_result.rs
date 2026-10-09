//! Host-owned process facts. Output text is a projection, never an input to these facts.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellStatus {
    Exited,
    LaunchFailed,
    ExecutionFailed,
    TimedOut,
    Cancelled,
    NotStarted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellAttempt {
    pub status: ShellStatus,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub output: String,
    pub output_truncated: bool,
    pub escalated: bool,
}

/// One command invocation. Attempts retain the original run and any approved retry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellResult {
    pub command: String,
    pub cwd: String,
    pub duration_ms: u64,
    pub status: ShellStatus,
    pub exit_code: Option<i32>,
    pub is_soft_fail: bool,
    pub is_error: bool,
    pub attempts: Vec<ShellAttempt>,
    pub approval: Option<String>,
    pub note: Option<String>,
}

impl ShellResult {
    pub fn update_verdict(&mut self) {
        self.is_error = self.approval.as_deref().is_some_and(|approval| {
            matches!(approval, "denied" | "cancelled" | "context_invalidated")
        }) || match self.status {
            ShellStatus::Exited => self
                .exit_code
                .is_none_or(|code| code != 0 && !self.is_soft_fail),
            _ => true,
        };
    }
}
