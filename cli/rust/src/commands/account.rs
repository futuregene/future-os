//! `future account` — port of cli/src/commands/account.ts (stub in P0; P1
//! ports profile/balance bodies).

use crate::output::Output;

/// `isAccountCommand(command)` — type-guard port; `undefined` is not a command.
pub fn is_account_command(command: Option<&str>) -> bool {
    matches!(command, Some("profile" | "balance"))
}

/// `account(command, rest)` (P1).
pub async fn account(command: &str, _rest: &[String], _out: &Output) -> Result<(), String> {
    Err(not_implemented(&format!("account {command}")))
}

fn not_implemented(what: &str) -> String {
    format!("`future {what}` is not implemented yet in the Rust CLI (P1)")
}
