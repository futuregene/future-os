use crate::sandbox::paths;
use crate::sandbox::rules::{
    Access, Decision, RuleLayer, RuleMatcherSnapshot, RuleSetSnapshot, RuleSnapshot,
};
use serde::Serialize;
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
    #[error("sandbox policy could not be serialized: {0}")]
    PolicySerialization(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinuxSandboxPlan {
    pub writable_roots: Vec<PathBuf>,
    pub read_only_paths: Vec<PathBuf>,
    pub unreadable_paths: Vec<PathBuf>,
    pub reopened_paths: Vec<PathBuf>,
    pub missing_protected_paths: Vec<PathBuf>,
    pub unsupported_dynamic_globs: Vec<String>,
    pub policy_digest: String,
}

impl LinuxSandboxPlan {
    /// Compile a deterministic mount policy without spawning a process.
    /// `exists` is injected so all policy behavior is cross-platform testable.
    pub fn compile(
        snapshot: &RuleSetSnapshot,
        exists: &dyn Fn(&Path) -> bool,
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
        let mut missing_protected_paths = Vec::new();
        let mut unsupported_dynamic_globs = Vec::new();

        let flattened: Vec<(usize, RuleLayer, &RuleSnapshot)> = snapshot
            .layers
            .iter()
            .enumerate()
            .flat_map(|(priority, layer)| {
                layer
                    .rules
                    .iter()
                    .map(move |rule| (priority, layer.layer, rule))
            })
            .collect();

        for (priority, _layer, rule) in &flattened {
            let path = match &rule.matcher {
                RuleMatcherSnapshot::Subtree { canonical } => {
                    validate_absolute(canonical)?;
                    canonical
                }
                RuleMatcherSnapshot::Glob { pattern } => {
                    validate_glob(pattern)?;
                    unsupported_dynamic_globs.push(pattern.clone());
                    continue;
                }
            };

            match rule.decision {
                Decision::Allow => {
                    if rule.access.covers_write()
                        && !blocked_by_higher_priority(&flattened, *priority, path, Access::Write)
                    {
                        if protected_by_lower_priority(&flattened, *priority, path, Access::Write) {
                            reopened_paths.push(path.clone());
                        } else {
                            writable_roots.push(path.clone());
                        }
                    }
                }
                Decision::Ask | Decision::Deny => {
                    if !exists(path) {
                        missing_protected_paths.push(path.clone());
                    }
                    if rule.access.covers_read() {
                        unreadable_paths.push(path.clone());
                    }
                    if rule.access.covers_write() {
                        read_only_paths.push(path.clone());
                    }
                }
            }
        }

        normalize_roots(&mut writable_roots);
        normalize_exact(&mut read_only_paths);
        normalize_exact(&mut unreadable_paths);
        normalize_exact(&mut reopened_paths);
        normalize_exact(&mut missing_protected_paths);
        unsupported_dynamic_globs.sort();
        unsupported_dynamic_globs.dedup();

        let digest_input = serde_json::to_vec(snapshot)
            .map_err(|error| LinuxSandboxPlanError::PolicySerialization(error.to_string()))?;
        let policy_digest = format!("{:x}", Sha256::digest(digest_input));

        Ok(Self {
            writable_roots,
            read_only_paths,
            unreadable_paths,
            reopened_paths,
            missing_protected_paths,
            unsupported_dynamic_globs,
            policy_digest,
        })
    }
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
    let path = Path::new(pattern);
    validate_absolute(path)?;
    if pattern.contains('[')
        || pattern.contains(']')
        || pattern.contains('{')
        || pattern.contains('}')
    {
        return Err(LinuxSandboxPlanError::UnsupportedMatcher(pattern.into()));
    }
    Ok(())
}

fn blocks(rule: &RuleSnapshot, path: &Path, access: Access) -> bool {
    if rule.decision == Decision::Allow || !access_overlap(rule.access, access) {
        return false;
    }
    match &rule.matcher {
        RuleMatcherSnapshot::Subtree { canonical } => paths::path_within(path, canonical),
        RuleMatcherSnapshot::Glob { .. } => false,
    }
}

fn blocked_by_higher_priority(
    rules: &[(usize, RuleLayer, &RuleSnapshot)],
    priority: usize,
    path: &Path,
    access: Access,
) -> bool {
    rules.iter().any(|(candidate_priority, _, rule)| {
        *candidate_priority < priority && blocks(rule, path, access)
    })
}

fn protected_by_lower_priority(
    rules: &[(usize, RuleLayer, &RuleSnapshot)],
    priority: usize,
    path: &Path,
    access: Access,
) -> bool {
    rules.iter().any(|(candidate_priority, _, rule)| {
        *candidate_priority > priority && blocks(rule, path, access)
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

    fn absolute(relative: &str) -> PathBuf {
        let root = if cfg!(windows) {
            PathBuf::from(r"C:\future-plan-test")
        } else {
            PathBuf::from("/future-plan-test")
        };
        relative.split('/').fold(root, |path, part| path.join(part))
    }

    fn subtree(path: &str, access: Access, decision: Decision) -> RuleSnapshot {
        RuleSnapshot {
            matcher: RuleMatcherSnapshot::Subtree {
                canonical: absolute(path),
            },
            access,
            decision,
        }
    }

    fn snapshot(layers: Vec<RuleLayerSnapshot>) -> RuleSetSnapshot {
        RuleSetSnapshot {
            workspace: absolute("work"),
            temp_roots: vec![absolute("tmp"), absolute("tmp/nested")],
            layers,
            resolution_errors: vec![],
        }
    }

    #[test]
    fn fallback_roots_are_deduplicated_and_external_allow_is_writable() {
        let input = snapshot(vec![RuleLayerSnapshot {
            layer: RuleLayer::Workspace,
            rules: vec![subtree("output", Access::Write, Decision::Allow)],
        }]);
        let plan = LinuxSandboxPlan::compile(&input, &|_| true).unwrap();
        assert_eq!(
            plan.writable_roots,
            vec![absolute("work"), absolute("tmp"), absolute("output")]
        );
    }

    #[test]
    fn narrow_high_priority_allow_reopens_lower_protection() {
        let input = snapshot(vec![
            RuleLayerSnapshot {
                layer: RuleLayer::Session,
                rules: vec![subtree("work/vendor/ok", Access::Write, Decision::Allow)],
            },
            RuleLayerSnapshot {
                layer: RuleLayer::Workspace,
                rules: vec![subtree("work/vendor", Access::Write, Decision::Deny)],
            },
        ]);
        let plan = LinuxSandboxPlan::compile(&input, &|_| true).unwrap();
        assert_eq!(plan.read_only_paths, vec![absolute("work/vendor")]);
        assert_eq!(plan.reopened_paths, vec![absolute("work/vendor/ok")]);
    }

    #[test]
    fn hard_deny_cannot_be_reopened_by_lower_allow() {
        let input = snapshot(vec![
            RuleLayerSnapshot {
                layer: RuleLayer::Override,
                rules: vec![subtree("work/.future", Access::Write, Decision::Deny)],
            },
            RuleLayerSnapshot {
                layer: RuleLayer::Session,
                rules: vec![subtree(
                    "work/.future/cache",
                    Access::Write,
                    Decision::Allow,
                )],
            },
        ]);
        let plan = LinuxSandboxPlan::compile(&input, &|_| true).unwrap();
        assert!(plan.reopened_paths.is_empty());
        assert!(!plan
            .writable_roots
            .contains(&absolute("work/.future/cache")));
    }

    #[test]
    fn ask_and_deny_compile_to_protection_and_missing_targets() {
        let input = snapshot(vec![RuleLayerSnapshot {
            layer: RuleLayer::Guard,
            rules: vec![
                subtree("work/.env", Access::Both, Decision::Ask),
                subtree("secret", Access::Read, Decision::Deny),
            ],
        }]);
        let secret = absolute("secret");
        let plan = LinuxSandboxPlan::compile(&input, &|path| path == secret).unwrap();
        assert!(plan.read_only_paths.contains(&absolute("work/.env")));
        assert_eq!(
            plan.unreadable_paths,
            vec![absolute("secret"), absolute("work/.env")]
        );
        assert_eq!(plan.missing_protected_paths, vec![absolute("work/.env")]);
    }

    #[test]
    fn supported_dynamic_globs_are_explicit_and_digest_is_deterministic() {
        let input = snapshot(vec![RuleLayerSnapshot {
            layer: RuleLayer::Guard,
            rules: vec![RuleSnapshot {
                matcher: RuleMatcherSnapshot::Glob {
                    pattern: absolute("work/**/*.pem").to_string_lossy().into_owned(),
                },
                access: Access::Both,
                decision: Decision::Ask,
            }],
        }]);
        let first = LinuxSandboxPlan::compile(&input, &|_| true).unwrap();
        let second = LinuxSandboxPlan::compile(&input, &|_| false).unwrap();
        assert_eq!(
            first.unsupported_dynamic_globs,
            vec![absolute("work/**/*.pem").to_string_lossy().into_owned()]
        );
        assert_eq!(first.policy_digest, second.policy_digest);
    }

    #[test]
    fn broken_layers_and_unsupported_matchers_fail_closed() {
        let mut broken = snapshot(vec![]);
        broken
            .resolution_errors
            .push("malformed workspace rules".into());
        assert!(matches!(
            LinuxSandboxPlan::compile(&broken, &|_| true),
            Err(LinuxSandboxPlanError::RuleLayerUnavailable(_))
        ));

        let unsupported = snapshot(vec![RuleLayerSnapshot {
            layer: RuleLayer::User,
            rules: vec![RuleSnapshot {
                matcher: RuleMatcherSnapshot::Glob {
                    pattern: absolute("work/[ab]").to_string_lossy().into_owned(),
                },
                access: Access::Read,
                decision: Decision::Deny,
            }],
        }]);
        assert!(matches!(
            LinuxSandboxPlan::compile(&unsupported, &|_| true),
            Err(LinuxSandboxPlanError::UnsupportedMatcher(_))
        ));
    }

    #[test]
    fn unsafe_relative_paths_fail_closed() {
        let input = snapshot(vec![RuleLayerSnapshot {
            layer: RuleLayer::User,
            rules: vec![RuleSnapshot {
                matcher: RuleMatcherSnapshot::Subtree {
                    canonical: PathBuf::from("relative/path"),
                },
                access: Access::Write,
                decision: Decision::Allow,
            }],
        }]);
        assert!(matches!(
            LinuxSandboxPlan::compile(&input, &|_| true),
            Err(LinuxSandboxPlanError::UnsafePath(_))
        ));
    }
}
