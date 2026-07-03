//! OS-level sandbox for spawned bash commands (SANDBOX_PLAN.md).
//!
//! The sandbox defines the technical boundary (where commands may write,
//! whether they may use the network); the approval flow decides when to stop
//! and ask the user before crossing it. Commands that stay inside the
//! boundary run autonomously without approval prompts.
//!
//! Platform support: macOS via Seatbelt (`/usr/bin/sandbox-exec`). Other
//! platforms currently degrade to the legacy allowlist + approval behavior
//! (`available == false`).

pub mod paths;
mod seatbelt;

use std::path::{Path, PathBuf};
use std::sync::Arc;

// ─── Policy (wire-level, set via gRPC set_sandbox_policy) ──────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxMode {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

impl SandboxMode {
    pub fn parse(value: &str) -> Self {
        match value {
            "read-only" => Self::ReadOnly,
            "danger-full-access" => Self::DangerFullAccess,
            _ => Self::WorkspaceWrite,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
            Self::DangerFullAccess => "danger-full-access",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalPolicy {
    Untrusted,
    OnRequest,
    Never,
}

impl ApprovalPolicy {
    pub fn parse(value: &str) -> Self {
        match value {
            "untrusted" => Self::Untrusted,
            "never" => Self::Never,
            _ => Self::OnRequest,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Untrusted => "untrusted",
            Self::OnRequest => "on-request",
            Self::Never => "never",
        }
    }
}

/// One allow/deny rule (Phase 2 evaluates these; Phase 1 only carries them).
#[derive(Debug, Clone)]
pub struct SandboxRule {
    pub match_kind: String,  // "command_prefix" | "path_glob"
    pub match_value: String, // wildcard pattern
    pub decision: String,    // "approve" | "reject"
}

/// Session sandbox policy as received from the frontend (or defaults).
#[derive(Debug, Clone)]
pub struct SandboxPolicy {
    pub mode: SandboxMode,
    /// Extra writable roots beyond the workspace and temp dirs.
    pub writable_roots: Vec<String>,
    pub network_access: bool,
    pub approval_policy: ApprovalPolicy,
    pub rules: Vec<SandboxRule>,
}

impl Default for SandboxPolicy {
    fn default() -> Self {
        Self {
            mode: SandboxMode::WorkspaceWrite,
            writable_roots: vec![],
            network_access: false,
            approval_policy: ApprovalPolicy::OnRequest,
            rules: vec![],
        }
    }
}

// ─── Resolved per-run settings ──────────────────────────────────────────────

/// Sandbox policy resolved against a concrete workspace: canonicalized
/// writable roots, platform availability, ready to wrap commands and to
/// answer boundary questions consistently with what the OS enforces.
#[derive(Debug, Clone)]
pub struct ResolvedSandbox {
    pub mode: SandboxMode,
    pub approval_policy: ApprovalPolicy,
    pub network_access: bool,
    /// Canonicalized workspace directory (always writable_roots[0]).
    pub workspace: PathBuf,
    /// Canonicalized writable roots: workspace, $TMPDIR, /tmp, extras.
    pub writable_roots: Vec<PathBuf>,
    /// Whether the platform sandbox (sandbox-exec) is usable here.
    pub available: bool,
    pub rules: Vec<SandboxRule>,
}

impl ResolvedSandbox {
    pub fn resolve(policy: &SandboxPolicy, workspace: &str) -> Self {
        let workspace = paths::canonicalize_lenient(Path::new(workspace));
        let mut roots = vec![workspace.clone()];
        // Temp dirs are fully open for read/write (most build tools need them).
        // Canonicalize so Seatbelt sees real paths (/tmp -> /private/tmp,
        // $TMPDIR -> /private/var/folders/... on macOS).
        let tmp = paths::canonicalize_lenient(&std::env::temp_dir());
        if !roots.iter().any(|r| r == &tmp) {
            roots.push(tmp);
        }
        #[cfg(unix)]
        {
            let slash_tmp = paths::canonicalize_lenient(Path::new("/tmp"));
            if !roots.iter().any(|r| r == &slash_tmp) {
                roots.push(slash_tmp);
            }
        }
        for extra in &policy.writable_roots {
            let root = paths::canonicalize_lenient(&paths::resolve_against(&workspace, extra));
            if !roots.iter().any(|r| r == &root) {
                roots.push(root);
            }
        }
        Self {
            mode: policy.mode,
            approval_policy: policy.approval_policy,
            network_access: policy.network_access,
            workspace,
            writable_roots: roots,
            available: platform_sandbox_available(),
            rules: policy.rules.clone(),
        }
    }

    /// Whether bash commands will actually run wrapped in the OS sandbox.
    pub fn wraps_bash(&self) -> bool {
        self.available && self.mode != SandboxMode::DangerFullAccess
    }

    /// Whether a candidate path (as given to write/edit) falls inside any
    /// writable root. Uses the §3.5 normalization rules.
    pub fn path_is_writable(&self, candidate: &str) -> bool {
        if self.mode == SandboxMode::DangerFullAccess {
            return true;
        }
        paths::within_any_root(&self.workspace, candidate, &self.writable_roots)
    }

    /// Build the bash invocation for `command`: sandbox-wrapped when the
    /// platform sandbox is available and the mode calls for it, plain
    /// otherwise. `escalated` forces a plain (unsandboxed) invocation for a
    /// single approved run.
    pub fn build_bash_command(&self, command: &str, escalated: bool) -> tokio::process::Command {
        if !escalated && self.wraps_bash() {
            #[cfg(target_os = "macos")]
            {
                return seatbelt::build_command(self, command);
            }
        }
        let mut child = tokio::process::Command::new("bash");
        child.args(["-c", command]);
        child
    }

    /// Structured sandbox_boundary payload for approval events — real values,
    /// not placeholders.
    pub fn boundary_json(
        &self,
        violation: Option<&str>,
        inside_sandbox: bool,
    ) -> serde_json::Value {
        serde_json::json!({
            "mode": self.mode.as_str(),
            "inside_sandbox": inside_sandbox,
            "sandbox_available": self.available,
            "violation": violation,
            "cwd": self.workspace.to_string_lossy(),
            "writable_roots": self.writable_roots
                .iter()
                .map(|r| r.to_string_lossy().to_string())
                .collect::<Vec<_>>(),
        })
    }
}

impl Default for ResolvedSandbox {
    /// Degraded/legacy default: no OS sandbox, workspace-write semantics with
    /// only the workspace boundary. Used when no policy has been set and the
    /// tool scope has no sandbox context (e.g. bare unit tests).
    fn default() -> Self {
        ResolvedSandbox::resolve(
            &SandboxPolicy::default(),
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

/// A request to re-run (or run) a command outside the sandbox, raised from
/// inside the bash tool after a sandbox denial or when the model explicitly
/// asks for escalated permissions.
#[derive(Debug, Clone)]
pub struct EscalationRequest {
    pub command: String,
    /// Model-provided reason (empty for heuristic-triggered escalations).
    pub justification: String,
    /// Tail of stderr from the failed sandboxed run (empty when the model
    /// requested escalation up front).
    pub failure_summary: String,
}

#[derive(Debug, Clone)]
pub enum EscalationDecision {
    Approved,
    Denied(String),
}

/// Callback the RPC layer injects into the tool scope so `run_bash` can raise
/// a `sandbox_escalation` approval without touching RPC/UI internals. The
/// closure blocks until the user decides (same semantics as ApprovalGate).
pub type EscalationRequester = Arc<dyn Fn(&EscalationRequest) -> EscalationDecision + Send + Sync>;

// ─── Sandbox-denial heuristic ───────────────────────────────────────────────

/// Conservative check: does this failed sandboxed run look like it was the
/// *sandbox* that stopped it (as opposed to an ordinary command failure)?
///
/// False negatives are fine — the model sees the raw error and can retry with
/// `escalated: true`. False positives are not acceptable (they would nag the
/// user with escalation prompts for ordinary failures), so match narrowly.
pub fn looks_like_sandbox_denial(sandbox: &ResolvedSandbox, exit_code: i32, stderr: &str) -> bool {
    if exit_code == 0 {
        return false;
    }
    // Seatbelt EPERM surfaces as "Operation not permitted" from most tools.
    if stderr.contains("Operation not permitted") || stderr.contains("sandbox-exec") {
        return true;
    }
    // Network-shaped failures are only attributed to the sandbox when the
    // sandbox is actually blocking the network.
    if !sandbox.network_access {
        const NETWORK_MARKERS: &[&str] = &[
            "Could not resolve host",
            "nodename nor servname provided",
            "Temporary failure in name resolution",
            "Network is unreachable",
            "getaddrinfo",
        ];
        if NETWORK_MARKERS.iter().any(|marker| stderr.contains(marker)) {
            return true;
        }
    }
    false
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

    #[test]
    fn resolve_includes_workspace_and_tmp_roots() {
        let ws = temp_workspace("resolve");
        let resolved = ResolvedSandbox::resolve(&SandboxPolicy::default(), &ws);
        assert!(resolved.writable_roots.len() >= 2);
        assert_eq!(resolved.workspace, resolved.writable_roots[0]);
        // Workspace itself and paths under tmp are writable.
        assert!(resolved.path_is_writable("file.txt"));
        let tmp_file = std::env::temp_dir().join("anything.txt");
        assert!(resolved.path_is_writable(tmp_file.to_string_lossy().as_ref()));
    }

    #[test]
    fn extra_writable_roots_are_honored() {
        let ws = temp_workspace("extra-ws");
        let extra = temp_workspace("extra-root");
        let policy = SandboxPolicy {
            writable_roots: vec![extra.clone()],
            ..Default::default()
        };
        let resolved = ResolvedSandbox::resolve(&policy, &ws);
        assert!(resolved.path_is_writable(&format!("{extra}/x.txt")));
        assert!(!resolved.path_is_writable("/etc/hosts"));
    }

    #[test]
    fn tilde_paths_resolve_to_home_not_workspace() {
        let ws = temp_workspace("tilde");
        let resolved = ResolvedSandbox::resolve(&SandboxPolicy::default(), &ws);
        // ~/somefile is outside the workspace roots (unless home is in tmp).
        assert!(!resolved.path_is_writable("~/somefile-outside.txt"));
    }

    #[test]
    fn full_access_mode_allows_everything_and_never_wraps() {
        let ws = temp_workspace("full");
        let policy = SandboxPolicy {
            mode: SandboxMode::DangerFullAccess,
            ..Default::default()
        };
        let resolved = ResolvedSandbox::resolve(&policy, &ws);
        assert!(resolved.path_is_writable("/etc/hosts"));
        assert!(!resolved.wraps_bash());
    }

    #[test]
    fn denial_heuristic_is_conservative() {
        let ws = temp_workspace("heuristic");
        let resolved = ResolvedSandbox::resolve(&SandboxPolicy::default(), &ws);
        // Ordinary failures do not look like sandbox denials.
        assert!(!looks_like_sandbox_denial(
            &resolved,
            1,
            "error[E0308]: mismatched types"
        ));
        assert!(!looks_like_sandbox_denial(
            &resolved,
            0,
            "Operation not permitted"
        ));
        // EPERM does.
        assert!(looks_like_sandbox_denial(
            &resolved,
            1,
            "touch: /etc/x: Operation not permitted"
        ));
        // Network markers count only while the sandbox blocks the network.
        assert!(looks_like_sandbox_denial(
            &resolved,
            6,
            "curl: (6) Could not resolve host: example.com"
        ));
        let open_net = ResolvedSandbox::resolve(
            &SandboxPolicy {
                network_access: true,
                ..Default::default()
            },
            &ws,
        );
        assert!(!looks_like_sandbox_denial(
            &open_net,
            6,
            "curl: (6) Could not resolve host: example.com"
        ));
    }

    #[test]
    fn mode_and_policy_parse_roundtrip() {
        assert_eq!(SandboxMode::parse("read-only"), SandboxMode::ReadOnly);
        assert_eq!(SandboxMode::parse(""), SandboxMode::WorkspaceWrite);
        assert_eq!(
            SandboxMode::parse("danger-full-access").as_str(),
            "danger-full-access"
        );
        assert_eq!(ApprovalPolicy::parse("never"), ApprovalPolicy::Never);
        assert_eq!(ApprovalPolicy::parse(""), ApprovalPolicy::OnRequest);
    }
}
