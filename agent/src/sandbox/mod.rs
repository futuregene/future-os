//! OS-level sandbox + path-based approval rules (APPROVAL_PLAN.md / SANDBOX_PLAN.md).
//!
//! Every approval is about a file-path access: [`rules::RuleSet`] resolves a
//! path + op to `Ask | Allow | Deny`. That verdict is enforced two ways:
//!   - read/write/edit tools: the approval layer prompts (Ask) / proceeds
//!     (Allow) / errors (Deny) before the in-process op runs.
//!   - bash: the rules compile into a Seatbelt profile (macOS); Ask and Deny
//!     both become an OS-level read/write denial, and a resulting failure
//!     surfaces via the escalation flow.
//!
//! Network is unrestricted. The whole system is gated by `enabled`: only GUI
//! sessions opt in; everything else runs fully open.

pub mod paths;
pub mod rules;
mod seatbelt;
#[cfg(windows)]
pub mod windows;
mod windows_plan;

#[cfg(windows)]
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rules::{Decision, Op, RuleSet};

/// The user-selected approval tier (composer / settings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SandboxTier {
    /// Off — no approval, no sandbox, everything runs.
    Off,
    /// Manual — approval rules on; bash asks (read-only allowlist bypass); no OS
    /// sandbox. The default, all platforms.
    #[default]
    Manual,
    /// Sandbox — approval rules on; bash runs inside the OS sandbox (macOS
    /// only; the GUI hides this option elsewhere).
    Sandbox,
}

impl SandboxTier {
    pub fn parse(value: &str) -> Self {
        match value {
            "off" => Self::Off,
            "sandbox" => Self::Sandbox,
            _ => Self::Manual,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Manual => "manual",
            Self::Sandbox => "sandbox",
        }
    }
}

/// Session sandbox policy from the frontend.
#[derive(Debug, Clone, Default)]
pub struct SandboxPolicy {
    pub tier: SandboxTier,
}

/// A resolved sandbox for one session/workspace: the layered rule set plus the
/// selected tier and whether the OS sandbox is usable here.
#[derive(Debug, Clone)]
pub struct ResolvedSandbox {
    pub tier: SandboxTier,
    /// Whether the platform sandbox (sandbox-exec) is usable here.
    pub available: bool,
    /// Canonicalized workspace directory.
    pub workspace: PathBuf,
    rules: RuleSet,
}

impl ResolvedSandbox {
    /// Resolve rules for `workspace`. The tier comes from the session policy.
    pub fn resolve(policy: &SandboxPolicy, workspace: &str) -> Self {
        let rules = RuleSet::resolve(Path::new(workspace));
        Self {
            tier: policy.tier,
            available: platform_sandbox_available(),
            workspace: rules.workspace.clone(),
            rules,
        }
    }

    /// Resolve sharing a session-rules handle so same-run "allow in this
    /// workspace/chat" injections reach this live sandbox.
    pub fn resolve_with_session(
        policy: &SandboxPolicy,
        workspace: &str,
        session: rules::SessionRules,
    ) -> Self {
        let rules = RuleSet::resolve_with_session(Path::new(workspace), session);
        Self {
            tier: policy.tier,
            available: platform_sandbox_available(),
            workspace: rules.workspace.clone(),
            rules,
        }
    }

    /// Whether approval rules apply at all (tools + evaluate). Off = fully open.
    pub fn enabled(&self) -> bool {
        self.tier != SandboxTier::Off
    }

    /// Whether bash runs pre-approval-gated (manual tier, or a sandbox tier on a
    /// platform without the OS sandbox). When true, bash asks (allowlist bypass);
    /// when false and enabled, bash is OS-sandboxed instead.
    pub fn bash_needs_approval(&self) -> bool {
        self.enabled() && !self.wraps_bash()
    }

    /// Whether `path` (canonicalized internally) is a built-in secret — used to
    /// suppress persistence of "allow in this workspace" for secret files.
    pub fn is_secret_path(&self, path: &Path) -> bool {
        self.rules
            .is_secret_path(&paths::canonicalize_lenient(path))
    }

    /// Fully-open sandbox (Off tier): no rules, no OS wrapping, no approval.
    /// Used for non-GUI clients and bare unit tests.
    pub fn disabled(workspace: &str) -> Self {
        let rules = RuleSet::resolve(Path::new(workspace));
        Self {
            tier: SandboxTier::Off,
            available: false,
            workspace: rules.workspace.clone(),
            rules,
        }
    }

    /// Evaluate a file access. `path` is canonicalized internally.
    pub fn evaluate(&self, path: &Path, op: Op) -> Decision {
        if !self.enabled() {
            return Decision::Allow;
        }
        self.rules.evaluate(&paths::canonicalize_lenient(path), op)
    }

    /// Convenience: is a write to `candidate` (relative/`~`/absolute) allowed
    /// without prompting? Non-Allow verdicts (Ask/Deny) return false.
    pub fn write_allowed(&self, candidate: &str) -> bool {
        let path = paths::resolve_against(&self.workspace, candidate);
        matches!(self.evaluate(&path, Op::Write), Decision::Allow)
    }

