//! `future tools` — port of cli/src/commands/tools.ts (stub in P0; P2 ports
//! list/describe/call bodies including the browser directory).

use crate::output::Output;

/// `isToolsCommand(command)` — type-guard port; `undefined` is not a command.
pub fn is_tools_command(command: Option<&str>) -> bool {
    matches!(command, Some("list" | "call" | "describe"))
}

/// `tools(command, args)` (P2).
pub async fn tools(command: &str, _args: &[String], _out: &Output) -> Result<(), String> {
    Err(not_implemented(&format!("tools {command}")))
}

fn not_implemented(what: &str) -> String {
    format!("`future {what}` is not implemented yet in the Rust CLI (P2)")
}
