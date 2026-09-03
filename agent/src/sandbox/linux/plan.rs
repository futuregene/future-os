use crate::sandbox::paths;
use crate::sandbox::rules::{
    Access, Decision, RuleLayer, RuleMatcherSnapshot, RuleSetSnapshot, RuleSnapshot,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

const MAX_GLOB_MATCHES: usize = 2_048;
const MAX_GLOB_NODES: usize = 100_000;
const MAX_GLOB_DEPTH: usize = 64;
const MAX_GLOB_SCAN_TIME: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LinuxSandboxPlanError {
    #[error("approval rule layer could not be loaded: {0}")]
    RuleLayerUnavailable(String),
    #[error("sandbox rule contains an unsafe path: {0}")]
    UnsafePath(PathBuf),
    #[error("sandbox rule matcher is unsupported: {0}")]
    UnsupportedMatcher(String),
    #[error("sandbox glob scan failed for {pattern}: {detail}")]
    GlobScan { pattern: String, detail: String },
    #[error("sandbox glob scan limit exceeded for {0}")]
    GlobLimit(String),
    #[error("sandbox mount plan exceeds the bounded helper request limit")]
    MountLimit,
    #[error("sandbox rule combination cannot be enforced safely at {0}")]
    UnsupportedAccessCombination(PathBuf),
    #[error("sandbox cannot reopen a protected path that does not exist: {0}")]
    MissingReopen(PathBuf),
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
                    validate_glob(pattern)?;
                    let matches = expand_glob(pattern)?;
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
                    if !path.exists() {
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
                if !path.exists() {
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
            if (blocks_read || blocks_write) && !path.exists() {
                missing_protected_paths.push(path.to_path_buf());
            }
            if blocks_read {
                unreadable_paths.push(path.to_path_buf());
            }
            if blocks_write {
                read_only_paths.push(path.to_path_buf());
            }
        }
    }
    Ok(())
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

pub(crate) fn expand_glob(pattern: &str) -> Result<Vec<PathBuf>, LinuxSandboxPlanError> {
    validate_glob(pattern)?;
    let root = glob_root(pattern);
    if !root.exists() {
        return Ok(Vec::new());
    }
    let started = Instant::now();
    let mut nodes = 0usize;
    let mut matches = Vec::new();
    for entry in walkdir::WalkDir::new(&root)
        .follow_links(false)
        .max_depth(MAX_GLOB_DEPTH)
    {
        if started.elapsed() > MAX_GLOB_SCAN_TIME {
            return Err(LinuxSandboxPlanError::GlobLimit(pattern.into()));
        }
        nodes += 1;
        if nodes > MAX_GLOB_NODES {
            return Err(LinuxSandboxPlanError::GlobLimit(pattern.into()));
        }
        let entry = entry.map_err(|error| LinuxSandboxPlanError::GlobScan {
            pattern: pattern.into(),
            detail: error.to_string(),
        })?;
        if entry.depth() == MAX_GLOB_DEPTH && entry.file_type().is_dir() {
            return Err(LinuxSandboxPlanError::GlobLimit(pattern.into()));
        }
        let lexical = entry.path();
        if glob_matches(pattern, lexical) {
            matches.push(lexical.to_path_buf());
            if entry.file_type().is_symlink() {
                let canonical = std::fs::canonicalize(lexical).map_err(|error| {
                    LinuxSandboxPlanError::GlobScan {
                        pattern: pattern.into(),
                        detail: error.to_string(),
                    }
                })?;
                validate_absolute(&canonical)?;
                matches.push(canonical);
            }
            if matches.len() > MAX_GLOB_MATCHES {
                return Err(LinuxSandboxPlanError::GlobLimit(pattern.into()));
            }
        }
    }
    normalize_exact(&mut matches);
    Ok(matches)
}

fn glob_root(pattern: &str) -> PathBuf {
    let mut root = PathBuf::new();
    for component in Path::new(pattern).components() {
        let text = component.as_os_str().to_string_lossy();
        if text.contains(['*', '?']) {
            break;
        }
        root.push(component.as_os_str());
    }
    if root.as_os_str().is_empty() {
        PathBuf::from("/")
    } else {
        root
    }
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
        for index in 0..=MAX_GLOB_MATCHES {
            std::fs::write(root.join("work").join(format!("{index}.pem")), "x").unwrap();
        }
        let pattern = root.join("work/*.pem").to_string_lossy().into_owned();
        assert!(matches!(
            expand_glob(&pattern),
            Err(LinuxSandboxPlanError::GlobLimit(_))
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
    fn reaching_glob_depth_limit_fails_closed() {
        let root = root();
        let mut deep = root.join("work");
        for _ in 0..MAX_GLOB_DEPTH {
            deep.push("d");
        }
        std::fs::create_dir_all(&deep).unwrap();
        let pattern = root.join("work/**/*.pem").to_string_lossy().into_owned();
        assert!(matches!(
            expand_glob(&pattern),
            Err(LinuxSandboxPlanError::GlobLimit(_))
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
