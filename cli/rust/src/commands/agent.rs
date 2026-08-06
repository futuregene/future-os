//! `future agent` — port of cli/src/commands/agent.ts (stub in P0; P1 ports
//! the gRPC get_state body).

use crate::output::Output;

/// `agentStatus(json)` — show running agent state (P1).
pub async fn agent_status(_json: bool, _out: &Output) -> Result<(), String> {
    Err(not_implemented("agent status"))
}

fn not_implemented(what: &str) -> String {
    format!("`future {what}` is not implemented yet in the Rust CLI (P1)")
}
