//! Platform-independent derivation of the Windows shell-sandbox enforcement plan
//! from the resolved rule set (SANDBOX_PLAN.md §11).
//!
//! The Win32 executor (`windows.rs`, `#[cfg(windows)]`) turns this plan into a
//! restricted token + a set of NTFS ACEs + a job object. Keeping the derivation
//! pure lets it be unit-tested on any platform — the same split as
//! `seatbelt::build_profile` (pure) vs `seatbelt::build_command` (syscalls).
//!
//! Two NTFS limitations shape what lands in the plan (see §11.3/§11.6):
//!   - `WRITE_RESTRICTED` provides a compatible write boundary but does not make
//!     capability-SID deny-read ACEs participate in read access checks. Read
//!     `ask`/`deny` rules are therefore diagnostics, not claimed enforcement.
//!   - NTFS ACLs cannot express path globs. Write rules backed by a glob are
//!     surfaced as structured unsupported entries and remain enforced only by
//!     the in-process tool layer.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::rules::{Access, Decision, MatcherSbpl};
use super::ResolvedSandbox;

/// A path matcher retained for diagnostics when the unelevated backend cannot
/// enforce the corresponding rule.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WindowsRuleMatcher {
    Subtree(PathBuf),
    Regex(String),
}

/// A rule that remains visible to diagnostics but is not enforced by the
/// unelevated shell backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnenforcedWindowsRule {
    pub matcher: WindowsRuleMatcher,
    pub access: Access,
    pub decision: Decision,
}

/// A literal/subtree write denial that can be projected to an NTFS ACE. `Ask`
/// is retained separately from `Deny` because W4 may reopen only approved ask
/// carveouts; deny rules are never approval-overridable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsWriteCarveout {
    pub path: PathBuf,
    pub decision: Decision,
}

/// Pure enforcement plan for one sandboxed shell run.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WindowsSandboxPlan {
    /// Subtrees that receive a write-capability ACE. Ancestor roots subsume
    /// nested roots so the executor receives a deterministic minimal set.
    pub writable_roots: Vec<PathBuf>,
    /// Literal/subtree ask and deny rules compiled as deny-write carveouts.
    pub write_carveouts: Vec<WindowsWriteCarveout>,
    /// Read ask/deny rules the unelevated shell backend cannot enforce.
    pub unenforced_read_rules: Vec<UnenforcedWindowsRule>,
    /// Write rules backed by globs, which NTFS ACLs cannot represent.
    pub unsupported_write_globs: Vec<UnenforcedWindowsRule>,
}

/// Derive the plan from a resolved sandbox's rule set. Reads stay broadly open
/// (delivered by the restricted token's SID set, not by per-path ACEs), so
/// allow-read rules need no ACE and are not collected here.
pub fn build_plan(sandbox: &ResolvedSandbox) -> WindowsSandboxPlan {
    let rules = sandbox.rule_set();
    let mut plan = WindowsSandboxPlan::default();

    // Base writable roots: workspace + temp (mirrors the engine's write
    // fallback and the Seatbelt base).
    plan.writable_roots.push(rules.workspace.clone());
    for tmp in super::rules::temp_roots() {
        plan.writable_roots.push(tmp);
    }

    // Layers arrive highest-priority first. Suppress only an identical matcher
    // already handled by a higher layer; partially-overlapping parent/child
    // rules must remain visible because their effective path sets differ.
    let mut seen_read = HashSet::new();
    let mut seen_write = HashSet::new();
    for layer in rules.profile_layers() {
        for rule in &layer {
            let matcher = matcher(rule.matcher_sbpl());
            let access = rule.access();
            let decision = rule.decision();

            if access.covers_read()
                && seen_read.insert(matcher.clone())
                && matches!(decision, Decision::Ask | Decision::Deny)
            {
                plan.unenforced_read_rules.push(UnenforcedWindowsRule {
                    matcher: matcher.clone(),
                    access: Access::Read,
                    decision,
                });
            }

            if access.covers_write() && seen_write.insert(matcher.clone()) {
                match (&matcher, decision) {
                    (WindowsRuleMatcher::Subtree(path), Decision::Allow) => {
                        plan.writable_roots.push(path.clone());
                    }
                    (WindowsRuleMatcher::Subtree(path), Decision::Ask | Decision::Deny) => {
                        plan.write_carveouts.push(WindowsWriteCarveout {
                            path: path.clone(),
                            decision,
                        });
                    }
                    (WindowsRuleMatcher::Regex(_), _) => {
                        plan.unsupported_write_globs.push(UnenforcedWindowsRule {
                            matcher,
                            access: Access::Write,
                            decision,
                        });
                    }
                }
            }
        }
    }

    minimize_roots(&mut plan.writable_roots);
    plan
}

