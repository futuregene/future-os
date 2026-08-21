//! Stable capability identities and crash-safe metadata for the Windows
//! unelevated sandbox. This module is platform-independent so policy identity
//! and persistence behavior can be tested on macOS/Linux CI.

#![allow(dead_code)] // Consumed by the Windows-only W2 executor.

use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::rules::Decision;
use super::windows_plan::{WindowsRuleMatcher, WindowsSandboxPlan};
use super::windows_request::WriteScope;

const STATE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    Policy,
    Request,
}

/// One deterministic Windows capability name. On Windows the name is passed
/// to `DeriveCapabilitySidsFromName`; metadata stores the name, never raw SID
/// pointers or mutable ACL state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityRecord {
    pub name: String,
    pub kind: CapabilityKind,
    pub policy_fingerprint: String,
    pub writable_root: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Frozen ask targets approved by this request. Repeated on request records
    /// so persisted metadata remains self-contained for audit/recovery.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub approved_targets: Vec<ApprovedCapabilityTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ApprovedCapabilityTarget {
    pub path: PathBuf,
    pub scope: WriteScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityState {
    pub schema_version: u32,
    pub records: Vec<CapabilityRecord>,
}

impl Default for CapabilityState {
    fn default() -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            records: Vec::new(),
        }
    }
}

/// Fingerprint every write-relevant part of the plan. Read diagnostics are
/// intentionally excluded: Windows shell read rules do not affect its token or
/// ACL generation and therefore must not churn write-policy identities.
pub fn policy_fingerprint(plan: &WindowsSandboxPlan) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"futureos-windows-write-policy-v1\0");
    for root in &plan.writable_roots {
        hash_path(&mut hasher, b"root", root);
    }
    for carveout in &plan.write_carveouts {
        hash_path(&mut hasher, b"carveout", &carveout.path);
        hasher.update(match carveout.decision {
            Decision::Ask => b"ask".as_slice(),
            Decision::Deny => b"deny".as_slice(),
            Decision::Allow => b"allow".as_slice(),
        });
        hasher.update([0]);
    }
    for rule in &plan.unsupported_write_globs {
        hasher.update(b"unsupported-glob\0");
        match &rule.matcher {
            WindowsRuleMatcher::Subtree(path) => hash_path(&mut hasher, b"subtree", path),
            WindowsRuleMatcher::Regex(regex) => {
                hasher.update(b"regex\0");
                hasher.update(regex.as_bytes());
                hasher.update([0]);
            }
        }
        hasher.update(match rule.decision {
            Decision::Ask => b"ask".as_slice(),
            Decision::Deny => b"deny".as_slice(),
            Decision::Allow => b"allow".as_slice(),
        });
        hasher.update([0]);
    }
    hex_digest(hasher.finalize())
}

/// One stable capability per write root and policy generation. A policy change
/// produces new names; old ACEs grant nothing unless an old SID is deliberately
/// loaded into a token that is already running.
pub fn policy_records(plan: &WindowsSandboxPlan) -> Vec<CapabilityRecord> {
    let fingerprint = policy_fingerprint(plan);
    plan.writable_roots
        .iter()
        .map(|root| make_record(CapabilityKind::Policy, &fingerprint, root, None, &[]))
        .collect()
}

/// Request-scoped capabilities used when a one-time approval reopens an `ask`
/// carveout. The set contains both the normal policy roots and one independent
/// SID for every approved target. The latter is essential: an approved target
/// may be outside all policy roots, and using its parent/root SID would widen
/// the grant to siblings. The frozen approved paths are identity input; a
/// changed request or target necessarily produces a different SID set.
pub fn request_records(
    plan: &WindowsSandboxPlan,
    request_id: &str,
    approved_roots: &[(PathBuf, WriteScope)],
) -> io::Result<Vec<CapabilityRecord>> {
    if request_id.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "capability request id must not be empty",
        ));
    }
    let fingerprint = policy_fingerprint(plan);
    let mut seen = HashSet::new();
    let mut approved_targets = approved_roots
        .iter()
        .map(|(path, scope)| ApprovedCapabilityTarget {
            path: path.clone(),
            scope: *scope,
        })
        .collect::<Vec<_>>();
    approved_targets.sort_by(|left, right| {
        left.path.cmp(&right.path).then_with(|| {
            scope_tag(left.scope)
                .as_bytes()
                .cmp(scope_tag(right.scope).as_bytes())
        })
    });
    approved_targets.retain(|target| seen.insert(target.clone()));
    if approved_targets.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "request capability requires at least one approved path",
        ));
    }
    let mut roots = plan.writable_roots.clone();
    roots.extend(approved_targets.iter().map(|target| target.path.clone()));
    roots.sort();
    roots.dedup();
    Ok(roots
        .iter()
        .map(|root| {
            make_record(
                CapabilityKind::Request,
                &fingerprint,
                root,
                Some(request_id),
                &approved_targets,
            )
        })
        .collect())
}

