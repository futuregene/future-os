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
/// Windows: the resolved PowerShell (pwsh 7+ when available, else Windows
/// PowerShell 5.1 — see [`windows_shell`]) with a wrapper script delivered via
/// `-EncodedCommand`. Base64/UTF-16LE encoding sidesteps the fragile
/// Rust→CreateProcess→PowerShell quote re-parsing that plain `-Command` is
/// subject to; the wrapper itself ([`windows_wrapper_script`]) handles stderr
/// merging and exit-code capture, which both differ from bash semantics.
pub fn shell_invocation(command: &str) -> (&'static str, Vec<String>) {
    #[cfg(not(target_os = "windows"))]
    {
        (unix_shell(), vec!["-c".to_string(), command.to_string()])
    }
    #[cfg(target_os = "windows")]
    {
        // Use cmd /c instead of PowerShell.  PowerShell tracks the
        // entire sub-process tree and will not exit until all child
        // processes (including Chrome auto-started by the CLI via
        // cmd /c start) have closed their handles.  cmd /c passes
        // through the exit code directly and terminates regardless.
        // The CLI's args parser handles cmd-quoting via multiple
        // parse strategies in parseToolArgs.
        ("cmd", vec!["/c".to_string(), command.to_string()])
    }
}

/// Build the PowerShell wrapper script for one user command. Split out from
/// [`shell_invocation`] so it can be asserted on directly (the encoded form is
/// opaque). The wrapper differs from a bash `( … ) 2>&1`:
/// - `& { … }` runs the command in a script block (accepts multi-statement
///   commands, unlike `( … )`), with `2>&1` merging the error stream and
///   `ForEach-Object { "$_" }` stringifying error records to plain text.
/// - `$LASTEXITCODE` only reflects native (.exe) processes. A PowerShell-level
///   failure — command not found, cmdlet error — never sets it, and `chcp`
///   pollutes it with 0, so it is cleared first and `$Error` catches failures
///   where no native command ran at all.
/// - Non-ASCII output (e.g. Chinese) survives capture. Three encodings must
///   line up, and Windows PowerShell 5.1 gets all three wrong by default:
///   * `chcp 65001` asks native (.exe) children to emit UTF-8.
///   * `[Console]::OutputEncoding` governs both how PowerShell decodes a native
///     child's stdout and how it encodes its own stdout (the bytes we capture).
///   * `$OutputEncoding` governs how PowerShell encodes strings piped INTO a
///     native command's stdin — it defaults to ASCII in 5.1, mangling non-ASCII
///     to `?`, so it must be set too.
///   All three use a BOM-less `UTF8Encoding($false)`: the default
///   `[Text.Encoding]::UTF8` carries a BOM that, on a redirected stdout, PS 5.1
///   prepends to the stream as a stray `EF BB BF` (a leading U+FEFF for us).
///   (pwsh 7 already defaults to BOM-less UTF-8; setting these is a harmless
///   no-op there.) Native tools that ignore the code page and hard-code OEM/
///   ANSI output can't be fixed here — those bytes become replacement chars
///   via `from_utf8_lossy` rather than corrupting the capture.
/// - `$ProgressPreference = 'SilentlyContinue'` suppresses progress records
///   (e.g. "Preparing modules for first use"). When powershell.exe's stderr is
///   a redirected pipe, PS 5.1 serializes such records as CLIXML (`#< CLIXML …`)
///   onto that stderr, which our capture would otherwise splice into the output.
#[cfg(target_os = "windows")]
pub fn windows_wrapper_script(command: &str) -> String {
    // The model may generate bash-style double-quoted-with-escapes content
    // (`{\"key\":\"val\"}`); PowerShell does not treat `\"` as an escape, so
    // reshape it to a form PowerShell parses (see `normalize_shell_quoting`).
    let command = ResolvedSandbox::normalize_shell_quoting(command);
    format!(
        "chcp 65001 > $null; \
         $OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); \
         $ProgressPreference = 'SilentlyContinue'; \
         $global:LASTEXITCODE = $null; \
         & {{ {} }} 2>&1 | ForEach-Object {{ \"$_\" }}; \
         if ($null -ne $LASTEXITCODE) {{ exit $LASTEXITCODE }} \
         elseif ($Error.Count -gt 0) {{ exit 1 }} \
         else {{ exit 0 }}",
        command
    )
}

/// Encode a script for PowerShell's `-EncodedCommand`: base64 of UTF-16LE.
#[allow(dead_code)]
#[cfg(target_os = "windows")]
fn encode_powershell_command(script: &str) -> String {
    use base64::Engine;
    let utf16: Vec<u8> = script
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect();
    base64::engine::general_purpose::STANDARD.encode(utf16)
}

