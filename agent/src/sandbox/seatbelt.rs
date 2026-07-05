//! macOS Seatbelt (sandbox-exec) profile compiled from the approval rules.
//!
//! The rule engine is first-match-wins with the highest-priority layer first
//! (SANDBOX_PLAN.md). SBPL is last-match-wins. So we emit a permissive base
//! (reads open, writes to workspace+temp) and then every rule from
//! **lowest** priority to **highest** (layers reversed, and reversed within
//! each layer) — the last SBPL match then equals the engine's first match.
//!
//! `ask` and `deny` both compile to an OS-level denial (bash can't prompt
//! mid-syscall); a resulting failure surfaces via the escalation flow. Network
//! is unrestricted in v2.

#![cfg(target_os = "macos")]

use super::rules::{Decision, MatcherSbpl, PathRule};
use super::ResolvedSandbox;

/// Quote a path for embedding in an SBPL string literal.
fn sb_quote(path: &std::path::Path) -> String {
    sb_quote_str(&path.to_string_lossy())
}

fn sb_quote_str(raw: &str) -> String {
    let escaped: String = raw
        .chars()
        .flat_map(|c| match c {
            '"' | '\\' => vec!['\\', c],
            _ => vec![c],
        })
        .collect();
    format!("\"{escaped}\"")
}

/// An SBPL filter fragment for a rule's matcher.
fn matcher_filter(rule: &PathRule) -> String {
    match rule.matcher_sbpl() {
        MatcherSbpl::Subtree(base) => format!("(subpath {})", sb_quote(base)),
        // SBPL regex literal. Our glob→regex output contains no `"`; escape
        // backslashes defensively via the same quoter minus the outer quotes.
        MatcherSbpl::Regex(re) => format!("(regex #{})", sb_quote_str(re)),
    }
}

/// Emit the read/write allow/deny clauses for one rule, if its access covers
/// that operation. `ask` compiles as `deny` for bash.
fn emit_rule(profile: &mut String, rule: &PathRule) {
    let filter = matcher_filter(rule);
    let access = rule.access();
    if access.covers_read() {
        let verb = match rule.decision() {
            Decision::Allow => "allow",
            Decision::Ask | Decision::Deny => "deny",
        };
        profile.push_str(&format!("({verb} file-read* {filter})\n"));
    }
    if access.covers_write() {
        let verb = match rule.decision() {
            Decision::Allow => "allow",
            Decision::Ask | Decision::Deny => "deny",
        };
        profile.push_str(&format!("({verb} file-write* {filter})\n"));
    }
}

/// Build the SBPL profile for this sandbox.
pub fn build_profile(sandbox: &ResolvedSandbox) -> String {
    let rules = sandbox.rule_set();
    let mut profile = String::from(
        "(version 1)\n\
         (deny default)\n\
         ; process management — build toolchains fork/exec constantly\n\
         (allow process-fork) (allow process-exec) (allow process-info*)\n\
         (allow signal (target same-sandbox)) (allow pseudo-tty)\n\
         ; system infrastructure (broad in v2; narrow via smoke tests)\n\
         (allow sysctl-read) (allow mach-lookup) (allow ipc-posix*) (allow file-ioctl)\n\
         ; network: unrestricted in v2\n\
         (allow network*) (allow system-socket)\n\
         ; ── base: reads open, writes to workspace + temp + pseudo-devices ──\n\
         (allow file-read*)\n",
    );

    // Pseudo-device writes always allowed (shells need them). /dev/stdout|stderr
    // resolve to /dev/fd/N on macOS — the open() hits the fd path.
    profile.push_str(
        "(allow file-write*\n \
          (literal \"/dev/null\") (literal \"/dev/zero\")\n \
          (literal \"/dev/stdout\") (literal \"/dev/stderr\")\n \
          (regex #\"^/dev/fd/\") (regex #\"^/dev/tty\") (literal \"/dev/dtracehelper\")\n",
    );
    // Base writable roots: workspace + temp (mirrors the engine's write fallback).
    profile.push_str(&format!("  (subpath {})\n", sb_quote(&rules.workspace)));
    for tmp in super::rules::temp_roots() {
        profile.push_str(&format!("  (subpath {})\n", sb_quote(&tmp)));
        // macOS: /tmp and /var symlink into /private; also allow the /private form.
    }
    profile.push_str(")\n");

    // Emit rules from LOWEST priority to HIGHEST so the last SBPL match equals
    // the engine's first match. Engine order is highest-first, so reverse the
    // layers and reverse within each layer.
    profile.push_str("; ── rule layers (low→high priority; last match wins) ──\n");
    for layer in rules.profile_layers().iter().rev() {
        for rule in layer.iter().rev() {
            emit_rule(&mut profile, rule);
        }
    }

    profile
}

/// `sandbox-exec -p <profile> bash -c <cmd>`.
pub fn build_command(sandbox: &ResolvedSandbox, command: &str) -> tokio::process::Command {
    let profile = build_profile(sandbox);
    let mut child = tokio::process::Command::new("/usr/bin/sandbox-exec");
    child.args(["-p", &profile, "bash", "-c", command]);
    child
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::{ResolvedSandbox, SandboxPolicy};

    fn enabled_sandbox() -> ResolvedSandbox {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let ws = std::env::temp_dir().join(format!("futureos-seatbelt-{stamp}"));
        std::fs::create_dir_all(&ws).unwrap();
        ResolvedSandbox::resolve(
            &SandboxPolicy {
                tier: crate::sandbox::SandboxTier::Sandbox,
            },
            ws.to_string_lossy().as_ref(),
        )
    }

    #[test]
    fn base_profile_shape() {
        let s = enabled_sandbox();
        let p = build_profile(&s);
        assert!(p.contains("(deny default)"));
        assert!(p.contains("(allow file-read*)"));
        assert!(p.contains("(allow network*)")); // network open in v2
        assert!(p.contains(s.workspace.to_string_lossy().as_ref()));
        assert!(p.contains("/dev/null"));
    }

    #[test]
    fn credential_reads_denied_in_profile() {
        let p = build_profile(&enabled_sandbox());
        // Built-in .ssh ask → compiled as deny file-read*.
        let ssh = dirs::home_dir().unwrap().join(".ssh");
        assert!(p.contains(&format!("(deny file-read* (subpath \"{}\"", ssh.display())));
    }

    #[test]
    fn rule_file_write_denied_in_profile() {
        let s = enabled_sandbox();
        let p = build_profile(&s);
        let rulefile = s.workspace.join(".future/approval_rule.json");
        // Layer-0 override → deny file-write* for the rule file, emitted last
        // (highest priority) so it wins over the base workspace write allow.
        assert!(p.contains(&format!(
            "(deny file-write* (subpath \"{}\"",
            rulefile.display()
        )));
        let deny_idx = p
            .find(&format!(
                "(deny file-write* (subpath \"{}\"",
                rulefile.display()
            ))
            .unwrap();
        let base_allow_idx = p.find("(allow file-write*").unwrap();
        assert!(
            deny_idx > base_allow_idx,
            "override deny must come after base allow"
        );
    }

    #[test]
    fn workspace_env_write_denied_in_profile() {
        let p = build_profile(&enabled_sandbox());
        // Built-in workspace `.env` ask → deny (glob or subpath).
        assert!(p.contains("file-write*") && p.contains(".env"));
    }
}
