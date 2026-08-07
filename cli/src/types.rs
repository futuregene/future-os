//! Types — port of `cli/src/types.ts`.

/// Result of running a subprocess (`runProcess` / `runInheritedProcess`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceResult {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Union types from types.ts (unused by the dispatch, kept for parity).
pub type AgentCommand = &'static str;
pub type ChannelCommand = &'static str;