fn matcher(matcher: MatcherSbpl<'_>) -> WindowsRuleMatcher {
    match matcher {
        MatcherSbpl::Subtree(path) => WindowsRuleMatcher::Subtree(path.to_path_buf()),
        MatcherSbpl::Regex(regex) => WindowsRuleMatcher::Regex(regex.to_owned()),
    }
}

/// Sort roots deterministically and remove exact/nested roots already covered
/// by an ancestor. This keeps ACL work bounded without changing write scope.
fn minimize_roots(paths: &mut Vec<PathBuf>) {
    paths.sort_by(|left, right| {
        left.components()
            .count()
            .cmp(&right.components().count())
            .then_with(|| left.cmp(right))
    });
    let mut roots: Vec<PathBuf> = Vec::with_capacity(paths.len());
    for path in paths.drain(..) {
        if roots.iter().any(|root| path_within(&path, root)) {
            continue;
        }
        roots.push(path);
    }
    *paths = roots;
}

fn path_within(path: &Path, root: &Path) -> bool {
    super::paths::path_within(path, root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::{SandboxPolicy, SandboxTier};

    fn temp_workspace() -> String {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("futureos-winplan-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir.to_string_lossy().to_string()
    }

    fn plan_for(workspace: &str) -> WindowsSandboxPlan {
        let sandbox = ResolvedSandbox::resolve(
            &SandboxPolicy {
                tier: SandboxTier::Manual,
            },
            workspace,
        );
        build_plan(&sandbox)
    }

    fn covers(roots: &[PathBuf], path: &Path) -> bool {
        roots.iter().any(|root| path_within(path, root))
    }

    fn has_carveout(plan: &WindowsSandboxPlan, path: &Path, decision: Decision) -> bool {
        plan.write_carveouts
            .iter()
            .any(|rule| rule.path == path && rule.decision == decision)
    }

    fn has_read_diagnostic(plan: &WindowsSandboxPlan, path: &Path, decision: Decision) -> bool {
        plan.unenforced_read_rules.iter().any(|rule| {
            rule.matcher == WindowsRuleMatcher::Subtree(path.to_path_buf())
                && rule.decision == decision
        })
    }

    #[test]
    fn workspace_and_temp_are_writable() {
        let ws = temp_workspace();
        let plan = plan_for(&ws);
        let workspace = crate::sandbox::paths::canonicalize_lenient(std::path::Path::new(&ws));
        assert!(
            covers(&plan.writable_roots, &workspace),
            "workspace must be covered by a writable root: {plan:?}"
        );
        assert!(
            super::super::rules::temp_roots()
                .iter()
                .all(|root| covers(&plan.writable_roots, root)),
            "temp roots must be covered by writable roots: {plan:?}"
        );
    }

    #[test]
    fn rule_file_is_deny_write_not_deny_read() {
        let ws = temp_workspace();
        let plan = plan_for(&ws);
        let workspace = crate::sandbox::paths::canonicalize_lenient(std::path::Path::new(&ws));
        let rule_file = workspace.join(".future/approval_rule.json");
        assert!(
            has_carveout(&plan, &rule_file, Decision::Deny),
            "rule file write must be denied"
        );
        assert!(
            !has_read_diagnostic(&plan, &rule_file, Decision::Deny),
            "rule file is not a read restriction"
        );
    }

    #[test]
    fn home_ssh_read_is_diagnostic_and_write_is_ask_carveout() {
        let ws = temp_workspace();
        // Capture + canonicalize home ONCE (rule bases are canonicalized
        // during resolution; /var -> /private/var on macOS must not split
        // assertion from guard). Immune to other tests mutating $HOME
        // concurrently (TestHome in rpc::commands).
        let home = crate::sandbox::paths::canonicalize_lenient(&dirs::home_dir().unwrap());
        let rules = crate::sandbox::rules::RuleSet::resolve_isolated_with_home(
            std::path::Path::new(&ws),
            &home,
        );
        let sandbox = ResolvedSandbox {
            tier: SandboxTier::Manual,
            backend_receipt: crate::sandbox::platform_backend_receipt(),
            workspace: rules.workspace.clone(),
            rules,
        };
        let plan = build_plan(&sandbox);
        let ssh = home.join(".ssh");
        assert!(
            has_read_diagnostic(&plan, &ssh, Decision::Ask),
            "~/.ssh read ask must be reported as unenforced"
        );
        assert!(
            has_carveout(&plan, &ssh, Decision::Ask),
            "~/.ssh write must remain an ask carveout"
        );
    }

    #[test]
    fn allow_write_rule_lands_in_writable_allow_read_does_not() {
        use crate::sandbox::rules::{Access, Decision};
        let ws = temp_workspace();
        let home = crate::sandbox::paths::canonicalize_lenient(&dirs::home_dir().unwrap());
        let rules = crate::sandbox::rules::RuleSet::resolve_isolated_with_home(
            std::path::Path::new(&ws),
            &home,
        );
        let allow_write = home.join("futureos-winplan-external-write");
        rules.add_session_rule(
            &allow_write.to_string_lossy(),
            Access::Both,
            Decision::Allow,
        );
        // Read-only allow: broadly open already, so it must NOT add an ACE.
        let allow_read = home.join("futureos-winplan-external-read");
        rules.add_session_rule(&allow_read.to_string_lossy(), Access::Read, Decision::Allow);
        let sandbox = ResolvedSandbox {
            tier: SandboxTier::Manual,
            backend_receipt: crate::sandbox::platform_backend_receipt(),
            workspace: rules.workspace.clone(),
            rules,
        };
        let plan = build_plan(&sandbox);
        assert!(
            covers(&plan.writable_roots, &allow_write),
            "allow-write subtree must get a write ACE: {plan:?}"
        );
        assert!(
            !covers(&plan.writable_roots, &allow_read),
            "allow-read needs no ACE (reads are broadly open): {plan:?}"
        );
    }

    #[test]
    fn literal_env_is_structured_and_globs_are_reported() {
        let ws = temp_workspace();
        let plan = plan_for(&ws);
        let workspace = crate::sandbox::paths::canonicalize_lenient(std::path::Path::new(&ws));
        // Literal `.env` (no glob metachars) → an enforceable deny.
        let env = workspace.join(".env");
        assert!(
            has_read_diagnostic(&plan, &env, Decision::Ask),
            "literal workspace .env read ask must be diagnostic"
        );
        assert!(
            has_carveout(&plan, &env, Decision::Ask),
            "literal workspace .env write ask must be enforceable"
        );
        // Glob workspace secrets (`.env.*`, `**/*.pem`, `**/*.key`, `**/*.p12`,
        // `**/id_rsa*`) cannot be ACE'd → counted, not enforced.
        assert!(
            plan.unsupported_write_globs.len() >= 5,
            "workspace write globs must remain visible: {plan:?}"
        );
        assert!(
            plan.unenforced_read_rules
                .iter()
                .filter(|rule| matches!(rule.matcher, WindowsRuleMatcher::Regex(_)))
                .count()
                >= 5,
            "workspace read globs must remain visible: {plan:?}"
        );
    }

    #[test]
    fn higher_priority_exact_match_suppresses_workspace_rule() {
        let ws = temp_workspace();
        let workspace = crate::sandbox::paths::canonicalize_lenient(Path::new(&ws));
        let future_dir = workspace.join(".future");
        std::fs::create_dir_all(&future_dir).unwrap();
        let external = dirs::home_dir().unwrap().join("futureos-winplan-priority");
        let rule_file = serde_json::json!({
            "rules": [{
                "path": external,
                "access": "write",
                "action": "deny"
            }]
        });
        std::fs::write(
            future_dir.join("approval_rule.json"),
            serde_json::to_vec(&rule_file).unwrap(),
        )
        .unwrap();

        let rules = crate::sandbox::rules::RuleSet::resolve_isolated_with_home(
            &workspace,
            Path::new("/nonexistent-home-for-plan-priority"),
        );
        rules.add_session_rule(&external.to_string_lossy(), Access::Write, Decision::Allow);
        let sandbox = ResolvedSandbox {
            tier: SandboxTier::Manual,
            backend_receipt: crate::sandbox::platform_backend_receipt(),
            workspace: rules.workspace.clone(),
            rules,
        };
        let plan = build_plan(&sandbox);

        assert!(covers(&plan.writable_roots, &external));
        assert!(!has_carveout(&plan, &external, Decision::Deny));
    }

    #[test]
    fn ask_and_deny_write_carveouts_remain_distinguishable() {
        let ws = temp_workspace();
        let workspace = crate::sandbox::paths::canonicalize_lenient(Path::new(&ws));
        let rules = crate::sandbox::rules::RuleSet::resolve_isolated_with_home(
            &workspace,
            Path::new("/nonexistent-home-for-plan-decisions"),
        );
        let ask_path = workspace.join("approval-required");
        let deny_path = workspace.join("never-allow");
        rules.add_session_rule(&ask_path.to_string_lossy(), Access::Write, Decision::Ask);
        rules.add_session_rule(&deny_path.to_string_lossy(), Access::Write, Decision::Deny);
        let sandbox = ResolvedSandbox {
            tier: SandboxTier::Manual,
            backend_receipt: crate::sandbox::platform_backend_receipt(),
            workspace: rules.workspace.clone(),
            rules,
        };
        let plan = build_plan(&sandbox);

        assert!(has_carveout(&plan, &ask_path, Decision::Ask));
        assert!(has_carveout(&plan, &deny_path, Decision::Deny));
    }

    #[test]
    fn writable_roots_are_minimized_by_ancestor() {
        let ws = temp_workspace();
        let workspace = crate::sandbox::paths::canonicalize_lenient(Path::new(&ws));
        let home = crate::sandbox::paths::canonicalize_lenient(&dirs::home_dir().unwrap());
        let rules = crate::sandbox::rules::RuleSet::resolve_isolated_with_home(&workspace, &home);
        let parent = home.join("futureos-winplan-root");
        let child = parent.join("nested");
        rules.add_session_rule(&child.to_string_lossy(), Access::Write, Decision::Allow);
        rules.add_session_rule(&parent.to_string_lossy(), Access::Write, Decision::Allow);
        let sandbox = ResolvedSandbox {
            tier: SandboxTier::Manual,
            backend_receipt: crate::sandbox::platform_backend_receipt(),
            workspace: rules.workspace.clone(),
            rules,
        };
        let plan = build_plan(&sandbox);

        assert!(plan.writable_roots.contains(&parent));
        assert!(!plan.writable_roots.contains(&child));
    }
}
