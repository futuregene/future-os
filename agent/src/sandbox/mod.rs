//! OS-level sandbox + path-based approval rules (APPROVAL_PLAN.md / SANDBOX_PLAN.md).
//!
//! Every approval is about a file-path access: [`rules::RuleSet`] resolves a
//! path + op to `Ask | Allow | Deny`. That verdict is enforced two ways:
//!   - read/write/edit tools: the approval layer prompts (Ask) / proceeds
//!     (Allow) / errors (Deny) before the in-process op runs.
//!   - shell: the rules compile into a Seatbelt profile (macOS) or Bubblewrap
//!     mount plan (Linux); Ask and Deny become OS-level read/write denials, and
//!     a resulting failure surfaces via the escalation flow.
//!
//! Network is unrestricted. The whole system is gated by `enabled`: only GUI
//! sessions opt in; everything else runs fully open.

pub mod backend;
pub mod linux;
pub mod paths;
pub mod rules;
mod seatbelt;
#[cfg(windows)]
pub mod windows;
#[path = "windows/capability.rs"]
mod windows_capability;
mod windows_plan;
pub(crate) mod windows_request;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use backend::PreparedShell;
use rules::{Decision, Op, RuleSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxBackendReceipt {
    Unavailable,
    MacosSeatbelt {
        executable: PathBuf,
    },
    LinuxBubblewrap {
        probe: linux::probe::LinuxSandboxProbe,
    },
    WindowsRestricted,
}

impl SandboxBackendReceipt {
    pub fn is_available(&self) -> bool {
        !matches!(self, Self::Unavailable)
    }
}

/// The user-selected approval tier (composer / settings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SandboxTier {
    /// Off — no approval, no sandbox, everything runs.
    Off,
    /// Manual — approval rules on; shell asks (read-only allowlist bypass); no OS
    /// sandbox. The default, all platforms.
    #[default]
    Manual,
    /// Sandbox — approval rules on; shell runs inside the available OS sandbox.
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
    /// Verified backend identity/capability receipt. Availability is derived
    /// from this receipt rather than maintained as an independent boolean.
    pub backend_receipt: SandboxBackendReceipt,
    /// Canonicalized workspace directory.
    pub workspace: PathBuf,
    rules: RuleSet,
}

