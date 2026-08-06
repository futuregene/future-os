//! `future run` — port of cli/src/commands/run.ts (stub in P0; P2 ports the
//! one-shot prompt body over gRPC).

use crate::output::Output;

/// `run(args)` — args are everything after `future run` (P2).
pub async fn run_command(_args: &[String], _out: &Output) -> Result<(), String> {
    Err(not_implemented("run"))
}

fn not_implemented(what: &str) -> String {
    format!("`future {what}` is not implemented yet in the Rust CLI (P2)")
}
