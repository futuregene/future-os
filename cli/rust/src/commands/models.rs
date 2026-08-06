//! `future models` — port of cli/src/commands/models.ts (stub in P0; P1 ports
//! the gRPC get_available_models body).

use crate::output::Output;

/// `models(args)` (P1).
pub async fn models(_args: &[String], _out: &Output) -> Result<(), String> {
    Err(not_implemented("models"))
}

fn not_implemented(what: &str) -> String {
    format!("`future {what}` is not implemented yet in the Rust CLI (P1)")
}
