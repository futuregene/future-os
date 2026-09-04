use crate::sandbox::paths;
use crate::sandbox::rules::{
    Access, Decision, RuleLayer, RuleMatcherSnapshot, RuleSetSnapshot, RuleSnapshot,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LinuxSandboxPlanError {
    #[error("approval rule layer could not be loaded: {0}")]
    RuleLayerUnavailable(String),
    #[error("sandbox rule contains an unsafe path: {0}")]
    UnsafePath(PathBuf),
    #[error("sandbox rule matcher is unsupported: {0}")]
    UnsupportedMatcher(String),
    #[error(transparent)]
    GlobScan(Box<super::glob_scan::ScanError>),
    #[error("sandbox mount plan exceeds the bounded helper request limit")]
    MountLimit,
    #[error("sandbox rule combination cannot be enforced safely at {0}")]
    UnsupportedAccessCombination(PathBuf),
    #[error("sandbox cannot reopen a protected path that does not exist: {0}")]
    MissingReopen(PathBuf),
    #[error("sandbox path inspection failed at {path}: {detail}")]
    PathInspection { path: PathBuf, detail: String },
    #[error("sandbox policy could not be serialized: {0}")]
    PolicySerialization(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobSnapshot {
    pub pattern: String,
    pub matches: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinuxSandboxPlan {
    pub writable_roots: Vec<PathBuf>,
    pub read_only_paths: Vec<PathBuf>,
    pub unreadable_paths: Vec<PathBuf>,
    /// Writable narrow allows that must be mounted after broader protections.
    pub reopened_paths: Vec<PathBuf>,
    /// Read-only narrow allows that reopen reads while preserving a write deny.
    pub reopened_read_only_paths: Vec<PathBuf>,
    pub missing_protected_paths: Vec<PathBuf>,
    /// Glob rules are hard-enforced only for `glob_snapshots.matches` found
    /// before launch. These patterns require detection-only rescanning after
    /// the command because bwrap cannot protect future name matches.
    pub unsupported_dynamic_globs: Vec<String>,
    pub glob_snapshots: Vec<GlobSnapshot>,
    pub policy_digest: String,
}

impl LinuxSandboxPlan {
    /// Compile a deterministic mount policy. Glob expansion is a bounded,
    /// no-follow walk performed immediately before each command is prepared.
    pub fn compile(snapshot: &RuleSetSnapshot) -> Result<Self, LinuxSandboxPlanError> {
        Self::compile_with_cancel(snapshot, &|| false)
    }

    pub fn compile_with_cancel(
        snapshot: &RuleSetSnapshot,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Self, LinuxSandboxPlanError> {
        if let Some(error) = snapshot.resolution_errors.first() {
            return Err(LinuxSandboxPlanError::RuleLayerUnavailable(error.clone()));
        }
        validate_absolute(&snapshot.workspace)?;
        for root in &snapshot.temp_roots {
            validate_absolute(root)?;
        }

        let mut writable_roots = vec![snapshot.workspace.clone()];
        writable_roots.extend(snapshot.temp_roots.iter().cloned());
        let mut read_only_paths = Vec::new();
        let mut unreadable_paths = Vec::new();
        let mut reopened_paths = Vec::new();
        let mut reopened_read_only_paths = Vec::new();
        let mut missing_protected_paths = Vec::new();
        let mut unsupported_dynamic_globs = Vec::new();
        let mut glob_snapshots = Vec::new();

        // Preserve the exact RuleSet evaluation order. Both the layer index and
        // the rule's index inside that layer participate in first-match; Linux
        // mount generation must not let a later overlapping rule override it.
        let flattened: Vec<(usize, usize, RuleLayer, &RuleSnapshot)> = snapshot
            .layers
            .iter()
            .enumerate()
            .flat_map(|(priority, layer)| {
                layer
                    .rules
                    .iter()
                    .enumerate()
                    .map(move |(rule_index, rule)| (priority, rule_index, layer.layer, rule))
            })
            .collect();

        // Expand all patterns together, then replay the original rule order.
        // Grouping filesystem reads must not merge or reorder access decisions.
        let patterns: Vec<_> = flattened
            .iter()
            .filter_map(|(_, _, _, rule)| {
                if let RuleMatcherSnapshot::Glob { pattern } = &rule.matcher {
                    Some(pattern.clone())
                } else {
                    None
                }
            })
            .collect();
        let expanded = expand_globs(&patterns, "pre_launch", cancelled)?;

        for (priority, rule_index, _layer, rule) in &flattened {
            let paths = match &rule.matcher {
                RuleMatcherSnapshot::Subtree { lexical, canonical } => {
                    validate_absolute(lexical)?;
                    validate_absolute(canonical)?;
                    let mut paths = vec![lexical.clone(), canonical.clone()];
                    normalize_exact(&mut paths);
                    paths
                }
                RuleMatcherSnapshot::Glob { pattern } => {
                    let matches = expanded[pattern].clone();
                    unsupported_dynamic_globs.push(pattern.clone());
                    glob_snapshots.push(GlobSnapshot {
                        pattern: pattern.clone(),
                        matches: matches.clone(),
                    });
                    matches
                }
            };

            for path in paths {
                apply_rule(
                    &flattened,
                    *priority,
                    *rule_index,
                    rule,
                    &path,
                    &mut writable_roots,
                    &mut read_only_paths,
                    &mut unreadable_paths,
                    &mut reopened_paths,
                    &mut reopened_read_only_paths,
                    &mut missing_protected_paths,
                )?;
            }
        }

        normalize_roots(&mut writable_roots);
        normalize_exact(&mut read_only_paths);
        normalize_exact(&mut unreadable_paths);
        normalize_exact(&mut reopened_paths);
        normalize_exact(&mut reopened_read_only_paths);
        normalize_exact(&mut missing_protected_paths);
        // An opaque/missing mask also denies writes. Never bind the same
        // target read-only first: it either has no host inode at all or its
        // identity will be hidden by the final opaque mount.
        read_only_paths.retain(|path| {
            !unreadable_paths.contains(path) && !missing_protected_paths.contains(path)
        });
        unreadable_paths.retain(|path| !missing_protected_paths.contains(path));
        unsupported_dynamic_globs.sort();
        unsupported_dynamic_globs.dedup();
        glob_snapshots.sort_by(|a, b| a.pattern.cmp(&b.pattern));
        glob_snapshots.dedup_by(|a, b| a.pattern == b.pattern);

        let mount_count = writable_roots.len()
            + read_only_paths.len()
            + unreadable_paths.len()
            + reopened_paths.len()
            + reopened_read_only_paths.len()
            + missing_protected_paths.len();
        if mount_count > super::request::MAX_MOUNTS {
            return Err(LinuxSandboxPlanError::MountLimit);
        }

        let policy_digest = policy_digest(snapshot)?;

        Ok(Self {
            writable_roots,
            read_only_paths,
            unreadable_paths,
            reopened_paths,
            reopened_read_only_paths,
            missing_protected_paths,
            unsupported_dynamic_globs,
            glob_snapshots,
            policy_digest,
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_rule(
    flattened: &[(usize, usize, RuleLayer, &RuleSnapshot)],
    priority: usize,
    rule_index: usize,
    rule: &RuleSnapshot,
    path: &Path,
    writable_roots: &mut Vec<PathBuf>,
    read_only_paths: &mut Vec<PathBuf>,
    unreadable_paths: &mut Vec<PathBuf>,
    reopened_paths: &mut Vec<PathBuf>,
    reopened_read_only_paths: &mut Vec<PathBuf>,
    missing_protected_paths: &mut Vec<PathBuf>,
) -> Result<(), LinuxSandboxPlanError> {
    match rule.decision {
        Decision::Allow => {
            let read_blocked = effective_blocked(flattened, path, Access::Read);
            if rule.access.covers_write()
                && !matched_by_earlier_rule(flattened, priority, rule_index, path, Access::Write)
            {
                // A mount cannot safely grant write while continuing to deny
                // read on the same inode. Reject rather than silently widen.
                if read_blocked {
                    return Err(LinuxSandboxPlanError::UnsupportedAccessCombination(
                        path.to_path_buf(),
                    ));
                }
                if protected_by_later_rule(flattened, priority, rule_index, path, Access::Write) {
                    if !mount_path_exists(path)? {
                        return Err(LinuxSandboxPlanError::MissingReopen(path.to_path_buf()));
                    }
                    reopened_paths.push(path.to_path_buf());
                } else {
                    writable_roots.push(path.to_path_buf());
                }
            } else if rule.access.covers_read()
                && !matched_by_earlier_rule(flattened, priority, rule_index, path, Access::Read)
                && protected_by_later_rule(flattened, priority, rule_index, path, Access::Read)
            {
                if !mount_path_exists(path)? {
                    return Err(LinuxSandboxPlanError::MissingReopen(path.to_path_buf()));
                }
                reopened_read_only_paths.push(path.to_path_buf());
            }
        }
        Decision::Ask | Decision::Deny => {
            let blocks_read = rule.access.covers_read()
                && !matched_by_earlier_rule(flattened, priority, rule_index, path, Access::Read);
            let blocks_write = rule.access.covers_write()
                && !matched_by_earlier_rule(flattened, priority, rule_index, path, Access::Write);
            if !blocks_read && !blocks_write {
                return Ok(());
            }
            if !mount_path_exists(path)? {
                missing_protected_paths.push(path.to_path_buf());
            } else if blocks_read {
                unreadable_paths.push(path.to_path_buf());
            } else if blocks_write {
                read_only_paths.push(path.to_path_buf());
            }
        }
    }
    Ok(())
}

fn mount_path_exists(path: &Path) -> Result<bool, LinuxSandboxPlanError> {
    inspect_mount_path(
        path,
        |path| std::fs::symlink_metadata(path),
        |path| std::fs::metadata(path),
    )
}

fn inspect_mount_path(
    path: &Path,
    lstat: impl FnOnce(&Path) -> std::io::Result<std::fs::Metadata>,
    stat: impl FnOnce(&Path) -> std::io::Result<std::fs::Metadata>,
) -> Result<bool, LinuxSandboxPlanError> {
    let error = |error: std::io::Error| LinuxSandboxPlanError::PathInspection {
        path: path.into(),
        detail: error.to_string(),
    };
    match lstat(path) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(error(err)),
        Ok(metadata) if metadata.file_type().is_symlink() => {
            // A dangling/inaccessible link is not an absent mount target.
            // Do not turn EACCES/ELOOP/ENOTDIR or a link race into MissingProtected.
            stat(path).map(|_| true).map_err(error)
        }
        Ok(_) => Ok(true),
    }
}

pub(crate) fn policy_digest(snapshot: &RuleSetSnapshot) -> Result<String, LinuxSandboxPlanError> {
    let digest_input = serde_json::to_vec(snapshot)
        .map_err(|error| LinuxSandboxPlanError::PolicySerialization(error.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(digest_input)))
}

fn validate_absolute(path: &Path) -> Result<(), LinuxSandboxPlanError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(LinuxSandboxPlanError::UnsafePath(path.to_path_buf()));
    }
    Ok(())
}

fn validate_glob(pattern: &str) -> Result<(), LinuxSandboxPlanError> {
    validate_absolute(Path::new(pattern))?;
    if pattern.contains(['[', ']', '{', '}']) {
        return Err(LinuxSandboxPlanError::UnsupportedMatcher(pattern.into()));
    }
    Ok(())
}

pub(crate) fn expand_globs(
    patterns: &[String],
    phase: &'static str,
    cancelled: &dyn Fn() -> bool,
) -> Result<std::collections::BTreeMap<String, Vec<PathBuf>>, LinuxSandboxPlanError> {
    for pattern in patterns {
        validate_glob(pattern)?;
    }
    super::glob_scan::scan(patterns, phase, cancelled).map_err(LinuxSandboxPlanError::GlobScan)
}

#[cfg(test)]
fn expand_glob(pattern: &str) -> Result<Vec<PathBuf>, LinuxSandboxPlanError> {
    Ok(expand_globs(&[pattern.into()], "test", &|| false)?
        .remove(pattern)
        .unwrap())
}

fn glob_matches(pattern: &str, path: &Path) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let value: Vec<char> = path.to_string_lossy().chars().collect();
    let mut memo = std::collections::HashMap::new();
    fn matches_from(
        p: &[char],
        v: &[char],
        pi: usize,
        vi: usize,
        memo: &mut std::collections::HashMap<(usize, usize), bool>,
    ) -> bool {
        if let Some(result) = memo.get(&(pi, vi)) {
            return *result;
        }
        let result = if pi == p.len() {
            vi == v.len()
        } else if p[pi] == '*' {
            if pi + 1 < p.len() && p[pi + 1] == '*' {
                let next = if pi + 2 < p.len() && p[pi + 2] == '/' {
                    pi + 3
                } else {
                    pi + 2
                };
                matches_from(p, v, next, vi, memo)
                    || (vi < v.len() && matches_from(p, v, pi, vi + 1, memo))
            } else {
                matches_from(p, v, pi + 1, vi, memo)
                    || (vi < v.len() && v[vi] != '/' && matches_from(p, v, pi, vi + 1, memo))
            }
        } else if p[pi] == '?' {
            vi < v.len() && v[vi] != '/' && matches_from(p, v, pi + 1, vi + 1, memo)
        } else {
            vi < v.len() && p[pi] == v[vi] && matches_from(p, v, pi + 1, vi + 1, memo)
        };
        memo.insert((pi, vi), result);
        result
    }
    matches_from(&pattern, &value, 0, 0, &mut memo)
}

fn rule_blocks(rule: &RuleSnapshot, path: &Path, access: Access) -> bool {
    if rule.decision == Decision::Allow || !access_overlap(rule.access, access) {
        return false;
    }
    match &rule.matcher {
        RuleMatcherSnapshot::Subtree { lexical, canonical } => {
            paths::path_within(path, lexical) || paths::path_within(path, canonical)
        }
        RuleMatcherSnapshot::Glob { pattern } => glob_matches(pattern, path),
    }
}

fn effective_blocked(
    rules: &[(usize, usize, RuleLayer, &RuleSnapshot)],
    path: &Path,
    access: Access,
) -> bool {
    rules
        .iter()
        .find(|(_, _, _, rule)| rule_matches(rule, path, access))
        .is_some_and(|(_, _, _, rule)| rule.decision != Decision::Allow)
}

fn rule_matches(rule: &RuleSnapshot, path: &Path, access: Access) -> bool {
    access_overlap(rule.access, access)
        && match &rule.matcher {
            RuleMatcherSnapshot::Subtree { lexical, canonical } => {
                paths::path_within(path, lexical) || paths::path_within(path, canonical)
            }
            RuleMatcherSnapshot::Glob { pattern } => glob_matches(pattern, path),
        }
}

fn matched_by_earlier_rule(
    rules: &[(usize, usize, RuleLayer, &RuleSnapshot)],
    priority: usize,
    rule_index: usize,
    path: &Path,
    access: Access,
) -> bool {
    rules
        .iter()
        .any(|(candidate_priority, candidate_rule_index, _, rule)| {
            (*candidate_priority, *candidate_rule_index) < (priority, rule_index)
                && rule_matches(rule, path, access)
        })
}

fn protected_by_later_rule(
    rules: &[(usize, usize, RuleLayer, &RuleSnapshot)],
    priority: usize,
    rule_index: usize,
    path: &Path,
    access: Access,
) -> bool {
    rules
        .iter()
        .any(|(candidate_priority, candidate_rule_index, _, rule)| {
            (*candidate_priority, *candidate_rule_index) > (priority, rule_index)
                && rule_blocks(rule, path, access)
        })
}

fn access_overlap(left: Access, right: Access) -> bool {
    match right {
        Access::Read => left.covers_read(),
        Access::Write | Access::Both => left.covers_write(),
    }
}

fn normalize_roots(paths: &mut Vec<PathBuf>) {
    paths.sort_by_key(|path| path.components().count());
    let mut normalized: Vec<PathBuf> = Vec::new();
    for path in paths.drain(..) {
        if !normalized
            .iter()
            .any(|root| paths::path_within(&path, root))
        {
            normalized.push(path);
        }
    }
    *paths = normalized;
}

fn normalize_exact(paths: &mut Vec<PathBuf>) {
    paths.sort();
    paths.dedup();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::rules::{RuleLayerSnapshot, RuleSetSnapshot};

    fn root() -> PathBuf {
        let root = crate::test_support::unique_temp_path("linux-plan");
        std::fs::create_dir_all(root.join("work/vendor/ok")).unwrap();
        std::fs::create_dir_all(root.join("tmp/nested")).unwrap();
        root
    }
    fn subtree(root: &Path, path: &str, access: Access, decision: Decision) -> RuleSnapshot {
        let path = root.join(path);
        RuleSnapshot {
            matcher: RuleMatcherSnapshot::Subtree {
                lexical: path.clone(),
                canonical: path,
            },
            access,
            decision,
        }
    }
    fn snapshot(root: &Path, layers: Vec<RuleLayerSnapshot>) -> RuleSetSnapshot {
        RuleSetSnapshot {
            workspace: root.join("work"),
            temp_roots: vec![root.join("tmp"), root.join("tmp/nested")],
            layers,
            resolution_errors: vec![],
        }
    }

    #[test]
    fn roots_reopen_hard_deny_and_missing_are_compiled() {
        let root = root();
        let input = snapshot(
            &root,
            vec![
                RuleLayerSnapshot {
                    layer: RuleLayer::Override,
                    rules: vec![subtree(
                        &root,
                        "work/.future",
                        Access::Write,
                        Decision::Deny,
                    )],
                },
                RuleLayerSnapshot {
                    layer: RuleLayer::Session,
                    rules: vec![
                        subtree(&root, "work/vendor/ok", Access::Write, Decision::Allow),
                        subtree(&root, "work/.future/cache", Access::Write, Decision::Allow),
                    ],
                },
                RuleLayerSnapshot {
                    layer: RuleLayer::Workspace,
                    rules: vec![
                        subtree(&root, "work/vendor", Access::Write, Decision::Deny),
                        subtree(&root, "output", Access::Write, Decision::Allow),
                    ],
                },
            ],
        );
        let plan = LinuxSandboxPlan::compile(&input).unwrap();
        assert!(plan.writable_roots.contains(&root.join("output")));
        assert!(plan.reopened_paths.contains(&root.join("work/vendor/ok")));
        assert!(!plan
            .reopened_paths
            .contains(&root.join("work/.future/cache")));
        assert!(plan
            .missing_protected_paths
            .contains(&root.join("work/.future")));
        assert!(!plan.read_only_paths.contains(&root.join("work/.future")));
        assert!(!plan.unreadable_paths.contains(&root.join("work/.future")));
    }

    #[test]
    fn missing_and_existing_secret_masks_are_exclusive() {
        let root = root();
        for (access, name) in [
            (Access::Read, "read"),
            (Access::Write, "write"),
            (Access::Both, "both"),
        ] {
            let relative = format!("work/{name}");
            let path = root.join(&relative);
            let input = snapshot(
                &root,
                vec![RuleLayerSnapshot {
                    layer: RuleLayer::Guard,
                    rules: vec![subtree(&root, &relative, access, Decision::Ask)],
                }],
            );
            let missing = LinuxSandboxPlan::compile(&input).unwrap();
            assert_eq!(
                missing.missing_protected_paths.as_slice(),
                std::slice::from_ref(&path)
            );
            assert!(missing.read_only_paths.is_empty());
            assert!(missing.unreadable_paths.is_empty());
            std::fs::write(&path, "secret").unwrap();
            let existing = LinuxSandboxPlan::compile(&input).unwrap();
            assert!(existing.missing_protected_paths.is_empty());
            assert_eq!(
                existing.unreadable_paths.contains(&path),
                access.covers_read()
            );
            assert_eq!(
                existing.read_only_paths.contains(&path),
                !access.covers_read()
            );
        }
    }

    #[test]
    fn separate_read_and_write_rules_use_only_the_opaque_mask() {
        let root = root();
        std::fs::write(root.join("work/secret"), "secret").unwrap();
        let input = snapshot(
            &root,
            vec![RuleLayerSnapshot {
                layer: RuleLayer::Guard,
                rules: vec![
                    subtree(&root, "work/secret", Access::Write, Decision::Ask),
                    subtree(&root, "work/secret", Access::Read, Decision::Ask),
                ],
            }],
        );
        let plan = LinuxSandboxPlan::compile(&input).unwrap();
        assert!(plan.read_only_paths.is_empty());
        assert_eq!(plan.unreadable_paths, [root.join("work/secret")]);
    }

    #[test]
    fn path_inspection_errors_are_not_missing_paths() {
        let path = Path::new("/unreadable/secret");
        for kind in [
            std::io::ErrorKind::PermissionDenied,
            std::io::ErrorKind::NotADirectory,
        ] {
            let result = inspect_mount_path(
                path,
                |_| Err(std::io::Error::from(kind)),
                |_| unreachable!(),
            );
            assert!(matches!(
                result,
                Err(LinuxSandboxPlanError::PathInspection { .. })
            ));
        }
        assert!(!inspect_mount_path(
            path,
            |_| Err(std::io::Error::from(std::io::ErrorKind::NotFound)),
            |_| unreachable!()
        )
        .unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn dangling_secret_symlink_is_not_treated_as_absent() {
        let root = root();
        let path = root.join("work/secret");
        std::os::unix::fs::symlink(root.join("not-present"), &path).unwrap();
        assert!(matches!(
            mount_path_exists(&path),
            Err(LinuxSandboxPlanError::PathInspection { .. })
        ));
    }

    #[test]
    fn read_allow_reopens_read_only_and_write_only_reopen_fails_closed() {
        let root = root();
        let read_allow = snapshot(
            &root,
            vec![
                RuleLayerSnapshot {
                    layer: RuleLayer::Session,
                    rules: vec![subtree(
                        &root,
                        "work/vendor/ok",
                        Access::Read,
                        Decision::Allow,
                    )],
                },
                RuleLayerSnapshot {
                    layer: RuleLayer::Workspace,
                    rules: vec![subtree(&root, "work/vendor", Access::Both, Decision::Deny)],
                },
            ],
        );
        let plan = LinuxSandboxPlan::compile(&read_allow).unwrap();
        assert!(plan
            .reopened_read_only_paths
            .contains(&root.join("work/vendor/ok")));
        assert!(!plan.reopened_paths.contains(&root.join("work/vendor/ok")));

        let write_only_allow = snapshot(
            &root,
            vec![
                RuleLayerSnapshot {
                    layer: RuleLayer::Session,
                    rules: vec![subtree(
                        &root,
                        "work/vendor/ok",
                        Access::Write,
                        Decision::Allow,
                    )],
                },
                RuleLayerSnapshot {
                    layer: RuleLayer::Workspace,
                    rules: vec![subtree(&root, "work/vendor", Access::Read, Decision::Deny)],
                },
            ],
        );
        assert!(matches!(
            LinuxSandboxPlan::compile(&write_only_allow),
            Err(LinuxSandboxPlanError::UnsupportedAccessCombination(path))
                if path == root.join("work/vendor/ok")
        ));

        let missing_reopen = snapshot(
            &root,
            vec![
                RuleLayerSnapshot {
                    layer: RuleLayer::Session,
                    rules: vec![subtree(
                        &root,
                        "work/vendor/missing",
                        Access::Both,
                        Decision::Allow,
                    )],
                },
                RuleLayerSnapshot {
                    layer: RuleLayer::Workspace,
                    rules: vec![subtree(&root, "work/vendor", Access::Both, Decision::Deny)],
                },
            ],
        );
        assert!(matches!(
            LinuxSandboxPlan::compile(&missing_reopen),
            Err(LinuxSandboxPlanError::MissingReopen(path))
                if path == root.join("work/vendor/missing")
        ));
    }

    #[cfg(unix)]
    #[test]
    fn glob_snapshot_includes_lexical_symlink_and_canonical_target() {
        let root = root();
        let real = root.join("outside/secret.pem");
        std::fs::create_dir_all(real.parent().unwrap()).unwrap();
        std::fs::write(&real, "secret").unwrap();
        std::os::unix::fs::symlink(&real, root.join("work/link.pem")).unwrap();
        let pattern = root.join("work/*.pem").to_string_lossy().into_owned();
        let input = snapshot(
            &root,
            vec![RuleLayerSnapshot {
                layer: RuleLayer::Guard,
                rules: vec![RuleSnapshot {
                    matcher: RuleMatcherSnapshot::Glob {
                        pattern: pattern.clone(),
                    },
                    access: Access::Both,
                    decision: Decision::Ask,
                }],
            }],
        );
        let plan = LinuxSandboxPlan::compile(&input).unwrap();
        assert!(plan.unreadable_paths.contains(&root.join("work/link.pem")));
        assert!(plan
            .unreadable_paths
            .contains(&std::fs::canonicalize(real).unwrap()));
        assert_eq!(plan.glob_snapshots[0].pattern, pattern);
        assert_eq!(plan.unsupported_dynamic_globs, [pattern]);
    }

    #[test]
    fn narrow_glob_allow_shadows_the_lower_glob_mount() {
        let root = root();
        let allowed = root.join("work/vendor/ok.pem");
        std::fs::write(&allowed, "ok").unwrap();
        let allow_pattern = allowed.to_string_lossy().into_owned();
        let deny_pattern = root
            .join("work/vendor/*.pem")
            .to_string_lossy()
            .into_owned();
        let input = snapshot(
            &root,
            vec![
                RuleLayerSnapshot {
                    layer: RuleLayer::Session,
                    rules: vec![RuleSnapshot {
                        matcher: RuleMatcherSnapshot::Glob {
                            pattern: allow_pattern,
                        },
                        access: Access::Write,
                        decision: Decision::Allow,
                    }],
                },
                RuleLayerSnapshot {
                    layer: RuleLayer::Workspace,
                    rules: vec![RuleSnapshot {
                        matcher: RuleMatcherSnapshot::Glob {
                            pattern: deny_pattern,
                        },
                        access: Access::Write,
                        decision: Decision::Deny,
                    }],
                },
            ],
        );
        let plan = LinuxSandboxPlan::compile(&input).unwrap();
        assert!(!plan.read_only_paths.contains(&allowed));
        assert!(plan.reopened_paths.contains(&allowed));
    }

    #[test]
    fn glob_match_limit_fails_closed() {
        let root = root();
        for index in 0..=super::super::glob_scan::MAX_MATCHES {
            std::fs::write(root.join("work").join(format!("{index}.pem")), "x").unwrap();
        }
        let pattern = root.join("work/*.pem").to_string_lossy().into_owned();
        assert!(matches!(
            expand_glob(&pattern),
            Err(LinuxSandboxPlanError::GlobScan(error)) if error.code == "glob_scan_match_limit"
        ));
    }

    #[test]
    fn malformed_layer_matcher_and_relative_path_fail_closed() {
        let root = root();
        let mut broken = snapshot(&root, vec![]);
        broken.resolution_errors.push("malformed rules".into());
        assert!(matches!(
            LinuxSandboxPlan::compile(&broken),
            Err(LinuxSandboxPlanError::RuleLayerUnavailable(_))
        ));
        let unsupported = snapshot(
            &root,
            vec![RuleLayerSnapshot {
                layer: RuleLayer::User,
                rules: vec![RuleSnapshot {
                    matcher: RuleMatcherSnapshot::Glob {
                        pattern: root.join("work/[ab]").to_string_lossy().into_owned(),
                    },
                    access: Access::Read,
                    decision: Decision::Deny,
                }],
            }],
        );
        assert!(matches!(
            LinuxSandboxPlan::compile(&unsupported),
            Err(LinuxSandboxPlanError::UnsupportedMatcher(_))
        ));
        let relative = snapshot(
            &root,
            vec![RuleLayerSnapshot {
                layer: RuleLayer::User,
                rules: vec![RuleSnapshot {
                    matcher: RuleMatcherSnapshot::Subtree {
                        lexical: "relative".into(),
                        canonical: "relative".into(),
                    },
                    access: Access::Write,
                    decision: Decision::Allow,
                }],
            }],
        );
        assert!(matches!(
            LinuxSandboxPlan::compile(&relative),
            Err(LinuxSandboxPlanError::UnsafePath(_))
        ));
    }

    #[test]
    fn glob_matcher_supports_recursive_and_segment_wildcards() {
        assert!(glob_matches("/a/**/*.pem", Path::new("/a/x/y.pem")));
        assert!(glob_matches("/a/**/*.pem", Path::new("/a/y.pem")));
        assert!(!glob_matches("/a/*.pem", Path::new("/a/x/y.pem")));
        assert!(glob_matches("/a/?.pem", Path::new("/a/x.pem")));
        assert!(glob_matches("/a/?.pem", Path::new("/a/密.pem")));
    }

    #[test]
    fn compiled_scanner_matches_existing_linux_glob_semantics() {
        for pattern in [
            "/a/*.pem",
            "/a/**/x?.pem",
            "/a/prefix**suffix",
            "/a/***/x",
            "/a/密?.pem",
            "/a/[x].pem",
            "/a/*/x.pem",
        ] {
            let compiled = super::super::glob_scan::compile_matcher(pattern).unwrap();
            for path in [
                "/a/x.pem",
                "/a/xx.pem",
                "/a/d/x1.pem",
                "/a/prefix/x/suffix",
                "/a/密钥.pem",
                "/a/[x].pem",
                "/a/d\n/x.pem",
                "/a/x.pem\n",
                "/a//x.pem",
            ] {
                assert_eq!(
                    compiled.is_match(path),
                    glob_matches(pattern, Path::new(path)),
                    "pattern={pattern}, path={path}"
                );
            }
        }
    }

    #[test]
    fn reaching_glob_depth_limit_fails_closed() {
        let root = root();
        let mut deep = root.join("work");
        for _ in 0..super::super::glob_scan::MAX_DEPTH {
            deep.push("d");
        }
        std::fs::create_dir_all(&deep).unwrap();
        let pattern = root.join("work/**/*.pem").to_string_lossy().into_owned();
        assert!(matches!(
            expand_glob(&pattern),
            Err(LinuxSandboxPlanError::GlobScan(error)) if error.code == "glob_scan_depth_limit"
        ));
    }

    #[test]
    fn same_layer_first_match_wins_for_overlapping_write_rules() {
        let root = root();
        let parent = root.join("outside");
        let child = parent.join("child");
        std::fs::create_dir_all(&child).unwrap();

        let deny_then_allow = snapshot(
            &root,
            vec![RuleLayerSnapshot {
                layer: RuleLayer::Workspace,
                rules: vec![
                    subtree(&root, "outside", Access::Write, Decision::Deny),
                    subtree(&root, "outside/child", Access::Write, Decision::Allow),
                ],
            }],
        );
        let plan = LinuxSandboxPlan::compile(&deny_then_allow).unwrap();
        assert!(plan.read_only_paths.contains(&parent));
        assert!(!plan.writable_roots.contains(&child));
        assert!(!plan.reopened_paths.contains(&child));

        let allow_then_deny = snapshot(
            &root,
            vec![RuleLayerSnapshot {
                layer: RuleLayer::Workspace,
                rules: vec![
                    subtree(&root, "outside", Access::Write, Decision::Allow),
                    subtree(&root, "outside/child", Access::Write, Decision::Deny),
                ],
            }],
        );
        let plan = LinuxSandboxPlan::compile(&allow_then_deny).unwrap();
        assert!(plan.writable_roots.contains(&parent));
        assert!(!plan.read_only_paths.contains(&child));

        let narrow_allow_then_broad_deny = snapshot(
            &root,
            vec![RuleLayerSnapshot {
                layer: RuleLayer::Workspace,
                rules: vec![
                    subtree(&root, "outside/child", Access::Write, Decision::Allow),
                    subtree(&root, "outside", Access::Write, Decision::Deny),
                ],
            }],
        );
        let plan = LinuxSandboxPlan::compile(&narrow_allow_then_broad_deny).unwrap();
        assert!(plan.read_only_paths.contains(&parent));
        assert!(plan.reopened_paths.contains(&child));
    }
}
