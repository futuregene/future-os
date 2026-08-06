//! `future skills` — port of cli/src/commands/skills.ts (stub in P0; P2 ports
//! list/install/uninstall/install-builtin/update bodies).

use crate::output::Output;

/// `isSkillsCommand(command)` — type-guard port; `undefined` is not a command.
pub fn is_skills_command(command: Option<&str>) -> bool {
    matches!(
        command,
        Some("list" | "install" | "uninstall" | "install-builtin" | "update")
    )
}

/// `skills(command, args)` (P2).
pub async fn skills(command: &str, _args: &[String], _out: &Output) -> Result<(), String> {
    Err(not_implemented(&format!("skills {command}")))
}

fn not_implemented(what: &str) -> String {
    format!("`future {what}` is not implemented yet in the Rust CLI (P2)")
}
