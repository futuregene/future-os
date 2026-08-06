//! `future doctor` — port of cli/src/commands/doctor.ts (stub in P0; P1 ports
//! the environment diagnostic body).

use crate::output::Output;

/// `doctor()` (P1).
pub async fn doctor(_out: &Output) -> Result<(), String> {
    Err(not_implemented("doctor"))
}

fn not_implemented(what: &str) -> String {
    format!("`future {what}` is not implemented yet in the Rust CLI (P1)")
}