impl ResolvedSandbox {
    /// Resolve rules for `workspace`. The tier comes from the session policy.
    pub fn resolve(policy: &SandboxPolicy, workspace: &str) -> Self {
        let rules = RuleSet::resolve(Path::new(workspace));
        let backend_receipt = platform_backend_receipt();
        let tier = effective_tier(policy.tier, &backend_receipt);
        Self {
            tier,
            backend_receipt,
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
        let backend_receipt = platform_backend_receipt();
        let tier = effective_tier(policy.tier, &backend_receipt);
        Self {
            tier,
            backend_receipt,
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
            backend_receipt: SandboxBackendReceipt::Unavailable,
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
        self.tier == SandboxTier::Sandbox && self.backend_receipt.is_available()
    }

    #[cfg(test)]
    pub(crate) fn set_backend_available_for_test(&mut self, available: bool) {
        self.backend_receipt = if available {
            self.tier = SandboxTier::Sandbox;
            SandboxBackendReceipt::MacosSeatbelt {
                executable: PathBuf::from("/usr/bin/sandbox-exec"),
            }
        } else {
            SandboxBackendReceipt::Unavailable
        };
    }

    #[cfg(test)]
    pub(crate) fn set_linux_backend_available_for_test(&mut self) {
        self.tier = SandboxTier::Sandbox;
        self.backend_receipt = SandboxBackendReceipt::LinuxBubblewrap {
            probe: linux::probe::LinuxSandboxProbe {
                available: true,
                code: linux::probe::LinuxSandboxProbeCode::Available,
                path: Some(PathBuf::from("/usr/bin/bwrap")),
                version: Some("test".into()),
                identity: Some(linux::probe::BwrapIdentity {
                    device: 1,
                    inode: 2,
                    size: 3,
                    modified_nanos: 4,
                }),
                capabilities: None,
                expires_at_unix_ms: None,
                diagnostic: None,
            },
        };
    }

    /// Read access to the resolved rule set (Seatbelt profile builder).
    pub fn rule_set(&self) -> &RuleSet {
        &self.rules
    }

    /// Prepare a structured shell invocation. Backend construction failures
    /// are infrastructure errors: callers must return them directly rather
    /// than feeding them into sandbox-denial escalation.
    pub fn prepare_shell_for_cwd(
        &self,
        command: &str,
        escalated: bool,
        cwd: &Path,
    ) -> anyhow::Result<PreparedShell> {
        #[cfg(not(target_os = "linux"))]
        let _ = cwd;
        if !escalated && self.wraps_shell() {
            #[cfg(target_os = "macos")]
            {
                return Ok(seatbelt::prepare(self, command));
            }
            #[cfg(target_os = "linux")]
            if let SandboxBackendReceipt::LinuxBubblewrap { probe } = &self.backend_receipt {
                let plan = linux::plan::LinuxSandboxPlan::compile(&self.rules.snapshot())?;
                return Ok(linux::runner::prepare(probe, plan, command, cwd)?);
            }
        }
        Ok(PreparedShell::plain(command))
    }

    pub fn prepare_shell(&self, command: &str, escalated: bool) -> PreparedShell {
        self.prepare_shell_for_cwd(command, escalated, &self.workspace)
            .expect("sandbox backend preparation failed")
    }

    /// Compatibility adapter for callers that need a Tokio command.
    pub fn build_shell_command(&self, command: &str, escalated: bool) -> tokio::process::Command {
        self.prepare_shell(command, escalated)
            .into_command()
            .expect("prepared shell command construction failed")
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
        let command = if let Some(rest) = command.strip_prefix("powershell ") {
            let inner = rest.trim();
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
                    result.extend(&chars[i..=end]);
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
        let backend = match self.backend_receipt {
            SandboxBackendReceipt::Unavailable => "none",
            SandboxBackendReceipt::MacosSeatbelt { .. } => "macos_seatbelt",
            SandboxBackendReceipt::LinuxBubblewrap { .. } => "linux_bubblewrap",
            SandboxBackendReceipt::WindowsRestricted => "windows_restricted",
        };
        let policy_digest = linux::plan::policy_digest(&self.rules.snapshot()).ok();
        serde_json::json!({
            "inside_sandbox": inside_sandbox,
            "sandbox_available": self.backend_receipt.is_available(),
            "backend": backend,
            "policy_digest": policy_digest,
            "tier": self.tier.as_str(),
            "violation": violation,
            "cwd": self.workspace.to_string_lossy(),
        })
    }
}

fn effective_tier(requested: SandboxTier, receipt: &SandboxBackendReceipt) -> SandboxTier {
    if requested == SandboxTier::Sandbox && !receipt.is_available() {
        tracing::warn!(
            "OS sandbox requested but unavailable; explicitly falling back to manual approval"
        );
        SandboxTier::Manual
    } else {
        requested
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
        let script = windows_wrapper_script(command);
        (
            windows_shell().program,
            vec![
                // -NoProfile: skip profile scripts (speed + no stray output).
                // -NonInteractive: fail fast instead of hanging on a prompt —
                //   there is no console for the agent to answer one.
                // -NoLogo: suppress the startup banner on 5.1.
                "-NoProfile".to_string(),
                "-NonInteractive".to_string(),
                "-NoLogo".to_string(),
                "-EncodedCommand".to_string(),
                encode_powershell_command(&script),
            ],
        )
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
///
///   Use the existing `[Text.Encoding]::UTF8` static instance rather than
///   constructing `UTF8Encoding($false)`: a WRITE_RESTRICTED token puts Windows
///   PowerShell 5.1 in Constrained Language Mode, which rejects construction of
///   that .NET type. Each assignment is wrapped in its own `try` so a blocked
///   one cannot prevent the others; `$Error.Count` is recorded first so a setup
///   failure cannot turn a successful user command into `exit 1`.
///
///   Constrained Language Mode also rejects the `[Console]::OutputEncoding`
///   setter outright, so a restricted PowerShell 5.1 keeps emitting in its
///   console output code page no matter what this script does. With stdout
///   redirected to a pipe there is no console, so `[Console]::OutputEncoding`
///   resolves to the OEM code page (`GetOEMCP`), not the ANSI code page
///   (`GetACP`); the reader decodes it back with
///   [`decode_restricted_shell_output`] (OEM for 5.1, UTF-8 for pwsh 7).
///   pwsh 7 always emits UTF-8 and is unaffected by either restriction.
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
         try {{ $OutputEncoding = [System.Text.Encoding]::UTF8 }} catch {{}}; \
         try {{ [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 }} catch {{}}; \
         $ProgressPreference = 'SilentlyContinue'; \
         $global:LASTEXITCODE = $null; \
         $futureosInitialErrorCount = $Error.Count; \
         & {{ {} }} 2>&1 | ForEach-Object {{ \"$_\" }}; \
         if ($null -ne $LASTEXITCODE) {{ exit $LASTEXITCODE }} \
         elseif ($Error.Count -gt $futureosInitialErrorCount) {{ exit 1 }} \
         else {{ exit 0 }}",
        command
    )
}

/// Encode a script for PowerShell's `-EncodedCommand`: base64 of UTF-16LE.
#[cfg(target_os = "windows")]
fn encode_powershell_command(script: &str) -> String {
    use base64::Engine;
    let utf16: Vec<u8> = script
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect();
    base64::engine::general_purpose::STANDARD.encode(utf16)
}

/// Decode captured shell output using the system legacy OEM code page
/// (`GetOEMCP`). Windows PowerShell 5.1 running under a WRITE_RESTRICTED token
/// enters Constrained Language Mode, where it cannot assign
/// `[Console]::OutputEncoding`; with stdout redirected to a pipe there is no
/// console, so that property resolves to the OEM code page rather than the ANSI
/// code page. On CJK locales the two coincide (e.g. both 936/GBK), which hides
/// the distinction, but on Western/Russian/Greek locales they differ (ANSI
/// 1252 vs OEM 437/850, 1251 vs 866, 1253 vs 737), so decoding with `CP_ACP`
/// would corrupt non-ASCII output there. pwsh 7 hard-codes UTF-8 and is
/// unaffected; callers only use this for the 5.1 restricted path.
#[cfg(target_os = "windows")]
pub(crate) fn decode_oem_lossy(bytes: &[u8]) -> String {
    use windows_sys::Win32::Globalization::{MultiByteToWideChar, CP_OEMCP};

    if bytes.is_empty() {
        return String::new();
    }
    // First pass: query the required UTF-16 length.
    let wide_len = unsafe {
        MultiByteToWideChar(
            CP_OEMCP,
            0,
            bytes.as_ptr(),
            bytes.len() as i32,
            std::ptr::null_mut(),
            0,
        )
    };
    if wide_len <= 0 {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let mut wide = vec![0u16; wide_len as usize];
    let written = unsafe {
        MultiByteToWideChar(
            CP_OEMCP,
            0,
            bytes.as_ptr(),
            bytes.len() as i32,
            wide.as_mut_ptr(),
            wide_len,
        )
    };
    if written <= 0 {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    wide.truncate(written as usize);
    String::from_utf16_lossy(&wide)
}

/// Decode output captured from a restricted Windows shell. Windows PowerShell
/// 5.1 cannot set UTF-8 under Constrained Language Mode and therefore emits in
/// its console output code page, which is the OEM code page when stdout is a
/// pipe; pwsh 7 always emits UTF-8. Everything else is UTF-8.
#[cfg(target_os = "windows")]
pub(crate) fn decode_restricted_shell_output(bytes: &[u8]) -> String {
    if windows_shell().program == "powershell" {
        decode_oem_lossy(bytes)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
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
    SHELL.get_or_init(|| resolve_unix_shell(std::env::var_os("SHELL"), &on_path))
}

/// Pure shell-resolution logic (env + PATH probe injected for tests):
/// `$SHELL` when it names an executable bash/zsh, else the first of
/// bash/zsh on PATH, else `sh`.
#[cfg(not(target_os = "windows"))]
fn resolve_unix_shell(
    shell_env: Option<std::ffi::OsString>,
    on_path: &dyn Fn(&str) -> bool,
) -> String {
    // $SHELL, but only if it is a bash/zsh we can actually run.
    if let Some(raw) = shell_env {
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
}

/// Basename of the resolved Unix shell for prompt text ("bash" / "zsh" / "sh").
#[cfg(not(target_os = "windows"))]
fn unix_shell_display_name() -> &'static str {
    let shell = unix_shell();
    shell.rsplit('/').next().unwrap_or(shell)
}

#[cfg(not(target_os = "windows"))]
fn probe_legacy_bash(shell: &str, timeout: std::time::Duration) -> Option<bool> {
    let mut child = std::process::Command::new(shell)
        .args([
            "--noprofile",
            "--norc",
            "-c",
            "if (( BASH_VERSINFO[0] < 4 )); then exit 0; else exit 1; fi",
        ])
        // Non-interactive bash still evaluates BASH_ENV when it is set. A
        // version probe must not run user startup code or inherit its latency.
        .env_remove("BASH_ENV")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + timeout;

    loop {
        match child.try_wait() {
            Ok(Some(status)) => return legacy_bash_from_exit_code(status.code()),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Ok(None) | Err(_) => {
                // Do not leave a failed version probe running in the background.
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn legacy_bash_from_exit_code(code: Option<i32>) -> Option<bool> {
    match code {
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    }
}

/// Whether the resolved Unix shell is a legacy bash (< 4.0) that lacks
/// associative arrays, globstar, and other bash 4+ features.
/// Used by the prompt layer to constrain LLM-generated commands to
/// POSIX-compatible syntax. The probe runs once with a 2-second timeout and
/// conservatively treats a probe failure as legacy. Always false on Windows.
pub fn shell_is_legacy_bash() -> bool {
    #[cfg(not(target_os = "windows"))]
    {
        use std::sync::OnceLock;
        static LEGACY: OnceLock<bool> = OnceLock::new();
        *LEGACY.get_or_init(|| legacy_bash_probe(unix_shell()))
    }
    #[cfg(target_os = "windows")]
    {
        false
    }
}

/// Probe result for a given shell path: true only when the shell is bash
/// and the version probe confirms (or conservatively assumes) legacy.
/// Extracted from the OnceLock closure so both the bash and non-bash edges
/// are directly testable (the OnceLock initializes at most once per
/// process, with whatever $SHELL the test runner has).
#[cfg(not(target_os = "windows"))]
fn legacy_bash_probe(shell: &str) -> bool {
    let name = shell.rsplit('/').next().unwrap_or(shell);
    name == "bash" && probe_legacy_bash(shell, std::time::Duration::from_secs(2)).unwrap_or(true)
}

/// Whether an executable named `name` resolves on PATH. Pure env scan.
#[cfg(not(target_os = "windows"))]
fn on_path(name: &str) -> bool {
    on_path_in(std::env::var_os("PATH"), name)
}

/// `on_path` with the PATH value injected, so the no-PATH arm is testable
/// without mutating the process environment.
#[cfg(not(target_os = "windows"))]
fn on_path_in(path: Option<std::ffi::OsString>, name: &str) -> bool {
    let Some(path) = path else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(name).is_file())
}

/// Load the user's login-shell environment into this process at startup, so
/// commands find tools the user installed via their shell rc (nvm/pyenv/conda,
/// Homebrew, npm-global) — not just the minimal PATH a GUI launched from the
/// Finder/dock inherits. Mirrors what VS Code and similar tools do.
///
/// Runs `$SHELL -l -i -c` once (login + interactive) to dump `env` between
/// markers. Interactive mode is REQUIRED on macOS: zsh users conventionally
/// put version-manager PATH entries (nvm, pyenv, rbenv) in `.zshrc`, not
/// `.zprofile`, so `-l` alone silently loses those tools when the agent is
/// launched from the Finder/dock. Running user rc code is accepted — the
/// login profile files (`.bash_profile` / `.zprofile`) execute either way, so
/// omitting `-i` buys no real isolation, only missing PATH entries. `BASH_ENV`
/// is still removed so non-interactive bash doesn't source an extra file.
///
/// RC noise on stderr is discarded; a 5s timeout guards against a hanging
/// rc. PATH is always taken from the login shell; other vars are merged only
/// when absent, so intentional launcher overrides are never clobbered.
/// No-op on Windows, where GUI processes already inherit the full registry PATH.
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
        .env_remove("BASH_ENV")
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

    // stdout is Stdio::piped() above, so the take always succeeds.
    let mut stdout = child.stdout.take().expect("stdout piped above");
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

    let (path, merged) = plan_env_merge(&dump, marker, &|key| std::env::var_os(key).is_some());
    let merged_count = merged.len();
    for (key, value) in &merged {
        std::env::set_var(key, value);
    }
    if let Some(value) = path {
        std::env::set_var("PATH", value);
        tracing::info!("hydrated PATH from login shell ({shell}); merged {merged_count} env vars");
    }
}

/// Plan the env merge from a login-shell dump: the PATH value (always
/// applied) plus the vars absent from the current process env (additive
/// only — never overwrite a var the launcher set on purpose). Pure so the
/// merge rules are testable without mutating the process environment.
///
/// Content strictly between the two markers is the env dump (rc scripts may
/// print before the first marker; we ignore that).
#[cfg(not(target_os = "windows"))]
fn plan_env_merge<'a>(
    dump: &'a str,
    marker: &str,
    has_var: &dyn Fn(&str) -> bool,
) -> (Option<&'a str>, Vec<(&'a str, &'a str)>) {
    let Some(start) = dump.find(marker) else {
        return (None, vec![]);
    };
    let after = &dump[start + marker.len()..];
    let body = after.split(marker).next().unwrap_or("");

    let mut path = None;
    let mut merged = vec![];
    for line in body.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.is_empty() {
            continue;
        }
        if key == "PATH" {
            path = Some(value);
        } else if !has_var(key) {
            merged.push((key, value));
        }
    }
    (path, merged)
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
    platform_sandbox_availability().unwrap_or(false)
}

fn platform_backend_receipt() -> SandboxBackendReceipt {
    #[cfg(target_os = "macos")]
    {
        if Path::new("/usr/bin/sandbox-exec").exists() {
            return SandboxBackendReceipt::MacosSeatbelt {
                executable: PathBuf::from("/usr/bin/sandbox-exec"),
            };
        }
    }
    #[cfg(target_os = "windows")]
    {
        if cached_windows_sandbox_probe()
            .map(|probe| probe.available)
            .unwrap_or(false)
        {
            return SandboxBackendReceipt::WindowsRestricted;
        }
    }
    #[cfg(target_os = "linux")]
    {
        let probe = linux::probe::probe_linux_sandbox_host();
        if probe.available {
            return SandboxBackendReceipt::LinuxBubblewrap { probe };
        }
    }
    SandboxBackendReceipt::Unavailable
}

/// Stable, product-facing sandbox diagnostic shared by RPC, CLI doctor, and
/// execution availability. Optional fields are omitted on backends where they
/// do not apply; `diagnostic` details never cross this boundary.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxProbeResult {
    pub available: bool,
    pub code: String,
    pub backend: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<linux::probe::LinuxSandboxCapabilities>,
}

/// Resolve product sandbox support without collapsing a transient Windows
/// probe failure into an authoritative unsupported result.
#[allow(clippy::needless_return)] // Each target compiles a different cfg branch.
pub fn platform_sandbox_probe_product() -> std::io::Result<SandboxProbeResult> {
    #[cfg(target_os = "macos")]
    {
        let path = PathBuf::from("/usr/bin/sandbox-exec");
        let available = path.exists();
        Ok(SandboxProbeResult {
            available,
            code: if available {
                "available"
            } else {
                "binary_missing"
            }
            .to_string(),
            backend: "macos_seatbelt".to_string(),
            path: available.then_some(path),
            version: None,
            capabilities: None,
        })
    }
    #[cfg(target_os = "windows")]
    {
        let probe = probe_windows_sandbox_product()?;
        Ok(SandboxProbeResult {
            available: probe.available,
            code: probe.code.to_string(),
            backend: "windows_restricted".to_string(),
            path: None,
            version: None,
            capabilities: None,
        })
    }
    #[cfg(target_os = "linux")]
    {
        let probe = linux::probe::probe_linux_sandbox_host();
        Ok(SandboxProbeResult {
            available: probe.available,
            code: serde_json::to_value(probe.code)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_else(|| "probe_failed".to_string()),
            backend: "linux_bubblewrap".to_string(),
            path: probe.path,
            version: probe.version,
            capabilities: probe.capabilities,
        })
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Ok(SandboxProbeResult {
            available: false,
            code: "platform_unsupported".to_string(),
            backend: "none".to_string(),
            path: None,
            version: None,
            capabilities: None,
        })
    }
}

pub(crate) fn platform_sandbox_availability() -> std::io::Result<bool> {
    platform_sandbox_probe_product().map(|probe| probe.available)
}

#[cfg(target_os = "windows")]
fn cached_windows_sandbox_probe() -> std::io::Result<&'static WindowsSandboxProbe> {
    if let Some(result) = WINDOWS_SANDBOX_PROBE.get() {
        return Ok(result);
    }

    let result = probe_windows_sandbox_host()?;
    if let Some(diagnostic) = result.diagnostic() {
        // Initialization and cleanup failures can be transient (for example,
        // while another process is releasing a capability lock). Do not turn
        // them into a process-lifetime unsupported verdict.
        tracing::warn!(
            code = result.code,
            error = diagnostic,
            "Windows sandbox host probe failed"
        );
        return Err(std::io::Error::other(format!(
            "Windows sandbox probe failed [{}]",
            result.code
        )));
    }
    tracing::info!(
        available = result.available,
        code = result.code,
        "Windows sandbox host probe completed"
    );
    let _ = WINDOWS_SANDBOX_PROBE.set(result);
    WINDOWS_SANDBOX_PROBE
        .get()
        .ok_or_else(|| std::io::Error::other("Windows sandbox probe cache was not initialized"))
}

#[cfg(target_os = "windows")]
static WINDOWS_SANDBOX_PROBE: std::sync::OnceLock<WindowsSandboxProbe> = std::sync::OnceLock::new();

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowsSandboxProbe {
    pub available: bool,
    pub code: &'static str,
    #[serde(skip)]
    diagnostic: Option<String>,
}

impl WindowsSandboxProbe {
    #[cfg(any(target_os = "windows", test))]
    fn available() -> Self {
        Self {
            available: true,
            code: "available",
            diagnostic: None,
        }
    }

    fn unavailable_without_error(code: &'static str) -> Self {
        Self {
            available: false,
            code,
            diagnostic: None,
        }
    }

    #[cfg(any(target_os = "windows", test))]
    fn unavailable(code: &'static str, error: impl std::fmt::Display) -> Self {
        Self {
            available: false,
            code,
            diagnostic: Some(error.to_string()),
        }
    }

    pub(crate) fn diagnostic(&self) -> Option<&str> {
        self.diagnostic.as_deref()
    }
}

pub(crate) fn probe_windows_sandbox_host() -> std::io::Result<WindowsSandboxProbe> {
    #[cfg(target_os = "windows")]
    {
        windows::runner::probe_host()
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(WindowsSandboxProbe::unavailable_without_error(
            "platform_not_windows",
        ))
    }
}

/// Product-facing probe. UI availability and command execution share one
/// cached result so a transient second probe can never make the UI promise
/// protection that the session will not apply.
pub(crate) fn probe_windows_sandbox_product() -> std::io::Result<WindowsSandboxProbe> {
    #[cfg(target_os = "windows")]
    {
        Ok(cached_windows_sandbox_probe()?.clone())
    }
    #[cfg(not(target_os = "windows"))]
    probe_windows_sandbox_host()
}

pub(crate) fn reset_windows_sandbox_capabilities() -> std::io::Result<usize> {
    #[cfg(target_os = "windows")]
    {
        windows::runner::reset_capabilities()
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Windows sandbox reset is only available on Windows",
        ))
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
pub fn sandbox_violation(
    sandbox: &ResolvedSandbox,
    exit_code: i32,
    output: &str,
) -> Option<linux::violation::LinuxSandboxViolation> {
    if matches!(
        sandbox.backend_receipt,
        SandboxBackendReceipt::LinuxBubblewrap { .. }
    ) {
        let digest = linux::plan::policy_digest(&sandbox.rules.snapshot()).unwrap_or_default();
        return linux::violation::classify(exit_code, output, &digest);
    }
    None
}

pub fn looks_like_sandbox_denial(sandbox: &ResolvedSandbox, exit_code: i32, stderr: &str) -> bool {
    if exit_code == 0 {
        return false;
    }
    if matches!(
        sandbox.backend_receipt,
        SandboxBackendReceipt::LinuxBubblewrap { .. }
    ) {
        return sandbox_violation(sandbox, exit_code, stderr).is_some();
    }
    stderr.contains("Operation not permitted") || stderr.contains("sandbox-exec")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_backend_explicitly_falls_back_to_manual() {
        assert_eq!(
            effective_tier(SandboxTier::Sandbox, &SandboxBackendReceipt::Unavailable),
            SandboxTier::Manual
        );
        assert_eq!(
            effective_tier(SandboxTier::Off, &SandboxBackendReceipt::Unavailable),
            SandboxTier::Off
        );
    }

    #[test]
    fn windows_probe_response_hides_internal_diagnostics() {
        let result = WindowsSandboxProbe::unavailable("backend_initialization_failed", "secret");
        let value = serde_json::to_value(result).unwrap();
        assert_eq!(value["available"], false);
        assert_eq!(value["code"], "backend_initialization_failed");
        assert!(value.get("diagnostic").is_none());
        assert_eq!(
            serde_json::to_value(WindowsSandboxProbe::available()).unwrap()["code"],
            "available"
        );
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn classifies_legacy_bash_probe_exit_codes() {
        assert_eq!(legacy_bash_from_exit_code(Some(0)), Some(true));
        assert_eq!(legacy_bash_from_exit_code(Some(1)), Some(false));
        assert_eq!(legacy_bash_from_exit_code(Some(2)), None);
        assert_eq!(legacy_bash_from_exit_code(None), None);
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn legacy_bash_probe_short_circuits_non_bash_shells() {
        // Name gate: non-bash shells never spawn the version probe.
        assert!(!legacy_bash_probe("/bin/sh"));
        assert!(!legacy_bash_probe("zsh"));
        assert!(!legacy_bash_probe("/usr/local/bin/fish"));
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn legacy_bash_probe_runs_real_bash_version_check() {
        // "bash" resolves via PATH; the probe spawns the real binary which
        // exits promptly with a classified status. The boolean outcome
        // depends on the host bash version — either way the probe arm ran.
        let _ = legacy_bash_probe("bash");
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn on_path_scans_the_process_path() {
        assert!(on_path("sh"));
        assert!(!on_path("definitely-not-a-real-binary-xyz"));
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn legacy_bash_probe_times_out_and_kills_child() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let shell = dir.path().join("bash");
        std::fs::write(&shell, "#!/bin/sh\nexec sleep 5\n").unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();

        let started = std::time::Instant::now();
        assert_eq!(
            probe_legacy_bash(
                &shell.to_string_lossy(),
                std::time::Duration::from_millis(25)
            ),
            None
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

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
        manual.backend_receipt = SandboxBackendReceipt::MacosSeatbelt {
            executable: PathBuf::from("/usr/bin/sandbox-exec"),
        };
        // Manual: shell needs approval, never OS-wrapped, even where available.
        assert!(!manual.wraps_shell());
        assert!(manual.shell_needs_approval());

        let mut sandbox = ResolvedSandbox::resolve(
            &SandboxPolicy {
                tier: SandboxTier::Sandbox,
            },
            &ws,
        );
        sandbox.backend_receipt = SandboxBackendReceipt::MacosSeatbelt {
            executable: PathBuf::from("/usr/bin/sandbox-exec"),
        };
        sandbox.tier = SandboxTier::Sandbox;
        assert!(sandbox.wraps_shell());
        assert!(!sandbox.shell_needs_approval());
        // Sandbox tier without the OS sandbox falls back to shell approval.
        sandbox.backend_receipt = SandboxBackendReceipt::Unavailable;
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
        // Reuse the static UTF-8 encoding rather than constructing a .NET type,
        // which is forbidden by Windows PowerShell Constrained Language Mode.
        assert!(script.contains("[System.Text.Encoding]::UTF8"));
        // Each assignment is isolated so a CLM-blocked one cannot break the
        // others (`[Console]::OutputEncoding` is blocked under CLM).
        assert!(script.contains("$OutputEncoding = [System.Text.Encoding]::UTF8"));
        assert!(script.contains("[Console]::OutputEncoding = [System.Text.Encoding]::UTF8"));
        assert!(script.contains("$futureosInitialErrorCount = $Error.Count"));
        assert!(script.contains("$Error.Count -gt $futureosInitialErrorCount"));
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
    #[cfg(target_os = "windows")]
    fn decode_oem_lossy_round_trips_ascii_and_cjk() {
        use windows_sys::Win32::Globalization::GetOEMCP;

        // ASCII is identical in every OEM code page.
        assert_eq!(decode_oem_lossy(b"hello"), "hello");
        assert_eq!(decode_oem_lossy(b""), "");
        // GBK (code page 936) encodes 中文 as D6 D0 CE C4. Only assert on
        // systems whose OEM code page is actually GBK; elsewhere the same
        // bytes decode differently (e.g. cp437) and this branch is skipped.
        if unsafe { GetOEMCP() } == 936 {
            assert_eq!(decode_oem_lossy(&[0xD6, 0xD0, 0xCE, 0xC4]), "中文");
        }
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

    // ─── SandboxTier ────────────────────────────────────────────────────────

    #[test]
    fn sandbox_tier_parse() {
        assert_eq!(SandboxTier::parse("off"), SandboxTier::Off);
        assert_eq!(SandboxTier::parse("sandbox"), SandboxTier::Sandbox);
        assert_eq!(SandboxTier::parse("manual"), SandboxTier::Manual);
        assert_eq!(SandboxTier::parse("unknown"), SandboxTier::Manual);
    }

    #[test]
    fn sandbox_tier_as_str() {
        assert_eq!(SandboxTier::Off.as_str(), "off");
        assert_eq!(SandboxTier::Manual.as_str(), "manual");
        assert_eq!(SandboxTier::Sandbox.as_str(), "sandbox");
    }

    #[test]
    fn sandbox_tier_default_is_manual() {
        assert_eq!(SandboxTier::default(), SandboxTier::Manual);
    }

    // ─── SandboxPolicy ─────────────────────────────────────────────────────

    #[test]
    fn sandbox_policy_default() {
        let policy = SandboxPolicy::default();
        assert_eq!(policy.tier, SandboxTier::Manual);
    }

    // ─── ResolvedSandbox::resolve_with_session ─────────────────────────────

    #[test]
    fn resolve_with_session_shares_rules() {
        let ws = temp_workspace("with-session");
        let session = rules::SessionRules::default();
        let s = ResolvedSandbox::resolve_with_session(
            &SandboxPolicy {
                tier: SandboxTier::Manual,
            },
            &ws,
            session,
        );
        assert_eq!(s.tier, SandboxTier::Manual);
        // workspace is canonicalized (macOS /var → /private/var)
        assert!(s.workspace.to_string_lossy().contains("with-session"));
    }

    // ─── boundary_json ─────────────────────────────────────────────────────

    #[test]
    fn boundary_json_fields() {
        let ws = temp_workspace("boundary");
        let s = enabled(&ws);
        let json = s.boundary_json(Some("violation"), false);
        assert_eq!(json["violation"], "violation");
        assert_eq!(json["inside_sandbox"], false);
        assert_eq!(json["tier"], "manual");
        assert!(json["cwd"].is_string());
    }

    #[test]
    fn boundary_json_no_violation() {
        let ws = temp_workspace("boundary2");
        let s = enabled(&ws);
        let json = s.boundary_json(None, true);
        assert!(json["violation"].is_null());
        assert_eq!(json["inside_sandbox"], true);
    }

    // ─── rule_set ──────────────────────────────────────────────────────────

    #[test]
    fn rule_set_returns_reference() {
        let ws = temp_workspace("ruleset");
        let s = enabled(&ws);
        let _rs = s.rule_set();
        // Just verify it doesn't panic and returns a reference
    }

    // ─── is_secret_path ────────────────────────────────────────────────────

    #[test]
    fn is_secret_path_with_workspace_files() {
        let ws = temp_workspace("secret");
        let s = enabled(&ws);
        // .env inside workspace is a built-in secret
        assert!(s.is_secret_path(Path::new(&format!("{ws}/.env"))));
        assert!(s.is_secret_path(Path::new(&format!("{ws}/.env.local"))));
        // Regular file is not
        assert!(!s.is_secret_path(Path::new(&format!("{ws}/public.txt"))));
    }

    // ─── write_allowed ─────────────────────────────────────────────────────

    #[test]
    fn write_allowed_inside_workspace() {
        let ws = temp_workspace("write-allowed");
        let s = enabled(&ws);
        assert!(s.write_allowed("test.txt"));
        assert!(s.write_allowed("subdir/test.txt"));
    }

    // ─── build_shell_command (non-escalated, non-sandboxed) ────────────────

    #[test]
    fn build_shell_command_off_tier() {
        let ws = temp_workspace("build-cmd");
        let s = ResolvedSandbox::disabled(&ws);
        let cmd = s.build_shell_command("echo hello", false);
        let std_cmd = cmd.as_std();
        let program = std_cmd.get_program().to_string_lossy().to_string();
        let name = program.rsplit('/').next().unwrap_or(&program);
        assert!(
            matches!(name, "bash" | "zsh" | "sh" | "pwsh" | "powershell"),
            "unexpected shell: {program}"
        );
    }

    #[test]
    fn build_shell_command_escalated_always_uses_shell() {
        let ws = temp_workspace("esc-cmd");
        let mut s = ResolvedSandbox::resolve(
            &SandboxPolicy {
                tier: SandboxTier::Sandbox,
            },
            &ws,
        );
        s.backend_receipt = SandboxBackendReceipt::MacosSeatbelt {
            executable: PathBuf::from("/usr/bin/sandbox-exec"),
        };
        // Escalated should skip the OS sandbox
        let cmd = s.build_shell_command("echo escalated", true);
        let std_cmd = cmd.as_std();
        let program = std_cmd.get_program().to_string_lossy().to_string();
        let name = program.rsplit('/').next().unwrap_or(&program);
        assert!(
            matches!(name, "bash" | "zsh" | "sh" | "pwsh" | "powershell"),
            "escalated should use platform shell: {program}"
        );
    }

    // ─── shell_display_name / shell_supports_chain_operators ───────────────

    #[test]
    fn shell_display_name_non_empty() {
        let name = shell_display_name();
        assert!(!name.is_empty());
    }

    #[test]
    fn shell_supports_chain_operators_returns_bool() {
        let _ = shell_supports_chain_operators();
    }

    // ─── evaluate with enabled sandbox ─────────────────────────────────────

    #[test]
    fn evaluate_workspace_read_allowed() {
        let ws = temp_workspace("eval-read");
        let s = enabled(&ws);
        assert_eq!(
            s.evaluate(Path::new(&format!("{ws}/file.txt")), Op::Read),
            Decision::Allow
        );
    }

    #[test]
    fn evaluate_workspace_write_allowed() {
        let ws = temp_workspace("eval-write");
        let s = enabled(&ws);
        assert_eq!(
            s.evaluate(Path::new(&format!("{ws}/file.txt")), Op::Write),
            Decision::Allow
        );
    }

    #[test]
    fn evaluate_env_in_workspace_asks() {
        let ws = temp_workspace("eval-env");
        let s = enabled(&ws);
        // .env inside workspace is a built-in secret (asks even in-workspace)
        assert_eq!(
            s.evaluate(Path::new(&format!("{ws}/.env")), Op::Write),
            Decision::Ask
        );
    }

    // ─── ResolvedSandbox::default ──────────────────────────────────────────

    #[test]
    fn resolved_sandbox_default_is_disabled() {
        let s = ResolvedSandbox::default();
        assert_eq!(s.tier, SandboxTier::Off);
        assert!(!s.enabled());
    }

    // ─── coverage batch: small public-surface arms ─────────────────────────

    #[test]
    fn add_session_allow_maps_read_op() {
        let ws = temp_workspace("allow-read");
        let s = enabled(&ws);
        let target = dirs::home_dir().unwrap().join("futureos-read-allow.txt");
        // Write stays gated; a session read-allow opens reads only.
        s.add_session_allow(&target.to_string_lossy(), Op::Read);
        assert_eq!(s.evaluate(&target, Op::Read), Decision::Allow);
        assert_eq!(s.evaluate(&target, Op::Write), Decision::Ask);
    }

    #[test]
    fn sandbox_denial_heuristic_variants() {
        let ws = temp_workspace("denial-heur");
        let mut s = enabled(&ws);
        s.set_backend_available_for_test(true);
        assert!(!looks_like_sandbox_denial(&s, 0, "Operation not permitted"));
        assert!(looks_like_sandbox_denial(
            &s,
            1,
            "touch: /x: Operation not permitted"
        ));
        assert!(looks_like_sandbox_denial(
            &s,
            1,
            "sandbox-exec: deny(1) file-write"
        ));
        assert!(!looks_like_sandbox_denial(&s, 1, "file not found"));
    }

    #[test]
    fn linux_denial_classification_excludes_shell_and_infrastructure_failures() {
        let ws = temp_workspace("linux-denial-heur");
        let mut sandbox = enabled(&ws);
        sandbox.set_linux_backend_available_for_test();
        assert!(looks_like_sandbox_denial(
            &sandbox,
            1,
            "touch: Permission denied"
        ));
        assert!(looks_like_sandbox_denial(
            &sandbox,
            1,
            "write: Read-only file system"
        ));
        for code in [2, 125, 126, 127] {
            assert!(!looks_like_sandbox_denial(
                &sandbox,
                code,
                "Permission denied"
            ));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn seatbelt_profile_is_generated_for_sandbox_tier() {
        let ws = temp_workspace("seatbelt-profile");
        let s = ResolvedSandbox::resolve(
            &SandboxPolicy {
                tier: SandboxTier::Sandbox,
            },
            &ws,
        );
        let profile = seatbelt_profile(&s);
        assert!(profile.contains("deny default"));
    }

    // ─── unix shell resolution / login-shell hydration ─────────────────────

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn resolve_unix_shell_prefers_usable_shell_env() {
        // /bin/sh exists everywhere but is not bash/zsh → probe order wins.
        let sh = std::ffi::OsString::from("/bin/sh");
        assert_eq!(resolve_unix_shell(Some(sh), &|name| name == "zsh"), "zsh");
        // A bash/zsh $SHELL that resolves to a real file is used verbatim.
        let bash = std::ffi::OsString::from("/bin/bash");
        assert_eq!(resolve_unix_shell(Some(bash), &|_| false), "/bin/bash");
        // Nothing usable anywhere → POSIX sh.
        assert_eq!(resolve_unix_shell(None, &|_| false), "sh");
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn on_path_without_path_env_is_false() {
        assert!(!on_path_in(None, "bash"));
        assert!(on_path_in(Some(std::ffi::OsString::from("/bin")), "sh"));
        assert!(!on_path_in(
            Some(std::ffi::OsString::from("/bin")),
            "future-nope"
        ));
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn plan_env_merge_parses_dump_between_markers() {
        let marker = "__m__";
        // Noise lines, an empty key, and a no-`=` line are all skipped; PATH
        // is captured; an already-present var is not merged.
        let dump = format!("rc noise\n{marker}\nPATH=/a:/b\n=noval\nnoequals\nNEW_VAR=1\nEXISTING=2\n{marker}\ntrailing");
        let (path, merged) = plan_env_merge(&dump, marker, &|key| key == "EXISTING");
        assert_eq!(path, Some("/a:/b"));
        assert_eq!(merged, vec![("NEW_VAR", "1")]);
        // No marker at all → nothing to apply.
        assert_eq!(
            plan_env_merge("no markers here", marker, &|_| false),
            (None, vec![])
        );
    }

    /// Serialises the hydrate tests: they mutate the process-wide $SHELL.
    #[cfg(not(target_os = "windows"))]
    static HYDRATE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[cfg(not(target_os = "windows"))]
    fn hydrate_test_lock() -> std::sync::MutexGuard<'static, ()> {
        HYDRATE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    #[cfg(not(target_os = "windows"))]
    fn fake_shell(body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let shell = dir.path().join("fakesh");
        std::fs::write(&shell, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        (dir, shell)
    }

    #[cfg(not(target_os = "windows"))]
    struct ShellEnvGuard(Option<std::ffi::OsString>);

    #[cfg(not(target_os = "windows"))]
    impl ShellEnvGuard {
        fn set(path: &std::path::Path) -> Self {
            let previous = std::env::var_os("SHELL");
            std::env::set_var("SHELL", path);
            Self(previous)
        }
    }

    #[cfg(not(target_os = "windows"))]
    impl Drop for ShellEnvGuard {
        fn drop(&mut self) {
            crate::test_support::restore_env("SHELL", &self.0);
        }
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn hydrate_skips_when_shell_spawn_fails() {
        let _lock = hydrate_test_lock();
        let _subscriber = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_writer(std::io::sink)
                .finish(),
        );
        // No pre-existing SHELL → the guard's remove-on-drop arm runs.
        std::env::remove_var("SHELL");
        let _guard = ShellEnvGuard::set(std::path::Path::new("/future-no-such-shell-xyz"));
        hydrate_from_login_shell(); // must return, not panic
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn hydrate_ignores_dump_without_marker() {
        let _lock = hydrate_test_lock();
        // Pre-set SHELL so the guard's restore arm with a previous value runs.
        let original = std::env::var_os("SHELL");
        std::env::set_var("SHELL", "/bin/sh");
        let (_dir, shell) = fake_shell("printf 'no markers in this output'");
        {
            let _guard = ShellEnvGuard::set(&shell);
            hydrate_from_login_shell(); // dump lacks the marker → nothing applied
        }
        // Leave the process env as we found it (parallel suites read $SHELL).
        crate::test_support::restore_env("SHELL", &original);
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn hydrate_times_out_and_kills_hung_shell() {
        let _lock = hydrate_test_lock();
        let _subscriber = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_writer(std::io::sink)
                .finish(),
        );
        let (_dir, shell) = fake_shell("exec sleep 30");
        let _guard = ShellEnvGuard::set(&shell);
        hydrate_from_login_shell(); // 5s timeout → kill → return
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn hydrate_merges_env_from_login_shell_dump() {
        let _lock = hydrate_test_lock();
        let _subscriber = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_writer(std::io::sink)
                .finish(),
        );
        // PATH is re-set to its CURRENT value (no effective change, safe for
        // parallel tests); the unique var must be absent beforehand to merge.
        std::env::remove_var("FUTURE_HYDRATE_MERGE_TEST");
        let current_path = std::env::var("PATH").unwrap_or_default();
        let (_dir, shell) = fake_shell(&format!(
            "printf '%s' '__future_env_boundary_9c4f__'; printf 'PATH={current_path}\\nFUTURE_HYDRATE_MERGE_TEST=merged\\n'; printf '%s' '__future_env_boundary_9c4f__'"
        ));
        let _guard = ShellEnvGuard::set(&shell);
        hydrate_from_login_shell();
        assert_eq!(
            std::env::var("FUTURE_HYDRATE_MERGE_TEST").as_deref(),
            Ok("merged")
        );
        assert_eq!(std::env::var("PATH").unwrap_or_default(), current_path);
        std::env::remove_var("FUTURE_HYDRATE_MERGE_TEST");
    }
}