    /// Add a runtime "allow in this workspace" rule for the rest of this run.
    pub fn add_session_allow(&self, abs_pattern: &str, op: Op) {
        let access = match op {
            Op::Read => rules::Access::Read,
            Op::Write => rules::Access::Write,
        };
        self.rules
            .add_session_rule(abs_pattern, access, Decision::Allow);
    }

    /// Whether bash commands run wrapped in the OS sandbox (Sandbox tier on a
    /// platform where sandbox-exec is available).
    pub fn wraps_bash(&self) -> bool {
        self.tier == SandboxTier::Sandbox && self.available
    }

    /// Read access to the resolved rule set (Seatbelt profile builder).
    pub fn rule_set(&self) -> &RuleSet {
        &self.rules
    }

    /// Build the bash invocation: Seatbelt-wrapped when enabled+available and
    /// not escalated; otherwise the platform-appropriate shell (`bash -c` on
    /// Unix, `cmd /c` on Windows). `escalated` forces an unsandboxed run for
    /// one approved command.
    pub fn build_bash_command(&self, command: &str, escalated: bool) -> tokio::process::Command {
        if !escalated && self.wraps_bash() {
            #[cfg(target_os = "macos")]
            {
                return seatbelt::build_command(self, command);
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let mut child = tokio::process::Command::new("bash");
            child.args(["-c", command]);
            child
        }
        #[cfg(target_os = "windows")]
        {
            let command = rewrite_cli_tools_args(command);
            let mut child = tokio::process::Command::new("cmd");
            child.args(["/c", &command]);
            child
        }
    }

    /// On Windows, cmd.exe strips double quotes from the command string, which
    /// corrupts JSON passed via `future-cli tools call --args`. Rewrite the
    /// invocation to pipe JSON through a temp file and use `--stdin` instead,
    /// avoiding the shell quoting problem entirely.
    #[cfg(windows)]
    fn rewrite_cli_tools_args(command: &str) -> Cow<'_, str> {
        // Quick rejection: must contain --args, tools call, and a future binary name
        if !command.contains("--args")
            || !command.contains("tools call")
            || !(command.contains("future-cli") || command.contains("future "))
        {
            return Cow::Borrowed(command);
        }

        let args_pos = match command.find("--args") {
            Some(p) => p,
            None => return Cow::Borrowed(command),
        };

        let before = &command[..args_pos];
        let after = command[args_pos + "--args".len()..].trim_start();

        // Extract the JSON from the --args value
        let (json, rest) = match extract_json_arg(after) {
            Some(v) => v,
            None => return Cow::Borrowed(command),
        };

        // Write JSON to a temp file that survives the pipe
        let tmp =
            std::env::temp_dir().join(format!("future-tool-args-{}.json", std::process::id()));
        if std::fs::write(&tmp, &json).is_err() {
            return Cow::Borrowed(command);
        }

        // Pipe the file content into --stdin. The rewritten command never
        // puts JSON on the command line, so cmd.exe cannot mangle it.
        // Clean up the temp file after the pipeline completes.
        let new_cmd = format!(
            "(type \"{}\" | {} --stdin {}) & del \"{}\"",
            tmp.display(),
            before.trim_end(),
            rest.trim_start(),
            tmp.display(),
        );

        Cow::Owned(new_cmd)
    }

