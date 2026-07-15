//! Platform-independent derivation of the Windows shell-sandbox enforcement plan
//! from the resolved rule set (SANDBOX_PLAN.md §11).
//!
//! The Win32 executor (`windows.rs`, `#[cfg(windows)]`) turns this plan into a
//! restricted token + a set of NTFS ACEs + a job object. Keeping the derivation
//! pure lets it be unit-tested on any platform — the same split as
//! `seatbelt::build_profile` (pure) vs `seatbelt::build_command` (syscalls).
//!
//! Two NTFS limitations shape what lands in the plan (see §11.3/§11.6):
//!   - **No glob ACEs.** Rules whose matcher is a glob (e.g. workspace
//!     `**/*.pem`) cannot be expressed as a path ACE, so they are counted in
//!     `skipped_globs` and left to the in-process tool layer. Subtree / literal
//!     rules (home secrets, credentials, rule files, literal `.env`) ARE
//!     enforced.
//!   - **NTFS is always deny-wins.** We collect deny paths and writable subtrees
//!     separately; a deny ACE always beats the broad read / workspace-write
//!     grant. This is stricter than the engine's first-match in the rare
//!     higher-allow-over-lower-deny case, which errs safe (an extra escalation).

#![allow(dead_code)] // Consumed by the `#[cfg(windows)]` executor (W1b).

use std::path::PathBuf;

use super::rules::{Decision, MatcherSbpl};
use super::ResolvedSandbox;

/// The enforcement plan for one sandboxed shell run: which subtrees the sandbox
/// principal may write, which paths are denied, and how many glob rules could
/// not be expressed as ACEs.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WindowsSandboxPlan {
    /// Subtrees the sandbox SID gets an explicit write ACE on (workspace + temp
    /// + allow-write subtree rules). Everything else stays unwritable.
    pub writable: Vec<PathBuf>,
    /// Paths denied read (home secrets, credentials, literal in-workspace
    /// secrets) — an explicit deny-read ACE, which wins over the broad read.
    pub deny_read: Vec<PathBuf>,
    /// Paths denied write (rule files, credentials, in-workspace secrets) — an
    /// explicit deny-write ACE, which wins over the workspace write grant.
    pub deny_write: Vec<PathBuf>,
    /// Count of glob rules NTFS ACLs cannot express, hence unenforced for shell runs
    /// (still covered by the in-process tool layer). Surfaced for diagnostics.
    pub skipped_globs: usize,
}

/// Derive the plan from a resolved sandbox's rule set. Reads stay broadly open
/// (delivered by the restricted token's SID set, not by per-path ACEs), so
/// allow-read rules need no ACE and are not collected here.
pub fn build_plan(sandbox: &ResolvedSandbox) -> WindowsSandboxPlan {
    let rules = sandbox.rule_set();
    let mut plan = WindowsSandboxPlan::default();

    // Base writable roots: workspace + temp (mirrors the engine's write
    // fallback and the Seatbelt base).
    plan.writable.push(rules.workspace.clone());
    for tmp in super::rules::temp_roots() {
        plan.writable.push(tmp);
    }

    for layer in rules.profile_layers() {
        for rule in &layer {
            let base = match rule.matcher_sbpl() {
                MatcherSbpl::Subtree(base) => base.to_path_buf(),
                // Globs (e.g. workspace `**/*.pem`) have no ACE form.
                MatcherSbpl::Regex(_) => {
                    plan.skipped_globs += 1;
                    continue;
                }
            };
            let access = rule.access();
            match rule.decision() {
                Decision::Allow => {
                    // Only write grants need an ACE; reads are broadly open.
                    if access.covers_write() {
                        plan.writable.push(base);
                    }
                }
                Decision::Ask | Decision::Deny => {
                    // `ask` and `deny` both become an OS-level denial for shell runs
                    // (it can't prompt mid-syscall) — same as Seatbelt.
                    if access.covers_read() {
                        plan.deny_read.push(base.clone());
                    }
                    if access.covers_write() {
                        plan.deny_write.push(base);
                    }
                }
            }
        }
    }

    dedup(&mut plan.writable);
    dedup(&mut plan.deny_read);
    dedup(&mut plan.deny_write);
    plan
}

/// Stable de-duplication preserving first occurrence.
fn dedup(paths: &mut Vec<PathBuf>) {
    let mut seen = std::collections::HashSet::new();
    paths.retain(|path| seen.insert(path.clone()));
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

    #[test]
    fn workspace_and_temp_are_writable() {
        let ws = temp_workspace();
        let plan = plan_for(&ws);
        let workspace = crate::sandbox::paths::canonicalize_lenient(std::path::Path::new(&ws));
        assert!(
            plan.writable.contains(&workspace),
            "workspace must be writable"
        );
        assert!(
            super::super::rules::temp_roots()
                .iter()
                .all(|t| plan.writable.contains(t)),
            "temp roots must be writable"
        );
    }

    #[test]
    fn rule_file_is_deny_write_not_deny_read() {
        let ws = temp_workspace();
        let plan = plan_for(&ws);
        let workspace = crate::sandbox::paths::canonicalize_lenient(std::path::Path::new(&ws));
        let rule_file = workspace.join(".future/approval_rule.json");
        assert!(
            plan.deny_write.contains(&rule_file),
            "rule file write must be denied"
        );
        assert!(
            !plan.deny_read.contains(&rule_file),
            "rule file is not a read secret"
        );
    }

    #[test]
    fn home_ssh_subtree_is_deny_read() {
        let ws = temp_workspace();
        let plan = plan_for(&ws);
        let ssh =
            crate::sandbox::paths::canonicalize_lenient(&dirs::home_dir().unwrap().join(".ssh"));
        assert!(
            plan.deny_read.contains(&ssh),
            "~/.ssh subtree must be deny-read"
        );
        assert!(
            plan.deny_write.contains(&ssh),
            "~/.ssh subtree must be deny-write (Both)"
        );
    }

    #[test]
    fn literal_env_is_enforced_but_glob_secrets_are_skipped() {
        let ws = temp_workspace();
        let plan = plan_for(&ws);
        let workspace = crate::sandbox::paths::canonicalize_lenient(std::path::Path::new(&ws));
        // Literal `.env` (no glob metachars) → an enforceable deny.
        let env = workspace.join(".env");
        assert!(
            plan.deny_read.contains(&env),
            "literal workspace .env must be deny-read"
        );
        // Glob workspace secrets (`.env.*`, `**/*.pem`, `**/*.key`, `**/*.p12`,
        // `**/id_rsa*`) cannot be ACE'd → counted, not enforced.
        assert!(
            plan.skipped_globs >= 5,
            "workspace glob secrets must be counted as skipped"
        );
    }
}
