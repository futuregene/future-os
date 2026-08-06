//! `future init` — port of cli/src/commands/init.ts (stub in P0; P1 ports the
//! install-builtin-skills + binary linking body).

use crate::output::Output;

/// `initCommand()` (P1).
pub async fn init_command(_out: &Output) -> Result<(), String> {
    Err(not_implemented("init"))
}

fn not_implemented(what: &str) -> String {
    format!("`future {what}` is not implemented yet in the Rust CLI (P1)")
}