/// The resolved Windows shell for command execution. pwsh (PowerShell 7+) is
/// preferred when on PATH: it supports `&&`/`||` chain operators, defaults to
/// UTF-8, and parses `-EncodedCommand` identically to 5.1. Falls back to the
/// always-present `powershell` (Windows PowerShell 5.1). Probed once.
#[cfg(target_os = "windows")]
pub struct WindowsShell {
    pub program: &'static str,
    /// pwsh 7+ supports `&&` / `||`; Windows PowerShell 5.1 does not.
    pub supports_chain_operators: bool,
}

#[cfg(target_os = "windows")]
pub fn windows_shell() -> &'static WindowsShell {
    use std::sync::OnceLock;
    static SHELL: OnceLock<WindowsShell> = OnceLock::new();
    SHELL.get_or_init(|| {
        if pwsh_on_path() {
            WindowsShell {
                program: "pwsh",
                supports_chain_operators: true,
            }
        } else {
            WindowsShell {
                program: "powershell",
                supports_chain_operators: false,
            }
        }
    })
}

/// Whether `pwsh.exe` (PowerShell 7+) resolves on PATH. A pure env scan — no
/// process spawn — so it is cheap and side-effect-free.
#[cfg(target_os = "windows")]
fn pwsh_on_path() -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join("pwsh.exe").is_file())
}

/// The shell used to execute commands on Unix, resolved once. Honors the
/// user's `$SHELL` when it is bash or zsh (both POSIX-compatible with the
/// `( … ) 2>&1` wrapper the caller applies); otherwise probes for bash then
/// zsh on PATH, falling back to `sh`. Never fish/nu — their syntax would break
/// the wrapper. Returns a program name or absolute path for `Command::new`.
#[cfg(not(target_os = "windows"))]
pub fn unix_shell() -> &'static str {
    use std::sync::OnceLock;
    static SHELL: OnceLock<String> = OnceLock::new();
    SHELL.get_or_init(|| {
        // $SHELL, but only if it is a bash/zsh we can actually run.
        if let Some(raw) = std::env::var_os("SHELL") {
            let path = PathBuf::from(&raw);
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if (name == "bash" || name == "zsh") && path.is_file() {
                return raw.to_string_lossy().into_owned();
            }
        }
        for cand in ["bash", "zsh"] {
            if on_path(cand) {
                return cand.to_string();
            }
        }
        // Last resort: POSIX sh is guaranteed present, and our wrapper is
        // POSIX-safe.
        "sh".to_string()
    })
}

/// Basename of the resolved Unix shell for prompt text ("bash" / "zsh" / "sh").
#[cfg(not(target_os = "windows"))]
fn unix_shell_display_name() -> &'static str {
    let shell = unix_shell();
    shell.rsplit('/').next().unwrap_or(shell)
}

/// Whether an executable named `name` resolves on PATH. Pure env scan.
#[cfg(not(target_os = "windows"))]
fn on_path(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(name).is_file())
}

/// Load the user's login-shell environment into this process at startup, so
/// commands find tools the user installed via their shell rc (nvm/pyenv/conda,
/// Homebrew, npm-global) — not just the minimal PATH a GUI launched from the
/// Finder/dock inherits. Mirrors what VS Code and similar tools do.
///
/// Runs `$SHELL -l -i -c` once to dump `env` between markers (rc noise on
/// stderr is discarded; a 5s timeout guards against a hanging rc). PATH is
/// always taken from the login shell; other vars are merged only when absent,
/// so intentional launcher overrides are never clobbered. No-op on Windows,
/// where GUI processes already inherit the full registry PATH.
#[cfg(not(target_os = "windows"))]
pub fn hydrate_from_login_shell() {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;

    // The shell whose rc files define the user's real env — their actual login
    // shell ($SHELL), even if it is fish/nu (we only harvest the resulting env,
    // we don't run shell-specific syntax beyond printf + the env binary).
    let shell = std::env::var("SHELL").unwrap_or_else(|_| unix_shell().to_string());
    let marker = "__future_env_boundary_9c4f__";
    let script = format!("printf '%s' '{marker}'; /usr/bin/env; printf '%s' '{marker}'");

    let mut child = match Command::new(&shell)
        .args(["-l", "-i", "-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!("login-shell env hydration skipped: spawn {shell} failed: {e}");
            return;
        }
    };

    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        return;
    };
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = stdout.read_to_string(&mut buf);
        let _ = tx.send(buf);
    });

    let dump = match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(buf) => {
            let _ = child.wait();
            buf
        }
        Err(_) => {
            tracing::debug!("login-shell env hydration timed out; using inherited env");
            let _ = child.kill();
            let _ = child.wait();
            return;
        }
    };

    // Content strictly between the two markers is the env dump (rc scripts may
    // print before the first marker; we ignore that).
    let Some(start) = dump.find(marker) else {
        return;
    };
    let after = &dump[start + marker.len()..];
    let body = after.split(marker).next().unwrap_or("");

    let mut applied_path = false;
    let mut merged = 0usize;
    for line in body.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.is_empty() {
            continue;
        }
        if key == "PATH" {
            std::env::set_var("PATH", value);
            applied_path = true;
        } else if std::env::var_os(key).is_none() {
            // Additive only: never overwrite a var the launcher set on purpose.
            std::env::set_var(key, value);
            merged += 1;
        }
    }
    if applied_path {
        tracing::info!("hydrated PATH from login shell ({shell}); merged {merged} env vars");
    }
}