fn make_record(
    kind: CapabilityKind,
    policy_fingerprint: &str,
    root: &Path,
    request_id: Option<&str>,
    approved_targets: &[ApprovedCapabilityTarget],
) -> CapabilityRecord {
    let mut hasher = Sha256::new();
    hasher.update(b"futureos-windows-capability-v1\0");
    hasher.update(match kind {
        CapabilityKind::Policy => b"policy".as_slice(),
        CapabilityKind::Request => b"request".as_slice(),
    });
    hasher.update([0]);
    hasher.update(policy_fingerprint.as_bytes());
    hasher.update([0]);
    hash_path(&mut hasher, b"root", root);
    if let Some(request_id) = request_id {
        hasher.update(request_id.as_bytes());
        hasher.update([0]);
    }
    for target in approved_targets {
        hash_path(&mut hasher, b"approved", &target.path);
        hasher.update(scope_tag(target.scope).as_bytes());
        hasher.update([0]);
    }
    let identity = hex_digest(hasher.finalize());
    CapabilityRecord {
        name: format!("futureos.windows.{}", &identity[..40]),
        kind,
        policy_fingerprint: policy_fingerprint.to_owned(),
        writable_root: root.to_path_buf(),
        request_id: request_id.map(str::to_owned),
        approved_targets: approved_targets.to_vec(),
    }
}

fn scope_tag(scope: WriteScope) -> &'static str {
    match scope {
        WriteScope::File => "file",
        WriteScope::Subtree => "subtree",
    }
}

