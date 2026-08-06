//! `future session` — port of cli/src/commands/session.ts (stub in P0; P1
//! ports list/inspect/rename/delete bodies).

use crate::output::Output;

/// `session(command, rest)` (P1).
pub async fn session(
    _command: Option<&str>,
    _rest: &[String],
    _out: &Output,
) -> Result<(), String> {
    Err(not_implemented("session"))
}

fn not_implemented(what: &str) -> String {
    format!("`future {what}` is not implemented yet in the Rust CLI (P1)")
}
