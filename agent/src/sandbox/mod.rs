//! OS-level sandbox + path-based approval rules (APPROVAL_PLAN.md / SANDBOX_PLAN.md).
//!
//! Every approval is about a file-path access: [`rules::RuleSet`] resolves a
//! path + op to `Ask | Allow | Deny`. That verdict is enforced two ways:
//!   - read/write/edit tools: the approval layer prompts (Ask) / proceeds
//!     (Allow) / errors (Deny) before the in-process op runs.
//!   - shell: the rules compile into a Seatbelt profile (macOS); Ask and Deny
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

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rules::{Decision, Op, RuleSet};

/// The user-selected approval tier (composer / settings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SandboxTier {
    /// Off — no approval, no sandbox, everything runs.
    Off,
    /// Manual — approval rules on; shell asks (read-only allowlist bypass); no OS
    /// sandbox. The default, all platforms.
    #[default]
    Manual,
    /// Sandbox — approval rules on; shell runs inside the OS sandbox (macOS
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

    /// Whether shell commands run pre-approval-gated (manual tier, or a sandbox tier on a
    /// platform without the OS sandbox). When true, the shell asks (allowlist bypass);
    /// when false and enabled, the shell is OS-sandboxed instead.
    pub fn shell_needs_approval(&self) -> bool {
        self.enabled() && !self.wraps_shell()
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

    /// Whether shell commands run wrapped in the OS sandbox (Sandbox tier on a
    /// platform where sandbox-exec is available).
    pub fn wraps_shell(&self) -> bool {
        self.tier == SandboxTier::Sandbox && self.available
    }

    /// Read access to the resolved rule set (Seatbelt profile builder).
    pub fn rule_set(&self) -> &RuleSet {
        &self.rules
    }

    /// Build the shell invocation: Seatbelt-wrapped when enabled+available and
    /// not escalated; otherwise the platform shell via [`shell_invocation`].
    /// `escalated` forces an unsandboxed run for one approved command.
    pub fn build_shell_command(&self, command: &str, escalated: bool) -> tokio::process::Command {
        if !escalated && self.wraps_shell() {
            #[cfg(target_os = "macos")]
            {
                return seatbelt::build_command(self, command);
            }
        }
        let (program, args) = shell_invocation(command);
        let mut child = tokio::process::Command::new(program);
        child.args(&args);
        child
    }

    /// Convert bash-style escaped double quotes (\") to single-quoted form
    /// so PowerShell can parse the arguments correctly. Also handles
    /// PowerShell backtick escapes (`", `{, `}) and strips any explicit
    /// powershell -Command wrapper the model may have generated (the agent
    /// already wraps commands in PowerShell).
    #[cfg(windows)]
    fn normalize_shell_quoting(command: &str) -> String {
        let command = command.trim();

        // Strip explicit powershell -Command "..." wrapper if the model
        // generated it — the agent already wraps commands in PowerShell.
        let command = if command.starts_with("powershell ") {
            let inner = command["powershell ".len()..].trim();
            let inner = inner
                .trim_start_matches("-Command ")
                .trim_start_matches("-c ")
                .trim();
            if (inner.starts_with('"') && inner.ends_with('"'))
                || (inner.starts_with('\'') && inner.ends_with('\''))
            {
                &inner[1..inner.len() - 1]
            } else {
                inner
            }
        } else {
            command
        };

        // Unescape PowerShell backtick escapes (backtick is PowerShell's
        // escape character; the model sometimes generates `" instead of \").
        let command = command
            .replace("`\"", "\"")
            .replace("`{", "{")
            .replace("`}", "}");

        let chars: Vec<char> = command.chars().collect();
        let mut result = String::with_capacity(command.len());
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '"' && i + 1 < chars.len() && chars[i + 1] == '{' {
                // Potential JSON argument in double quotes — find closing quote
                let end = match Self::find_closing_quote(&chars, i) {
                    Some(e) => e,
                    None => {
                        result.push(chars[i]);
                        i += 1;
                        continue;
                    }
                };
                let inner: String = chars[i + 1..end].iter().collect();
                if inner.contains("\\\"") {
                    // Bash-style: unescape and re-wrap in single quotes
                    result.push('\'');
                    result.push_str(&inner.replace("\\\"", "\""));
                    result.push('\'');
                } else {
                    // No escapes, pass through
                    for j in i..=end {
                        result.push(chars[j]);
                    }
                }
                i = end + 1;
            } else {
                result.push(chars[i]);
                i += 1;
            }
        }
        result
    }

    /// Find the closing double quote for a JSON-like argument starting at
    /// `start`, skipping over bash-style \" escaped quotes inside.
    #[cfg(windows)]
    fn find_closing_quote(chars: &[char], start: usize) -> Option<usize> {
        let mut i = start + 1;
        while i < chars.len() {
            if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '"' {
                i += 2; // skip \"
            } else if chars[i] == '"' {
                return Some(i);
            } else {
                i += 1;
            }
        }
        None
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

/// The platform shell invocation for one command string — the single source of
/// truth for how a shell command is executed on this OS.
///
/// Unix: `bash -c <command>` — the command arrives pre-wrapped by the caller
/// when stderr merging is wanted (`( … ) 2>&1`).
///
/// Windows: a PowerShell 5.1 wrapper that handles merging and exit capture
/// itself, because both differ from bash semantics:
/// - `& { … }` runs the command in a script block (accepts multi-statement
///   commands, unlike `( … )`), with `2>&1` merging the error stream and
///   `ForEach-Object { "$_" }` stringifying error records to plain text.
/// - `$LASTEXITCODE` only reflects native (.exe) processes. A PowerShell-level
///   failure — command not found, cmdlet error — never sets it, and `chcp`
///   pollutes it with 0, so it is cleared first and `$Error` catches failures
///   where no native command ran at all.
/// - `chcp 65001` + `[Console]::OutputEncoding` keep non-ASCII output (e.g.
///   Chinese) from being garbled by the default GBK/ANSI code page.
pub fn shell_invocation(command: &str) -> (&'static str, Vec<String>) {
    #[cfg(not(target_os = "windows"))]
    {
        ("bash", vec!["-c".to_string(), command.to_string()])
    }
    #[cfg(target_os = "windows")]
    {
        // The model may generate either single-quoted JSON args
        // (--args '{"key":"val"}') or bash-style double-quoted-with-escapes
        // (--args "{\"key\":\"val\"}"). PowerShell handles single quotes
        // natively but breaks on \" because backslash is not an escape
        // character there (PowerShell uses backtick `" instead).
        let command = ResolvedSandbox::normalize_shell_quoting(command);
        let script = format!(
            "chcp 65001 > $null; \
             [Console]::OutputEncoding = [System.Text.Encoding]::UTF8; \
             $global:LASTEXITCODE = $null; \
             & {{ {} }} 2>&1 | ForEach-Object {{ \"$_\" }}; \
             if ($null -ne $LASTEXITCODE) {{ exit $LASTEXITCODE }} \
             elseif ($Error.Count -gt 0) {{ exit 1 }} \
             else {{ exit 0 }}",
            command
        );
        (
            "powershell",
            vec!["-NoProfile".to_string(), "-Command".to_string(), script],
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
/// shell tool after a sandbox denial or when the model asks for it explicitly.
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

/// Callback the RPC layer injects so `run_shell` can raise a `sandbox_escalation`
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
    fn tier_maps_shell_handling() {
        let ws = temp_workspace("tiers");
        let mut manual = enabled(&ws);
        manual.available = true;
        // Manual: shell needs approval, never OS-wrapped, even where available.
        assert!(!manual.wraps_shell());
        assert!(manual.shell_needs_approval());

        let mut sandbox = ResolvedSandbox::resolve(
            &SandboxPolicy {
                tier: SandboxTier::Sandbox,
            },
            &ws,
        );
        sandbox.available = true;
        assert!(sandbox.wraps_shell());
        assert!(!sandbox.shell_needs_approval());
        // Sandbox tier without the OS sandbox falls back to shell approval.
        sandbox.available = false;
        assert!(!sandbox.wraps_shell());
        assert!(sandbox.shell_needs_approval());

        let off = ResolvedSandbox::disabled(&ws);
        assert!(!off.enabled());
        assert!(!off.shell_needs_approval());
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn shell_invocation_unix_is_bash_c_passthrough() {
        let (program, args) = shell_invocation("echo hi; false");
        assert_eq!(program, "bash");
        assert_eq!(args, vec!["-c".to_string(), "echo hi; false".to_string()]);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn shell_invocation_windows_wrapper_captures_exit_state() {
        let (program, args) = shell_invocation("Get-ChildItem");
        assert_eq!(program, "powershell");
        assert_eq!(args[0], "-NoProfile");
        assert_eq!(args[1], "-Command");
        let script = &args[2];
        // chcp pollutes $LASTEXITCODE with 0 — it must be cleared before the
        // user command so a PowerShell-level failure can't masquerade as exit 0.
        assert!(script.contains("$global:LASTEXITCODE = $null"));
        // Script block (not `( … )`) so multi-statement commands parse.
        assert!(script.contains("& { Get-ChildItem } 2>&1"));
        // Native exit code passes through; $Error catches cmdlet/not-found
        // failures that never set $LASTEXITCODE.
        assert!(script.contains("exit $LASTEXITCODE"));
        assert!(script.contains("$Error.Count"));
    }

    #[test]
    fn disabled_allows_everything() {
        let ws = temp_workspace("disabled");
        let s = ResolvedSandbox::disabled(&ws);
        assert_eq!(
            s.evaluate(Path::new("/etc/hosts"), Op::Write),
            Decision::Allow
        );
        assert!(!s.wraps_shell());
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
