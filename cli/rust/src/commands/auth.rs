//! `future auth` — port of cli/src/commands/auth.ts (stub in P0; P1 ports
//! login/status/credential/logout bodies).

use crate::output::Output;

/// `login(urlOverride)` — device-code OAuth flow (P1).
pub async fn login(_url_override: Option<String>, _out: &Output) -> Result<(), String> {
    Err(not_implemented("auth login"))
}

/// `status()` — show login state (P1).
pub async fn status(_out: &Output) -> Result<(), String> {
    Err(not_implemented("auth status"))
}

/// `credential({ json })` — output API key for scripting (P1).
pub async fn credential(_json: bool, _out: &Output) -> Result<(), String> {
    Err(not_implemented("auth credential"))
}

/// `logout()` — remove the stored API key (P1).
pub async fn logout(_out: &Output) -> Result<(), String> {
    Err(not_implemented("auth logout"))
}

fn not_implemented(what: &str) -> String {
    format!("`future {what}` is not implemented yet in the Rust CLI (P1)")
}
