//! Single-command Unix filesystem grants. Receipts never grant session authority.
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::rules::{Access, Decision, Op};
use super::windows_request::{WritePermissionRequest, WriteScope};
use super::{paths, ResolvedSandbox};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdditionalPermissions {
    #[serde(default)]
    pub read: Vec<WritePermissionRequest>,
    #[serde(default)]
    pub write: Vec<WritePermissionRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileTarget {
    pub path: PathBuf,
    pub scope: WriteScope,
    pub access: Access,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PermissionBinding {
    command_hash: String,
    cwd: PathBuf,
    policy_digest: String,
    identities: Vec<(u64, u64)>,
    targets: Vec<FileTarget>,
}

#[derive(Debug, Clone)]
pub struct PreparedPermissions {
    binding: PermissionBinding,
    /// Only ask targets become grants; already-allowed parents cannot unguard children.
    pub targets: Vec<FileTarget>,
    sandbox: ResolvedSandbox,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedPermissions {
    request_id: String,
    pub invocation_id: String,
    binding: PermissionBinding,
}

impl PreparedPermissions {
    pub fn needs_approval(&self) -> bool {
        !self.targets.is_empty()
    }

    pub fn approved_receipt(
        &self,
        request_id: String,
        invocation_id: String,
    ) -> ApprovedPermissions {
        ApprovedPermissions {
            request_id,
            invocation_id,
            binding: self.binding.clone(),
        }
    }

    pub fn matches(&self, receipt: &ApprovedPermissions) -> bool {
        !receipt.request_id.is_empty() && self.binding == receipt.binding
    }

    pub fn sandbox(&self) -> ResolvedSandbox {
        self.sandbox.clone()
    }
}

pub fn prepare(
    sandbox: &ResolvedSandbox,
    command: &str,
    permissions: &AdditionalPermissions,
) -> Result<PreparedPermissions> {
    use std::os::unix::fs::MetadataExt;
    if !sandbox.wraps_shell() {
        bail!("additional_permissions requires an active OS sandbox; use a normal command for the current approval tier");
    }
    // Freeze mutable session rules once for evaluation, binding and compilation.
    let mut frozen = sandbox.clone();
    frozen.rules = sandbox.rules.with_execution_grants(&[]);
    let sandbox = &frozen;
    let snapshot = sandbox.rules.snapshot();
    if let Some(error) = snapshot.resolution_errors.first() {
        bail!("approval rule layer could not be loaded: {error}");
    }
    if permissions.read.len() + permissions.write.len() > 8 {
        bail!("additional_permissions accepts at most 8 file targets in total");
    }
    let home = crate::utils::home_dir_opt().map(|home| paths::canonicalize_lenient(&home));
    let mut targets = Vec::new();
    let mut identities = Vec::new();
    for (requests, access, op) in [
        (&permissions.read, Access::Read, Op::Read),
        (&permissions.write, Access::Write, Op::Write),
    ] {
        for request in requests {
            if request.path.trim().is_empty() || request.path.contains(['\0', '*', '?', '[', ']']) {
                bail!("file permission path must be a nonempty literal path without wildcards");
            }
            if request.reason.trim().is_empty() || request.reason.chars().count() > 240 {
                bail!("file permission reason must contain 1..240 characters");
            }
            let resolved = paths::resolve_against(&sandbox.workspace, &request.path);
            let path = paths::canonicalize_existing(&resolved).with_context(|| {
                format!("permission target must exist: {}. To create a file, request subtree access to its existing parent directory", request.path)
            })?;
            if path.to_str().is_none() {
                bail!("permission target cannot be displayed losslessly as UTF-8");
            }
            if path.parent().is_none() || home.as_ref() == Some(&path) {
                bail!("filesystem and home roots cannot be granted");
            }
            let metadata = path.metadata()?;
            match request.scope {
                WriteScope::File if !metadata.is_file() => {
                    bail!("file scope requires an existing regular file")
                }
                WriteScope::Subtree if !metadata.is_dir() => {
                    bail!("subtree scope requires an existing directory")
                }
                _ => {}
            }
            if sandbox.rules.explicitly_denied(&path, op) {
                bail!("file permission is denied by policy: {}", path.display());
            }
            match sandbox.evaluate(&path, op) {
                Decision::Deny => bail!("file permission is denied by policy: {}", path.display()),
                Decision::Allow => continue,
                Decision::Ask => {}
            }
            let target = FileTarget {
                path,
                scope: request.scope,
                access,
            };
            if targets.contains(&target) {
                continue;
            }
            if targets
                .iter()
                .any(|other: &FileTarget| other.path == target.path && other.scope != target.scope)
            {
                bail!("conflicting file permission scopes");
            }
            identities.push((metadata.dev(), metadata.ino()));
            targets.push(target);
        }
    }
    let binding = PermissionBinding {
        command_hash: format!("{:x}", Sha256::digest(command.as_bytes())),
        cwd: sandbox.workspace.clone(),
        policy_digest: super::linux::plan::policy_digest(&snapshot)?,
        identities,
        targets: targets.clone(),
    };
    let mut granted = sandbox.clone();
    granted.rules = sandbox.rules.with_execution_grants(&targets);
    let prepared = PreparedPermissions {
        binding,
        targets,
        sandbox: granted,
    };
    // Fail before showing an approval if the Linux mount backend cannot express it.
    #[cfg(target_os = "linux")]
    {
        let plan =
            super::linux::plan::LinuxSandboxPlan::compile(&prepared.sandbox().rules.snapshot())?;
        // Opaque directories have mode 000. A bind of a child does not grant
        // traversal of its parent. Do not widen to an unapproved directory or
        // offer an approval the current helper cannot actually enforce.
        for target in &prepared.targets {
            if plan
                .unreadable_paths
                .iter()
                .any(|root| root != &target.path && paths::path_within(&target.path, root))
            {
                bail!("Linux cannot grant a child path inside an unreadable directory with this mount backend; use the approved read/write/edit tool for the specific file");
            }
        }
    }
    Ok(prepared)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, ResolvedSandbox, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("work");
        let home = dir.path().join("home");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        let secret = workspace.join(".env");
        std::fs::write(&secret, "secret").unwrap();
        std::fs::write(workspace.join(".env.other"), "other secret").unwrap();
        let rules = super::super::rules::RuleSet::resolve_isolated_with_home(&workspace, &home);
        let sandbox = ResolvedSandbox::with_rules_for_test(rules);
        (dir, sandbox, paths::canonicalize_existing(&secret).unwrap())
    }

    fn request(path: &std::path::Path, scope: WriteScope) -> WritePermissionRequest {
        WritePermissionRequest {
            path: path.to_string_lossy().into_owned(),
            scope,
            reason: "Read task configuration".into(),
        }
    }

    #[test]
    fn read_grant_is_exact_temporary_and_does_not_grant_writes() {
        let (_dir, sandbox, secret) = fixture();
        let permissions = AdditionalPermissions {
            read: vec![request(&secret, WriteScope::File)],
            ..Default::default()
        };
        let prepared = prepare(&sandbox, "cat key", &permissions).unwrap();
        let receipt = prepared.approved_receipt("request-1".into(), "tool-1".into());
        assert!(prepared.matches(&receipt));
        let granted = prepared.sandbox();
        assert_eq!(granted.evaluate(&secret, Op::Read), Decision::Allow);
        assert_eq!(granted.evaluate(&secret, Op::Write), Decision::Ask);
        assert_eq!(sandbox.evaluate(&secret, Op::Read), Decision::Ask);
        assert_eq!(
            granted.evaluate(&secret.with_file_name(".env.other"), Op::Read),
            Decision::Ask
        );
        assert!(!prepare(&sandbox, "cat other", &permissions)
            .unwrap()
            .matches(&receipt));
        let snapshot = granted.rules.snapshot();
        assert!(snapshot
            .layers
            .iter()
            .any(|layer| layer.rules.iter().any(|rule| matches!(
                rule.matcher,
                super::super::rules::RuleMatcherSnapshot::Literal { .. }
            ))));
    }

    #[test]
    fn deny_carveouts_survive_a_parent_directory_grant() {
        let (_dir, sandbox, secret) = fixture();
        let protected = secret.with_file_name("denied");
        std::fs::write(&protected, "protected").unwrap();
        sandbox
            .rules
            .add_session_rule(&protected.to_string_lossy(), Access::Both, Decision::Deny);
        sandbox.rules.add_session_rule(
            &sandbox.workspace.to_string_lossy(),
            Access::Read,
            Decision::Ask,
        );
        let permissions = AdditionalPermissions {
            read: vec![request(secret.parent().unwrap(), WriteScope::Subtree)],
            ..Default::default()
        };
        let prepared = prepare(&sandbox, "read permitted keys", &permissions).unwrap();
        let granted = prepared.sandbox();
        assert_eq!(granted.evaluate(&secret, Op::Read), Decision::Allow);
        assert_eq!(granted.evaluate(&protected, Op::Read), Decision::Deny);
        let denied = AdditionalPermissions {
            read: vec![request(&protected, WriteScope::File)],
            ..Default::default()
        };
        // No explicit deny becomes approvable through a broader ask/grant.
        assert!(prepare(&sandbox, "cat denied", &denied).is_err());
    }

    #[test]
    fn receipt_rejects_changed_policy_or_replaced_inode() {
        let (_dir, sandbox, secret) = fixture();
        let permissions = AdditionalPermissions {
            read: vec![request(&secret, WriteScope::File)],
            ..Default::default()
        };
        let receipt = prepare(&sandbox, "cat key", &permissions)
            .unwrap()
            .approved_receipt("request-1".into(), "tool-1".into());
        let replacement = secret.with_file_name("replacement");
        std::fs::write(&replacement, "replacement").unwrap();
        std::fs::rename(&replacement, &secret).unwrap();
        assert!(!prepare(&sandbox, "cat key", &permissions)
            .unwrap()
            .matches(&receipt));
        let receipt = prepare(&sandbox, "cat key", &permissions)
            .unwrap()
            .approved_receipt("request-2".into(), "tool-1".into());
        sandbox
            .rules
            .add_session_rule("/unrelated", Access::Read, Decision::Allow);
        assert!(!prepare(&sandbox, "cat key", &permissions)
            .unwrap()
            .matches(&receipt));
    }

    #[test]
    fn reject_missing_broad_unsupported_and_unsandboxed_requests() {
        let (_dir, sandbox, secret) = fixture();
        for (path, scope) in [
            (secret.with_file_name("missing"), WriteScope::File),
            (PathBuf::from("/"), WriteScope::Subtree),
            (secret.clone(), WriteScope::Subtree),
        ] {
            let permissions = AdditionalPermissions {
                read: vec![request(&path, scope)],
                ..Default::default()
            };
            assert!(prepare(&sandbox, "read", &permissions).is_err());
        }
        assert!(serde_json::from_value::<AdditionalPermissions>(
            serde_json::json!({"network": true})
        )
        .is_err());
        let permissions = AdditionalPermissions {
            read: vec![request(&secret, WriteScope::File)],
            ..Default::default()
        };
        assert!(prepare(&ResolvedSandbox::default(), "read", &permissions).is_err());
    }

    #[test]
    fn already_allowed_directory_does_not_authorize_guarded_children() {
        let (_dir, sandbox, _secret) = fixture();
        let guarded = sandbox.workspace.join(".env");
        std::fs::write(&guarded, "secret").unwrap();
        let permissions = AdditionalPermissions {
            read: vec![request(&sandbox.workspace, WriteScope::Subtree)],
            ..Default::default()
        };
        let prepared = prepare(&sandbox, "read", &permissions).unwrap();
        assert!(!prepared.needs_approval());
        assert_eq!(
            prepared.sandbox().evaluate(&guarded, Op::Read),
            Decision::Ask
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn child_of_an_opaque_directory_is_rejected_before_approval() {
        let (_dir, sandbox, _secret) = fixture();
        let home = sandbox.workspace.parent().unwrap().join("home");
        let key = home.join(".ssh/key");
        std::fs::write(&key, "secret").unwrap();
        let permissions = AdditionalPermissions {
            read: vec![request(&key, WriteScope::File)],
            ..Default::default()
        };
        let error = prepare(&sandbox, "cat key", &permissions).unwrap_err();
        assert!(error.to_string().contains("unreadable directory"));
        let directory = AdditionalPermissions {
            read: vec![request(&home.join(".ssh"), WriteScope::Subtree)],
            ..Default::default()
        };
        assert!(prepare(&sandbox, "read declared directory", &directory).is_ok());
    }
}