    /// Extract a JSON argument value from the start of `s`.
    /// Returns `(json_string, rest_of_command)` on success.
    /// Handles single-quoted, double-quoted, and bare `{...}` JSON.
    #[cfg(windows)]
    fn extract_json_arg(s: &str) -> Option<(String, &str)> {
        let first = s.chars().next()?;

        match first {
            '\'' => {
                let inner = &s[1..];
                let end = inner.find('\'')?;
                Some((inner[..end].to_string(), &inner[end + 1..]))
            }
            '"' => {
                let inner = &s[1..];
                let end = inner.find('"')?;
                Some((inner[..end].to_string(), &inner[end + 1..]))
            }
            '{' => {
                let mut depth = 0i32;
                for (i, ch) in s.char_indices() {
                    match ch {
                        '{' | '[' => depth += 1,
                        '}' | ']' => {
                            depth -= 1;
                            if depth == 0 {
                                return Some((s[..=i].to_string(), &s[i + 1..]));
                            }
                        }
                        _ => {}
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Structured `sandbox_boundary` payload for approval events.
    pub fn boundary_json(
        &self,
        violation: Option<&str>,
        inside_sandbox: bool,
    ) -> serde_json::Value {
        serde_json::json!({
            "inside_sandbox": inside_sandbox,
            "sandbox_available": self.available,
            "tier": self.tier.as_str(),
            "violation": violation,
            "cwd": self.workspace.to_string_lossy(),
        })
    }
}

impl Default for ResolvedSandbox {
    fn default() -> Self {
        ResolvedSandbox::disabled(
            &std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("/"))
                .to_string_lossy(),
        )
    }
}

/// Whether the OS-level sandbox is usable on this platform.
pub fn platform_sandbox_available() -> bool {
    #[cfg(target_os = "macos")]
    {
        Path::new("/usr/bin/sandbox-exec").exists()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Expose the generated Seatbelt profile (for smoke tests and diagnostics).
#[cfg(target_os = "macos")]
pub fn seatbelt_profile(sandbox: &ResolvedSandbox) -> String {
    seatbelt::build_profile(sandbox)
}

// ─── Escalation (post-hoc approval, carried into the tools layer) ──────────

/// A request to re-run a command outside the sandbox, raised from inside the
/// bash tool after a sandbox denial or when the model asks for it explicitly.
#[derive(Debug, Clone)]
pub struct EscalationRequest {
    pub command: String,
    pub justification: String,
    pub failure_summary: String,
}

#[derive(Debug, Clone)]
pub enum EscalationDecision {
    Approved,
    Denied(String),
}

/// Callback the RPC layer injects so `run_bash` can raise a `sandbox_escalation`
/// approval without touching RPC/UI internals. Blocks until the user decides.
pub type EscalationRequester = Arc<dyn Fn(&EscalationRequest) -> EscalationDecision + Send + Sync>;

// ─── Sandbox-denial heuristic ───────────────────────────────────────────────

/// Conservative check: does this failed sandboxed run look like the *sandbox*
/// stopped it? Network is unrestricted in v2, so only filesystem EPERM counts.
/// False negatives are fine (the model can retry with `escalated: true`);
/// false positives would nag the user, so match narrowly.
pub fn looks_like_sandbox_denial(_sandbox: &ResolvedSandbox, exit_code: i32, stderr: &str) -> bool {
    if exit_code == 0 {
        return false;
    }
    stderr.contains("Operation not permitted") || stderr.contains("sandbox-exec")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_workspace(name: &str) -> String {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("futureos-sandbox-{name}-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir.to_string_lossy().to_string()
    }

    fn enabled(workspace: &str) -> ResolvedSandbox {
        ResolvedSandbox::resolve(
            &SandboxPolicy {
                tier: SandboxTier::Manual,
            },
            workspace,
        )
    }

    #[test]
    fn tier_maps_bash_handling() {
        let ws = temp_workspace("tiers");
        let mut manual = enabled(&ws);
        manual.available = true;
        // Manual: bash needs approval, never OS-wrapped, even where available.
        assert!(!manual.wraps_bash());
        assert!(manual.bash_needs_approval());

        let mut sandbox = ResolvedSandbox::resolve(
            &SandboxPolicy {
                tier: SandboxTier::Sandbox,
            },
            &ws,
        );
        sandbox.available = true;
        assert!(sandbox.wraps_bash());
        assert!(!sandbox.bash_needs_approval());
        // Sandbox tier without the OS sandbox falls back to bash approval.
        sandbox.available = false;
        assert!(!sandbox.wraps_bash());
        assert!(sandbox.bash_needs_approval());

        let off = ResolvedSandbox::disabled(&ws);
        assert!(!off.enabled());
        assert!(!off.bash_needs_approval());
    }

    #[test]
    fn disabled_allows_everything() {
        let ws = temp_workspace("disabled");
        let s = ResolvedSandbox::disabled(&ws);
        assert_eq!(
            s.evaluate(Path::new("/etc/hosts"), Op::Write),
            Decision::Allow
        );
        assert!(!s.wraps_bash());
    }

    #[test]
    fn enabled_gates_writes_outside_workspace() {
        let ws = temp_workspace("enabled");
        let s = enabled(&ws);
        // In-workspace write allowed; outside asks.
        assert_eq!(
            s.evaluate(Path::new(&format!("{ws}/a.txt")), Op::Write),
            Decision::Allow
        );
        let outside = dirs::home_dir().unwrap().join("futureos-x-outside.txt");
        assert_eq!(s.evaluate(&outside, Op::Write), Decision::Ask);
        assert!(!s.write_allowed(outside.to_string_lossy().as_ref()));
    }

    #[test]
    fn session_allow_takes_effect() {
        let ws = temp_workspace("session");
        let s = enabled(&ws);
        let outside = dirs::home_dir().unwrap().join("futureos-notes");
        assert_eq!(s.evaluate(&outside, Op::Write), Decision::Ask);
        s.add_session_allow(&outside.to_string_lossy(), Op::Write);
        assert_eq!(s.evaluate(&outside, Op::Write), Decision::Allow);
    }

    #[test]
    fn denial_heuristic_only_fs_eperm() {
        let ws = temp_workspace("heuristic");
        let s = enabled(&ws);
        assert!(!looks_like_sandbox_denial(&s, 1, "error[E0308]"));
        assert!(looks_like_sandbox_denial(
            &s,
            1,
            "touch: /etc/x: Operation not permitted"
        ));
        // Network errors are NOT sandbox denials anymore (network is open).
        assert!(!looks_like_sandbox_denial(
            &s,
            6,
            "curl: (6) Could not resolve host"
        ));
    }
}