fn hash_path(hasher: &mut Sha256, label: &[u8], path: &Path) {
    hasher.update(label);
    hasher.update([0]);
    hasher.update(path.to_string_lossy().as_bytes());
    hasher.update([0]);
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = bytes.as_ref();
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

impl CapabilityState {
    pub fn load(path: &Path) -> io::Result<Self> {
        let bytes = fs::read(path)?;
        let state: Self = serde_json::from_slice(&bytes).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid capability state: {error}"),
            )
        })?;
        state.validate()?;
        Ok(state)
    }

    pub fn merge(&mut self, records: impl IntoIterator<Item = CapabilityRecord>) -> io::Result<()> {
        for record in records {
            if let Some(existing) = self.records.iter().find(|item| item.name == record.name) {
                if existing != &record {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("capability identity collision: {}", record.name),
                    ));
                }
                continue;
            }
            self.records.push(record);
        }
        self.records
            .sort_by(|left, right| left.name.cmp(&right.name));
        self.validate()
    }

    pub fn save_atomic(&self, path: &Path) -> io::Result<()> {
        self.validate()?;
        let parent = path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "state path has no parent")
        })?;
        fs::create_dir_all(parent)?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("state");
        let temporary = parent.join(format!(".{file_name}.{}.{}.tmp", std::process::id(), stamp));
        let bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            atomic_replace(&temporary, path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    fn validate(&self) -> io::Result<()> {
        if self.schema_version != STATE_SCHEMA_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "unsupported capability state schema: {}",
                    self.schema_version
                ),
            ));
        }
        let mut names = HashSet::new();
        for record in &self.records {
            if record.name.is_empty()
                || record.policy_fingerprint.len() != 64
                || !record.writable_root.is_absolute()
                || !names.insert(&record.name)
                || matches!(record.kind, CapabilityKind::Request) != record.request_id.is_some()
                || (matches!(record.kind, CapabilityKind::Request)
                    != !record.approved_targets.is_empty())
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid capability record: {}", record.name),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn atomic_replace(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // SAFETY: both buffers are owned, NUL-terminated UTF-16 paths and remain
    // alive for the duration of the synchronous call.
    let ok = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::windows_plan::{WindowsSandboxPlan, WindowsWriteCarveout};

    fn plan() -> WindowsSandboxPlan {
        let workspace = std::env::current_dir().unwrap().join("winplan-workspace");
        let temporary = std::env::temp_dir().join("futureos-winplan-temp");
        WindowsSandboxPlan {
            writable_roots: vec![workspace.clone(), temporary],
            write_carveouts: vec![WindowsWriteCarveout {
                path: workspace.join(".env"),
                decision: Decision::Ask,
            }],
            ..WindowsSandboxPlan::default()
        }
    }

    #[test]
    fn policy_identity_is_stable_and_root_scoped() {
        let records = policy_records(&plan());
        assert_eq!(records.len(), 2);
        assert_ne!(records[0].name, records[1].name);
        assert_eq!(records, policy_records(&plan()));
    }

    #[test]
    fn policy_change_rotates_capability_names() {
        let before = policy_records(&plan());
        let mut changed = plan();
        changed.write_carveouts[0].decision = Decision::Deny;
        let after = policy_records(&changed);
        assert!(before
            .iter()
            .zip(after.iter())
            .all(|(left, right)| left.name != right.name));
    }

    #[test]
    fn request_identity_binds_request_and_deduplicates_roots() {
        let release = std::env::current_dir().unwrap().join("release");
        let roots = vec![
            (release.clone(), WriteScope::Subtree),
            (release.clone(), WriteScope::Subtree),
        ];
        let first = request_records(&plan(), "request-a", &roots).unwrap();
        let second = request_records(&plan(), "request-b", &roots).unwrap();
        assert_eq!(first.len(), plan().writable_roots.len() + 1);
        assert!(first
            .iter()
            .zip(second.iter())
            .all(|(left, right)| left.name != right.name));
        assert_eq!(first[0].request_id.as_deref(), Some("request-a"));
        assert_eq!(
            first[0].approved_targets,
            vec![ApprovedCapabilityTarget {
                path: release.clone(),
                scope: WriteScope::Subtree,
            }]
        );
        assert!(first.iter().any(|record| record.writable_root == release));
    }

    #[test]
    fn request_keeps_nested_approved_root_instead_of_widening_to_parent() {
        let policy = plan();
        let approved = policy.writable_roots[0].join("protected/release");
        let records = request_records(
            &policy,
            "request-nested",
            &[(approved.clone(), WriteScope::Subtree)],
        )
        .unwrap();

        assert!(records
            .iter()
            .any(|record| record.writable_root == policy.writable_roots[0]));
        assert!(records
            .iter()
            .any(|record| record.writable_root == approved));
    }

    #[test]
    fn request_identity_binds_file_vs_subtree_scope() {
        let target = std::env::current_dir().unwrap().join("scope-target");
        let file = request_records(
            &plan(),
            "request-scope",
            &[(target.clone(), WriteScope::File)],
        )
        .unwrap();
        let subtree =
            request_records(&plan(), "request-scope", &[(target, WriteScope::Subtree)]).unwrap();
        assert!(file
            .iter()
            .zip(subtree.iter())
            .all(|(left, right)| left.name != right.name));
    }

    #[test]
    fn state_round_trips_atomically_and_rejects_corruption() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("capabilities.json");
        let mut state = CapabilityState::default();
        state.merge(policy_records(&plan())).unwrap();
        state.save_atomic(&path).unwrap();
        assert_eq!(CapabilityState::load(&path).unwrap(), state);

        std::fs::write(&path, b"{not-json").unwrap();
        assert_eq!(
            CapabilityState::load(&path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn state_rejects_identity_collision() {
        let record = policy_records(&plan()).remove(0);
        let mut conflicting = record.clone();
        conflicting.writable_root = std::env::current_dir().unwrap().join("different");
        let mut state = CapabilityState::default();
        state.merge([record]).unwrap();
        assert_eq!(
            state.merge([conflicting]).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
