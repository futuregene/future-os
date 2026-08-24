//! Platform-independent contract and preflight for Windows write capabilities.
//!
//! This module deliberately does not launch a process or mutate an ACL. It
//! freezes the model-declared targets, evaluates the normal approval rules and
//! produces trusted user semantics consumed by the approval flow and bound to
//! the exact receipt passed into the Windows process driver.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::paths;
use super::rules::{Decision, Op};
use super::ResolvedSandbox;

pub const MAX_WRITE_TARGETS: usize = 8;
const MAX_REASON_CHARS: usize = 240;

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct AdditionalPermissions {
    #[serde(default)]
    pub write: Vec<WritePermissionRequest>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct WritePermissionRequest {
    pub path: String,
    pub scope: WriteScope,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum WriteScope {
    File,
    Subtree,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenWriteTarget {
    /// Canonical, absolute path used by rules, request binding, and path audit.
    pub normalized_path: PathBuf,
    pub scope: WriteScope,
    /// Model-provided explanation retained for diagnostics only. It must never
    /// replace the trusted title or target list shown to the user.
    pub untrusted_reason: String,
    pub decision: Decision,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ApprovalTarget {
    pub path: String,
    pub scope: WriteScope,
}

/// Backend-generated semantics safe for the ordinary-user approval card.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CapabilityApprovalSemantics {
    /// Stable semantic action key; the client owns localization.
    pub behavior: &'static str,
    /// Complete list: the UI must never collapse it to "and N more".
    pub targets: Vec<ApprovalTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedWritePermissions {
    pub command_hash: String,
    pub targets: Vec<FrozenWriteTarget>,
    pub approval: Option<CapabilityApprovalSemantics>,
}

impl PreparedWritePermissions {
    pub fn needs_approval(&self) -> bool {
        self.approval.is_some()
    }

    pub fn approved_receipt(&self, request_id: String) -> Option<ApprovedWriteCapability> {
        Some(ApprovedWriteCapability {
            request_id,
            command_hash: self.command_hash.clone(),
            targets: self.approval.as_ref()?.targets.clone(),
        })
    }
}

/// Server-side receipt. It is held in the active tool scope and never sent to
/// the ordinary-user card. Exact equality is required again immediately before
/// the Windows process driver may consume it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedWriteCapability {
    pub request_id: String,
    pub command_hash: String,
    pub targets: Vec<ApprovalTarget>,
}

pub fn prepare(
    sandbox: &ResolvedSandbox,
    command: &str,
    permissions: &AdditionalPermissions,
) -> Result<PreparedWritePermissions> {
    let prepared = prepare_with(
        &sandbox.workspace,
        command,
        permissions,
        |path| sandbox.evaluate(path, Op::Write),
        crate::utils::home_dir_opt().as_deref(),
    )?;
    reject_explicit_ask_carveouts(&prepared, &super::windows_plan::build_plan(sandbox))?;
    Ok(prepared)
}

fn reject_explicit_ask_carveouts(
    prepared: &PreparedWritePermissions,
    plan: &super::windows_plan::WindowsSandboxPlan,
) -> Result<()> {
    for target in &prepared.targets {
        if target.decision != Decision::Ask {
            continue;
        }
        if let Some(carveout) = plan.write_carveouts.iter().find(|carveout| {
            carveout.decision == Decision::Ask
                && paths::path_within(&target.normalized_path, &carveout.path)
        }) {
            bail!(
                "Windows unelevated sandbox cannot reopen explicit ask carveout: {}",
                carveout.path.display()
            );
        }
    }
    Ok(())
}

fn prepare_with<F>(
    workspace: &Path,
    command: &str,
    permissions: &AdditionalPermissions,
    mut evaluate: F,
    home: Option<&Path>,
) -> Result<PreparedWritePermissions>
where
    F: FnMut(&Path) -> Decision,
{
    if permissions.write.len() > MAX_WRITE_TARGETS {
        bail!("additional_permissions.write accepts at most {MAX_WRITE_TARGETS} targets");
    }

    let canonical_home = home.and_then(|path| path.canonicalize().ok());
    let mut seen = HashSet::new();
    let mut targets = Vec::with_capacity(permissions.write.len());

    for request in &permissions.write {
        let raw_path = request.path.trim();
        if raw_path.is_empty() {
            bail!("write capability path must not be empty");
        }
        if request.reason.trim().is_empty() {
            bail!("write capability reason must not be empty");
        }
        if request.reason.chars().count() > MAX_REASON_CHARS {
            bail!("write capability reason is too long");
        }

        let resolved = paths::resolve_against(workspace, raw_path);
        let normalized = resolved
            .canonicalize()
            .with_context(|| format!("write capability target does not exist: {raw_path}"))?;
        reject_overbroad_target(&normalized, canonical_home.as_deref())?;
        validate_scope(&normalized, request.scope)?;

        let identity = (normalized.clone(), request.scope);
        if !seen.insert(identity) {
            // Exact duplicates add no authority and no information. Dropping
            // them keeps the complete displayed list honest and deterministic.
            continue;
        }

        let decision = evaluate(&normalized);
        if decision == Decision::Deny {
            bail!(
                "write capability is denied by policy: {}",
                normalized.display()
            );
        }
        targets.push(FrozenWriteTarget {
            normalized_path: normalized,
            scope: request.scope,
            untrusted_reason: request.reason.trim().to_owned(),
            decision,
        });
    }

    // A file and subtree request for the same object are not equivalent. Do
    // not guess which range the model intended or silently widen the grant.
    for (index, target) in targets.iter().enumerate() {
        if targets[index + 1..].iter().any(|other| {
            other.normalized_path == target.normalized_path && other.scope != target.scope
        }) {
            bail!(
                "conflicting write capability scopes for {}",
                target.normalized_path.display()
            );
        }
    }

    let approval_targets = targets
        .iter()
        .filter(|target| target.decision == Decision::Ask)
        .map(|target| ApprovalTarget {
            path: target.normalized_path.to_string_lossy().into_owned(),
            scope: target.scope,
        })
        .collect::<Vec<_>>();
    let approval = (!approval_targets.is_empty()).then_some(CapabilityApprovalSemantics {
        behavior: "manage_files",
        targets: approval_targets,
    });

    Ok(PreparedWritePermissions {
        command_hash: sha256_hex(command.as_bytes()),
        targets,
        approval,
    })
}

fn reject_overbroad_target(path: &Path, home: Option<&Path>) -> Result<()> {
    if path.parent().is_none() {
        bail!("filesystem or volume roots cannot be granted as write capabilities");
    }
    if home.is_some_and(|home| path == home) {
        bail!("the user home root cannot be granted as a write capability");
    }
    Ok(())
}

fn validate_scope(path: &Path, scope: WriteScope) -> Result<()> {
    let metadata = path
        .metadata()
        .with_context(|| format!("cannot inspect write capability target: {}", path.display()))?;
    match scope {
        WriteScope::File if !metadata.is_file() => {
            bail!(
                "file scope requires an existing regular file: {}",
                path.display()
            )
        }
        WriteScope::Subtree if !metadata.is_dir() => {
            bail!(
                "subtree scope requires an existing directory: {}",
                path.display()
            )
        }
        _ => Ok(()),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut result = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut result, "{byte:02x}").expect("writing to String cannot fail");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("futureos-w4-{name}-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    fn subtree(path: &Path, reason: &str) -> WritePermissionRequest {
        WritePermissionRequest {
            path: path.to_string_lossy().into_owned(),
            scope: WriteScope::Subtree,
            reason: reason.to_owned(),
        }
    }

    #[test]
    fn allow_targets_do_not_create_an_approval() {
        let workspace = temp_dir("allow");
        let permissions = AdditionalPermissions {
            write: vec![subtree(&workspace, "build output")],
        };
        let prepared = prepare_with(
            &workspace,
            "cargo build",
            &permissions,
            |_| Decision::Allow,
            None,
        )
        .unwrap();

        assert!(!prepared.needs_approval());
        assert_eq!(prepared.targets.len(), 1);
        assert_eq!(prepared.command_hash.len(), 64);
    }

    #[test]
    fn ask_targets_generate_only_trusted_behavior_and_complete_targets() {
        let workspace = temp_dir("ask-workspace");
        let first = temp_dir("ask-first");
        let second = temp_dir("ask-second");
        let permissions = AdditionalPermissions {
            write: vec![
                subtree(&first, "MODEL TEXT MUST NOT BECOME THE TITLE"),
                subtree(&second, "another model reason"),
            ],
        };
        let prepared = prepare_with(
            &workspace,
            "build-release",
            &permissions,
            |_| Decision::Ask,
            None,
        )
        .unwrap();

        let approval = prepared.approval.unwrap();
        assert_eq!(approval.behavior, "manage_files");
        assert_eq!(approval.targets.len(), 2);
        let json = serde_json::to_string(&approval).unwrap();
        assert!(!json.contains("MODEL TEXT"));
        assert!(!json.contains("another model reason"));
    }

    #[test]
    fn deny_is_not_approvable() {
        let workspace = temp_dir("deny-workspace");
        let target = temp_dir("deny-target");
        let error = prepare_with(
            &workspace,
            "write-secret",
            &AdditionalPermissions {
                write: vec![subtree(&target, "needed")],
            },
            |_| Decision::Deny,
            None,
        )
        .unwrap_err();
        assert!(error.to_string().contains("denied by policy"));
    }

    #[test]
    fn explicit_ask_carveout_is_rejected_instead_of_showing_a_broken_approval() {
        let workspace = temp_dir("explicit-ask-workspace");
        let target = workspace.join("protected");
        std::fs::create_dir_all(&target).unwrap();
        let prepared = prepare_with(
            &workspace,
            "write-protected",
            &AdditionalPermissions {
                write: vec![subtree(&target, "needed")],
            },
            |_| Decision::Ask,
            None,
        )
        .unwrap();
        let plan = super::super::windows_plan::WindowsSandboxPlan {
            writable_roots: vec![workspace],
            write_carveouts: vec![super::super::windows_plan::WindowsWriteCarveout {
                path: target,
                decision: Decision::Ask,
            }],
            ..super::super::windows_plan::WindowsSandboxPlan::default()
        };

        let error = reject_explicit_ask_carveouts(&prepared, &plan).unwrap_err();
        assert!(error.to_string().contains("cannot reopen explicit ask"));
    }

    #[test]
    fn more_than_eight_targets_is_rejected_before_path_access() {
        let workspace = temp_dir("too-many");
        let permissions = AdditionalPermissions {
            write: (0..9)
                .map(|index| WritePermissionRequest {
                    path: format!("missing-{index}"),
                    scope: WriteScope::Subtree,
                    reason: "needed".to_owned(),
                })
                .collect(),
        };
        let error =
            prepare_with(&workspace, "command", &permissions, |_| Decision::Ask, None).unwrap_err();
        assert!(error.to_string().contains("at most 8"));
    }

    #[test]
    fn exact_duplicates_are_deduplicated() {
        let workspace = temp_dir("dedupe-workspace");
        let target = temp_dir("dedupe-target");
        let permissions = AdditionalPermissions {
            write: vec![subtree(&target, "one"), subtree(&target, "two")],
        };
        let prepared =
            prepare_with(&workspace, "command", &permissions, |_| Decision::Ask, None).unwrap();
        assert_eq!(prepared.targets.len(), 1);
        assert_eq!(prepared.approval.unwrap().targets.len(), 1);
    }

    #[test]
    fn nonexistent_file_cannot_silently_expand_to_parent() {
        let workspace = temp_dir("missing-file");
        let missing = workspace.join("new.txt");
        let permissions = AdditionalPermissions {
            write: vec![WritePermissionRequest {
                path: missing.to_string_lossy().into_owned(),
                scope: WriteScope::File,
                reason: "create one file".to_owned(),
            }],
        };
        let error =
            prepare_with(&workspace, "command", &permissions, |_| Decision::Ask, None).unwrap_err();
        assert!(error.to_string().contains("does not exist"));
    }

    #[test]
    fn file_and_subtree_scopes_validate_target_kind() {
        let workspace = temp_dir("scope-workspace");
        let file = workspace.join("existing.txt");
        std::fs::write(&file, "x").unwrap();

        let file_request = AdditionalPermissions {
            write: vec![WritePermissionRequest {
                path: file.to_string_lossy().into_owned(),
                scope: WriteScope::File,
                reason: "modify file".to_owned(),
            }],
        };
        assert!(prepare_with(
            &workspace,
            "command",
            &file_request,
            |_| Decision::Ask,
            None,
        )
        .is_ok());

        let wrong_scope = AdditionalPermissions {
            write: vec![WritePermissionRequest {
                path: file.to_string_lossy().into_owned(),
                scope: WriteScope::Subtree,
                reason: "wrong".to_owned(),
            }],
        };
        assert!(
            prepare_with(&workspace, "command", &wrong_scope, |_| Decision::Ask, None,).is_err()
        );
    }

    #[test]
    fn filesystem_and_home_roots_are_rejected() {
        let workspace = temp_dir("broad-workspace");
        let root = workspace.ancestors().last().unwrap();
        let root_error = prepare_with(
            &workspace,
            "command",
            &AdditionalPermissions {
                write: vec![subtree(root, "too broad")],
            },
            |_| Decision::Ask,
            None,
        )
        .unwrap_err();
        assert!(root_error.to_string().contains("roots cannot be granted"));

        let home = temp_dir("fake-home");
        let home_error = prepare_with(
            &workspace,
            "command",
            &AdditionalPermissions {
                write: vec![subtree(&home, "too broad")],
            },
            |_| Decision::Ask,
            Some(&home),
        )
        .unwrap_err();
        assert!(home_error.to_string().contains("home root"));
    }
}
