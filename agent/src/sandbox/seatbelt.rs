//! macOS Seatbelt (sandbox-exec) profile generation and command wrapping.
//!
//! Profile shape (SANDBOX_PLAN.md §3.1): deny-by-default, broad reads minus
//! credential paths, writes only into the resolved writable roots, network
//! off unless the policy enables it. All paths are embedded canonicalized and
//! SBPL-quoted — never interpolated raw (injection safety).
//!
//! `mach-lookup` / `sysctl` stay broad in Phase 1 and get narrowed against
//! the profile smoke tests (see sandbox smoke tests).

#![cfg(target_os = "macos")]

use super::{ResolvedSandbox, SandboxMode};

/// Quote a path for embedding in an SBPL string literal.
fn sb_quote(path: &std::path::Path) -> String {
    let raw = path.to_string_lossy();
    let escaped: String = raw
        .chars()
        .flat_map(|c| match c {
            '"' | '\\' => vec!['\\', c],
            _ => vec![c],
        })
        .collect();
    format!("\"{escaped}\"")
}

/// Credential paths that stay unreadable even though reads are otherwise
/// broad (SANDBOX_PLAN.md §2.3 — decided scope for Phase 1).
///
/// Note: whole-directory denials must NOT cover `~/.future` — chat temp
/// workspaces live under `~/.future/agent/workspace`, so only the credential
/// files themselves are denied there.
fn sensitive_read_denials() -> Vec<(String, std::path::PathBuf)> {
    let Some(home) = dirs::home_dir() else {
        return vec![];
    };
    vec![
        ("subpath".to_string(), home.join(".ssh")),
        ("subpath".to_string(), home.join(".gnupg")),
        ("literal".to_string(), home.join(".future/agent/auth.json")),
        (
            "literal".to_string(),
            home.join(".future/agent-app/auth.json"),
        ),
    ]
}

/// Build the SBPL profile for this sandbox configuration.
pub fn build_profile(sandbox: &ResolvedSandbox) -> String {
    let mut profile = String::from(
        "(version 1)\n\
         (deny default)\n\
         ; process management — build toolchains fork/exec constantly\n\
         (allow process-fork)\n\
         (allow process-exec)\n\
         (allow process-info*)\n\
         (allow signal (target same-sandbox))\n\
         (allow pseudo-tty)\n\
         ; broad reads (credential paths denied below)\n\
         (allow file-read*)\n\
         ; system infrastructure — Phase 1 broad, narrowed via smoke tests\n\
         (allow sysctl-read)\n\
         (allow mach-lookup)\n\
         (allow ipc-posix*)\n\
         (allow file-ioctl)\n",
    );

    // Credential read denials come AFTER the broad allow (SBPL: the last
    // matching rule wins).
    let denials = sensitive_read_denials();
    if !denials.is_empty() {
        profile.push_str("(deny file-read*");
        for (kind, path) in &denials {
            profile.push_str(&format!(" ({kind} {})", sb_quote(path)));
        }
        profile.push_str(")\n");
    }

    // Writes: pseudo-devices always; writable roots only outside read-only.
    profile.push_str(
        "(allow file-write-data\n \
          (literal \"/dev/null\") (literal \"/dev/zero\")\n \
          (literal \"/dev/stdout\") (literal \"/dev/stderr\")\n \
          (regex #\"^/dev/tty\") (literal \"/dev/dtracehelper\"))\n",
    );
    if sandbox.mode != SandboxMode::ReadOnly {
        profile.push_str("(allow file-write*");
        for root in &sandbox.writable_roots {
            profile.push_str(&format!("\n  (subpath {})", sb_quote(root)));
        }
        profile.push_str(")\n");
    }

    // Network: deny-by-default already blocks it; open it all when enabled.
    if sandbox.network_access {
        profile.push_str("(allow network*)\n(allow system-socket)\n");
    }

    profile
}

/// Build the wrapped bash invocation: `sandbox-exec -p <profile> bash -c <cmd>`.
pub fn build_command(sandbox: &ResolvedSandbox, command: &str) -> tokio::process::Command {
    let profile = build_profile(sandbox);
    let mut child = tokio::process::Command::new("/usr/bin/sandbox-exec");
    child.args(["-p", &profile, "bash", "-c", command]);
    child
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::SandboxPolicy;

    fn resolved(policy: &SandboxPolicy) -> ResolvedSandbox {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let ws = std::env::temp_dir().join(format!("futureos-seatbelt-{stamp}"));
        std::fs::create_dir_all(&ws).unwrap();
        ResolvedSandbox::resolve(policy, ws.to_string_lossy().as_ref())
    }

    #[test]
    fn workspace_write_profile_allows_roots_denies_network() {
        let sandbox = resolved(&SandboxPolicy::default());
        let profile = build_profile(&sandbox);
        assert!(profile.contains("(deny default)"));
        assert!(profile.contains("(allow file-write*"));
        assert!(profile.contains(sandbox.workspace.to_string_lossy().as_ref()));
        assert!(!profile.contains("(allow network*)"));
        assert!(profile.contains(".ssh"));
    }

    #[test]
    fn read_only_profile_has_no_root_writes() {
        let sandbox = resolved(&SandboxPolicy {
            mode: SandboxMode::ReadOnly,
            ..Default::default()
        });
        let profile = build_profile(&sandbox);
        assert!(!profile.contains("(allow file-write*"));
        // Pseudo-device writes stay allowed so shells keep working.
        assert!(profile.contains("/dev/null"));
    }

    #[test]
    fn network_flag_opens_network() {
        let sandbox = resolved(&SandboxPolicy {
            network_access: true,
            ..Default::default()
        });
        assert!(build_profile(&sandbox).contains("(allow network*)"));
    }

    #[test]
    fn paths_with_quotes_are_escaped() {
        let tricky = std::path::Path::new("/tmp/we\"ird\\path");
        let quoted = sb_quote(tricky);
        assert_eq!(quoted, "\"/tmp/we\\\"ird\\\\path\"");
    }
}