/// No-op on Windows — GUI processes already inherit the full registry PATH.
#[cfg(target_os = "windows")]
pub fn hydrate_from_login_shell() {}

/// Runtime hint for prompt text: does the host's shell support `&&`/`||`
/// chaining? True for any POSIX shell and for pwsh 7 on Windows; false for
/// Windows PowerShell 5.1. Callable on every target so prompt code that runs
/// per-host (not `#[cfg]`-gated) can consult it.
pub fn shell_supports_chain_operators() -> bool {
    #[cfg(target_os = "windows")]
    {
        windows_shell().supports_chain_operators
    }
    #[cfg(not(target_os = "windows"))]
    {
        true
    }
}

/// Display name of the host shell for prompt text (e.g. "bash",
/// "PowerShell 7 (pwsh)", "Windows PowerShell 5.1"). Callable on every target.
pub fn shell_display_name() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        if windows_shell().supports_chain_operators {
            "PowerShell 7 (pwsh)"
        } else {
            "Windows PowerShell 5.1"
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        unix_shell_display_name()
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
    fn shell_invocation_unix_passes_command_through_to_the_resolved_shell() {
        let (program, args) = shell_invocation("echo hi; false");
        // The command is passed verbatim to `-c`; the program is the resolved
        // shell (bash/zsh/sh or an absolute $SHELL path), never fish/nu.
        assert_eq!(args, vec!["-c".to_string(), "echo hi; false".to_string()]);
        let name = program.rsplit('/').next().unwrap_or(program);
        assert!(
            matches!(name, "bash" | "zsh" | "sh"),
            "unexpected shell: {program}"
        );
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn windows_wrapper_script_captures_exit_state() {
        let script = windows_wrapper_script("Get-ChildItem");
        // chcp pollutes $LASTEXITCODE with 0 — it must be cleared before the
        // user command so a PowerShell-level failure can't masquerade as exit 0.
        assert!(script.contains("$global:LASTEXITCODE = $null"));
        // Script block (not `( … )`) so multi-statement commands parse.
        assert!(script.contains("& { Get-ChildItem } 2>&1"));
        // Native exit code passes through; $Error catches cmdlet/not-found
        // failures that never set $LASTEXITCODE.
        assert!(script.contains("exit $LASTEXITCODE"));
        assert!(script.contains("$Error.Count"));
        // BOM-less UTF-8 on both stdout and pipe-to-native-stdin (PS 5.1
        // defaults leak a BOM / ASCII respectively).
        assert!(script.contains("[System.Text.UTF8Encoding]::new($false)"));
        assert!(script.contains("$OutputEncoding = [Console]::OutputEncoding"));
        // Progress suppressed so PS 5.1 doesn't serialize "Preparing modules…"
        // as CLIXML onto the redirected stderr we capture.
        assert!(script.contains("$ProgressPreference = 'SilentlyContinue'"));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn shell_invocation_windows_uses_encoded_command() {
        let (program, args) = shell_invocation("Get-ChildItem");
        // pwsh when present, else Windows PowerShell 5.1 — both accept these args.
        assert!(program == "pwsh" || program == "powershell");
        // Non-interactive so a prompt can't hang the agent; profile/logo off.
        assert!(args.contains(&"-NoProfile".to_string()));
        assert!(args.contains(&"-NonInteractive".to_string()));
        // The command is the base64 payload right after -EncodedCommand.
        let enc = args
            .iter()
            .position(|a| a == "-EncodedCommand")
            .expect("has -EncodedCommand");
        let payload = &args[enc + 1];
        // The payload is base64 of the UTF-16LE wrapper script; decode and
        // confirm it round-trips to the readable wrapper.
        use base64::Engine;
        let raw = base64::engine::general_purpose::STANDARD
            .decode(payload)
            .expect("valid base64");
        let utf16: Vec<u16> = raw
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        let decoded = String::from_utf16(&utf16).expect("valid utf-16");
        assert!(decoded.contains("& { Get-ChildItem } 2>&1"));
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
