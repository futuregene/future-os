//! Approval rule evaluation (SANDBOX_PLAN.md §2.5).
//!
//! Rules are the first layer of the decision flow (§2.1): they can auto-reject
//! (deny) or auto-approve (skip the prompt) a tool call *before* the sandbox
//! auto-allow logic runs. An `approve` rule only skips the approval prompt — it
//! never bypasses the sandbox; the command still runs wrapped.
//!
//! Matching: `command_prefix` rules match a bash command, `path_glob` rules
//! match a write/edit path. Wildcards `*` / `?`. **Deny wins**: if any matching
//! rule rejects, the call is rejected regardless of approve rules (a safer
//! bias than opencode's pure last-match — see §2.5).

use super::approval::{argument_command, argument_path};
use crate::sandbox::SandboxRule;

#[derive(Debug, Clone)]
pub enum PolicyDecision {
    AskUser,
    AutoApprove,
    AutoReject(String),
}

/// Evaluate the session's approval rules against a tool call.
pub fn evaluate_policy(
    rules: &[SandboxRule],
    tool_name: &str,
    arguments: &serde_json::Value,
) -> PolicyDecision {
    let (match_kind, resource) = match tool_name {
        "bash" => ("command_prefix", argument_command(arguments)),
        "write" | "edit" => ("path_glob", argument_path(arguments)),
        _ => return PolicyDecision::AskUser,
    };
    let Some(resource) = resource else {
        return PolicyDecision::AskUser;
    };

    let mut approve = false;
    for rule in rules {
        if rule.match_kind != match_kind {
            continue;
        }
        if wildcard_match(&resource, &rule.match_value) {
            match rule.decision.as_str() {
                // Deny wins: the first matching reject settles it.
                "reject" => {
                    return PolicyDecision::AutoReject(format!(
                        "matches deny rule `{}`",
                        rule.match_value
                    ));
                }
                "approve" => approve = true,
                _ => {}
            }
        }
    }
    if approve {
        PolicyDecision::AutoApprove
    } else {
        PolicyDecision::AskUser
    }
}

/// Whether `input` matches a wildcard `pattern` (`*` = any run, `?` = any one
/// char), anchored full-match. A pattern ending in ` *` (e.g. `git push *`)
/// also matches the bare prefix with no trailing args (`git push`).
fn wildcard_match(input: &str, pattern: &str) -> bool {
    regex::Regex::new(&wildcard_to_regex(pattern))
        .map(|re| re.is_match(input))
        .unwrap_or(false)
}

fn wildcard_to_regex(pattern: &str) -> String {
    let mut re = String::from("^");
    for ch in pattern.chars() {
        match ch {
            '*' => re.push_str(".*"),
            '?' => re.push('.'),
            '.' | '+' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$' | '\\' => {
                re.push('\\');
                re.push(ch);
            }
            _ => re.push(ch),
        }
    }
    re.push('$');
    // Trailing " .*$" (from a "... *" pattern) → optional, so a bare command
    // with no arguments still matches its prefix rule.
    if let Some(stripped) = re.strip_suffix(" .*$") {
        return format!("{stripped}( .*)?$");
    }
    re
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(match_kind: &str, match_value: &str, decision: &str) -> SandboxRule {
        SandboxRule {
            match_kind: match_kind.to_string(),
            match_value: match_value.to_string(),
            decision: decision.to_string(),
        }
    }

    fn bash(command: &str) -> serde_json::Value {
        serde_json::json!({ "command": command })
    }

    fn write_to(path: &str) -> serde_json::Value {
        serde_json::json!({ "path": path, "content": "x" })
    }

    #[test]
    fn no_rules_asks_user() {
        assert!(matches!(
            evaluate_policy(&[], "bash", &bash("rm -rf x")),
            PolicyDecision::AskUser
        ));
    }

    #[test]
    fn approve_rule_auto_approves_matching_command() {
        let rules = vec![rule("command_prefix", "cargo *", "approve")];
        assert!(matches!(
            evaluate_policy(&rules, "bash", &bash("cargo build --release")),
            PolicyDecision::AutoApprove
        ));
        // Non-matching command still asks.
        assert!(matches!(
            evaluate_policy(&rules, "bash", &bash("npm install")),
            PolicyDecision::AskUser
        ));
    }

    #[test]
    fn trailing_star_also_matches_bare_prefix() {
        let rules = vec![rule("command_prefix", "git push *", "approve")];
        assert!(matches!(
            evaluate_policy(&rules, "bash", &bash("git push origin main")),
            PolicyDecision::AutoApprove
        ));
        assert!(matches!(
            evaluate_policy(&rules, "bash", &bash("git push")),
            PolicyDecision::AutoApprove
        ));
        // A different subcommand does not match.
        assert!(matches!(
            evaluate_policy(&rules, "bash", &bash("git status")),
            PolicyDecision::AskUser
        ));
    }

    #[test]
    fn deny_wins_over_approve() {
        // Approve broad, deny narrow — the narrow deny must win.
        let rules = vec![
            rule("command_prefix", "git *", "approve"),
            rule("command_prefix", "git push *", "reject"),
        ];
        assert!(matches!(
            evaluate_policy(&rules, "bash", &bash("git status")),
            PolicyDecision::AutoApprove
        ));
        assert!(matches!(
            evaluate_policy(&rules, "bash", &bash("git push origin")),
            PolicyDecision::AutoReject(_)
        ));
    }

    #[test]
    fn deny_wins_regardless_of_rule_order() {
        // Deny listed before the broad approve still wins.
        let rules = vec![
            rule("command_prefix", "rm *", "reject"),
            rule("command_prefix", "*", "approve"),
        ];
        assert!(matches!(
            evaluate_policy(&rules, "bash", &bash("rm -rf /")),
            PolicyDecision::AutoReject(_)
        ));
    }

    #[test]
    fn path_glob_rules_match_write_paths_only() {
        let rules = vec![rule("path_glob", "/tmp/build/*", "approve")];
        assert!(matches!(
            evaluate_policy(&rules, "write", &write_to("/tmp/build/out.txt")),
            PolicyDecision::AutoApprove
        ));
        assert!(matches!(
            evaluate_policy(&rules, "write", &write_to("/etc/hosts")),
            PolicyDecision::AskUser
        ));
        // A command_prefix rule never applies to a write path.
        let cmd_rules = vec![rule("command_prefix", "/tmp/*", "approve")];
        assert!(matches!(
            evaluate_policy(&cmd_rules, "write", &write_to("/tmp/x")),
            PolicyDecision::AskUser
        ));
    }

    #[test]
    fn wildcards_are_regex_safe() {
        // Dots in the pattern are literal, not "any char".
        let rules = vec![rule("command_prefix", "./deploy.sh *", "approve")];
        assert!(matches!(
            evaluate_policy(&rules, "bash", &bash("./deploy.sh prod")),
            PolicyDecision::AutoApprove
        ));
        assert!(matches!(
            evaluate_policy(&rules, "bash", &bash("xxdeployxsh prod")),
            PolicyDecision::AskUser
        ));
    }
}
