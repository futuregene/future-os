//! Startup snapshots and post-command detection share this no-follow scanner.
//! Unlike a user search, security scans must include hidden and ignored entries.
//! Groups share directory reads, but retain per-pattern results for rule ordering.
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub(super) const MAX_MATCHES: usize = 2_048;
pub(super) const MAX_DEPTH: usize = 64;
const MAX_PATTERNS: usize = 256;
const MAX_RESULT_BYTES: usize = 4 * 1024 * 1024;
const SCAN_TIME: Duration = Duration::from_secs(30);
type Matches = BTreeMap<String, Vec<PathBuf>>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: phase={phase}, root={root}, pattern={pattern}, visited={visited}, matches={matches}, elapsed_ms={elapsed_ms}; {detail}")]
pub struct ScanError {
    pub code: &'static str,
    pub phase: &'static str,
    pub root: PathBuf,
    pub pattern: String,
    pub visited: usize,
    pub matches: usize,
    pub elapsed_ms: u128,
    pub detail: String,
}

struct Budget {
    start: Instant,
    duration: Duration,
    phase: &'static str,
    visited: usize,
    matches: usize,
    bytes: usize,
}

impl Budget {
    fn error(
        &self,
        code: &'static str,
        root: &Path,
        pattern: &str,
        detail: impl Into<String>,
    ) -> Box<ScanError> {
        Box::new(ScanError {
            code,
            phase: self.phase,
            root: root.into(),
            pattern: pattern.into(),
            visited: self.visited,
            matches: self.matches,
            elapsed_ms: self.start.elapsed().as_millis(),
            detail: detail.into(),
        })
    }

    // Cooperative deadline: checks surround directory I/O and matching. A
    // stalled kernel filesystem call is not preempted by this wall-clock budget.
    fn check(
        &self,
        root: &Path,
        pattern: &str,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), Box<ScanError>> {
        if cancelled() {
            return Err(self.error(
                "glob_scan_cancelled",
                root,
                pattern,
                "command aborted before execution",
            ));
        }
        if self.start.elapsed() >= self.duration {
            return Err(self.error(
                "glob_scan_timeout",
                root,
                pattern,
                format!("shared_scan_budget_ms={}", self.duration.as_millis()),
            ));
        }
        Ok(())
    }
}

struct Pattern {
    text: String,
    matcher: Regex,
    // Prefixes before the first ** let finite patterns prune irrelevant
    // directories. ** inside a segment is recursive too in our existing grammar.
    prefixes: Vec<Regex>,
    recursive: bool,
    depth: usize,
}

impl Pattern {
    fn compile(text: &str, root: &Path) -> Result<Self, regex::Error> {
        let relative = Path::new(text).strip_prefix(root).expect("static prefix");
        let parts: Vec<_> = relative.iter().map(|p| p.to_string_lossy()).collect();
        let recursive = parts.iter().any(|part| part.contains("**"));
        let mut prefix = PathBuf::new();
        let mut prefixes = Vec::new();
        for part in &parts {
            if part.contains("**") {
                break;
            }
            prefix.push(part.as_ref());
            prefixes.push(compile_matcher(&prefix.to_string_lossy())?);
        }
        Ok(Self {
            text: text.into(),
            matcher: compile_matcher(text)?,
            prefixes,
            recursive,
            depth: parts.len(),
        })
    }

    fn can_descend(&self, relative: &Path, depth: usize) -> bool {
        if !self.recursive && depth >= self.depth {
            return false;
        }
        if depth == 0 {
            return true;
        }
        let checked = depth.min(self.prefixes.len());
        if checked == 0 {
            return true;
        }
        let prefix: PathBuf = relative.iter().take(checked).collect();
        self.prefixes[checked - 1].is_match(&prefix.to_string_lossy())
    }
}

/// Preserve the Linux matcher's existing grammar, including **/ consuming zero
/// characters. Brackets/braces/backslashes remain literals, not globset syntax.
pub(super) fn compile_matcher(pattern: &str) -> Result<Regex, regex::Error> {
    let mut expression = String::from("\\A");
    let mut chars = pattern.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '*' if chars.peek() == Some(&'*') => {
                chars.next();
                if chars.peek() == Some(&'/') {
                    chars.next();
                }
                expression.push_str("(?s:.*)");
            }
            '*' => expression.push_str("[^/]*"),
            '?' => expression.push_str("[^/]"),
            _ => expression.push_str(&regex::escape(&ch.to_string())),
        }
    }
    expression.push_str("\\z");
    Regex::new(&expression)
}

fn scan_root(pattern: &str) -> PathBuf {
    Path::new(pattern)
        .components()
        .take_while(|c| !c.as_os_str().to_string_lossy().contains(['*', '?']))
        .collect()
}

pub(super) fn scan(
    patterns: &[String],
    phase: &'static str,
    cancelled: &dyn Fn() -> bool,
) -> Result<Matches, Box<ScanError>> {
    scan_with_budget(patterns, phase, cancelled, SCAN_TIME).map(|(results, _)| results)
}

fn scan_with_budget(
    patterns: &[String],
    phase: &'static str,
    cancelled: &dyn Fn() -> bool,
    duration: Duration,
) -> Result<(Matches, usize), Box<ScanError>> {
    let mut budget = Budget {
        start: Instant::now(),
        duration,
        phase,
        visited: 0,
        matches: 0,
        bytes: 0,
    };
    let mut groups: BTreeMap<PathBuf, Vec<Pattern>> = BTreeMap::new();
    let mut results: BTreeMap<String, BTreeSet<PathBuf>> = BTreeMap::new();
    let mut unique_matches = BTreeSet::new();
    for text in patterns {
        let root = scan_root(text);
        budget.check(&root, text, cancelled)?;
        if results.contains_key(text) {
            continue;
        }
        if results.len() >= MAX_PATTERNS {
            return Err(budget.error(
                "glob_scan_pattern_limit",
                &root,
                text,
                format!("limit={MAX_PATTERNS}"),
            ));
        }
        let compiled = Pattern::compile(text, &root).map_err(|error| {
            budget.error("glob_scan_pattern_invalid", &root, text, error.to_string())
        })?;
        groups.entry(root).or_default().push(compiled);
        results.insert(text.clone(), BTreeSet::new());
    }
    for (root, patterns) in groups {
        // A group-level I/O/deadline failure is not attributable to the first
        // rule alone (e.g. .env.* sharing a walk with **/*.pem).
        let group_context = format!(
            "{} [shared root: {} rules]",
            patterns[0].text,
            patterns.len()
        );
        let context = group_context.as_str();
        budget.check(&root, context, cancelled)?;
        // NotFound is an absent static root. Permission and other errors must
        // not become a successful empty snapshot through Path::exists().
        match std::fs::symlink_metadata(&root) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(budget.error("glob_scan_io_error", &root, context, error.to_string()))
            }
            Ok(_) => {}
        }
        let mut walker = walkdir::WalkDir::new(&root)
            .follow_links(false)
            .follow_root_links(false)
            .into_iter();
        loop {
            budget.check(&root, context, cancelled)?;
            let Some(entry) = walker.next() else {
                break;
            };
            let entry = entry.map_err(|error| {
                budget.error("glob_scan_io_error", &root, context, error.to_string())
            })?;
            budget.visited += 1;
            budget.check(&root, context, cancelled)?;
            // A changed static root symlink is not safe to silently skip.
            if entry.depth() == 0 && entry.file_type().is_symlink() {
                return Err(budget.error(
                    "glob_scan_io_error",
                    &root,
                    context,
                    "static scan root became a symlink",
                ));
            }
            for pattern in &patterns {
                budget.check(&root, &pattern.text, cancelled)?;
                // A shallow rule does not participate in deeper matching just
                // because a recursive sibling needs to traverse that subtree.
                if !pattern.recursive && entry.depth() != pattern.depth {
                    continue;
                }
                if !pattern.matcher.is_match(&entry.path().to_string_lossy()) {
                    continue;
                }
                let mut paths = vec![entry.path().to_path_buf()];
                if entry.file_type().is_symlink() {
                    paths.push(std::fs::canonicalize(entry.path()).map_err(|error| {
                        budget.error(
                            "glob_scan_io_error",
                            &root,
                            &pattern.text,
                            error.to_string(),
                        )
                    })?);
                }
                for path in paths {
                    let matches = results.get_mut(&pattern.text).expect("registered pattern");
                    if matches.insert(path.clone()) {
                        // Bound stored associations as well as unique mounts:
                        // overlapping patterns otherwise multiply snapshot memory.
                        budget.bytes += path.as_os_str().len() + pattern.text.len();
                        unique_matches.insert(path);
                        budget.matches = unique_matches.len();
                        if budget.matches > MAX_MATCHES || budget.bytes > MAX_RESULT_BYTES {
                            let code = if budget.matches > MAX_MATCHES {
                                "glob_scan_match_limit"
                            } else {
                                "glob_scan_result_bytes_limit"
                            };
                            return Err(budget.error(code, &root, &pattern.text, format!("unique_limit={MAX_MATCHES}, snapshot_bytes={}, byte_limit={MAX_RESULT_BYTES}", budget.bytes)));
                        }
                    }
                }
            }
            if entry.file_type().is_dir() {
                let relative = entry.path().strip_prefix(&root).expect("walk root");
                let descend = patterns
                    .iter()
                    .find(|pattern| pattern.can_descend(relative, entry.depth()));
                if let Some(pattern) = descend {
                    if entry.depth() >= MAX_DEPTH {
                        return Err(budget.error(
                            "glob_scan_depth_limit",
                            &root,
                            &pattern.text,
                            format!("depth={}, limit={MAX_DEPTH}", entry.depth()),
                        ));
                    }
                } else {
                    walker.skip_current_dir();
                }
            }
        }
        budget.check(&root, context, cancelled)?;
    }
    tracing::debug!(
        phase,
        visited = budget.visited,
        matches = budget.matches,
        elapsed_ms = budget.start.elapsed().as_millis(),
        "Linux sandbox glob scan completed"
    );
    Ok((
        results
            .into_iter()
            .map(|(pattern, paths)| (pattern, paths.into_iter().collect()))
            .collect(),
        budget.visited,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finite_patterns_prune_without_depth_errors() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("node_modules/a/b/c")).unwrap();
        std::fs::write(dir.path().join(".env.local"), "secret").unwrap();
        let pattern = dir.path().join(".env.*").to_string_lossy().into_owned();
        let (found, visited) =
            scan_with_budget(std::slice::from_ref(&pattern), "test", &|| false, SCAN_TIME).unwrap();
        assert_eq!(visited, 3); // root, node_modules, .env.local; no descendants
        assert_eq!(found[&pattern], [dir.path().join(".env.local")]);
    }

    #[test]
    fn grouped_patterns_share_walk_and_include_ignored_and_hidden_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".hidden")).unwrap();
        std::fs::write(dir.path().join(".gitignore"), ".hidden\n").unwrap();
        std::fs::write(dir.path().join(".hidden/a.pem"), "secret").unwrap();
        std::fs::write(dir.path().join(".hidden/a.key"), "secret").unwrap();
        let patterns: Vec<_> = ["**/*.pem", "**/*.key", ".env.*"]
            .iter()
            .map(|p| dir.path().join(p).to_string_lossy().into_owned())
            .collect();
        let (found, visited) = scan_with_budget(&patterns, "test", &|| false, SCAN_TIME).unwrap();
        assert_eq!(visited, 5);
        assert_eq!(found[&patterns[0]].len(), 1);
        assert_eq!(found[&patterns[1]].len(), 1);
        assert!(found[&patterns[2]].is_empty());
    }

    #[test]
    fn timeout_and_cancellation_have_distinct_diagnostics() {
        let patterns = vec!["/unused/.env.*".into()];
        let timeout =
            scan_with_budget(&patterns, "pre_launch", &|| false, Duration::ZERO).unwrap_err();
        assert_eq!(timeout.code, "glob_scan_timeout");
        assert_eq!(timeout.phase, "pre_launch");
        let cancelled = scan(&patterns, "pre_launch", &|| true).unwrap_err();
        assert_eq!(cancelled.code, "glob_scan_cancelled");
    }

    #[test]
    fn abort_during_walk_discards_partial_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..20 {
            std::fs::write(dir.path().join(format!("{i}.pem")), "").unwrap();
        }
        let pattern = dir.path().join("*.pem").to_string_lossy().into_owned();
        let checks = std::cell::Cell::new(0);
        let error = scan(&[pattern], "pre_launch", &|| {
            checks.set(checks.get() + 1);
            checks.get() > 12
        })
        .unwrap_err();
        assert_eq!(error.code, "glob_scan_cancelled");
        assert!(error.visited > 0);
        assert!(error.visited < 21);
    }

    #[test]
    fn semantic_depth_boundary_is_not_resource_exhaustion() {
        let dir = tempfile::tempdir().unwrap();
        let mut directory = dir.path().to_path_buf();
        let mut pattern = dir.path().to_path_buf();
        for _ in 0..MAX_DEPTH {
            directory.push("d");
            pattern.push("*");
        }
        std::fs::create_dir_all(directory.join("irrelevant/descendant")).unwrap();
        let pattern = pattern.to_string_lossy().into_owned();
        assert_eq!(
            scan(std::slice::from_ref(&pattern), "test", &|| false).unwrap()[&pattern],
            [directory]
        );
    }

    #[test]
    fn finite_prefix_prunes_unrelated_branches() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("pkg-a/secrets")).unwrap();
        std::fs::create_dir_all(dir.path().join("other/more/deep")).unwrap();
        std::fs::write(dir.path().join("pkg-a/secrets/a.key"), "secret").unwrap();
        let pattern = dir
            .path()
            .join("pkg-*/secrets/*.key")
            .to_string_lossy()
            .into_owned();
        let (found, visited) =
            scan_with_budget(std::slice::from_ref(&pattern), "test", &|| false, SCAN_TIME).unwrap();
        assert_eq!(visited, 5);
        assert_eq!(found[&pattern].len(), 1);
    }

    #[test]
    fn a_second_scan_observes_new_matches_and_directory_matches() {
        let dir = tempfile::tempdir().unwrap();
        let pattern = dir.path().join("*.pem").to_string_lossy().into_owned();
        let patterns = std::slice::from_ref(&pattern);
        assert!(scan(patterns, "pre_launch", &|| false).unwrap()[&pattern].is_empty());
        std::fs::create_dir(dir.path().join("directory.pem")).unwrap();
        assert_eq!(
            scan(patterns, "post_command", &|| false).unwrap()[&pattern],
            [dir.path().join("directory.pem")]
        );
    }

    #[cfg(unix)]
    #[test]
    fn matching_symlink_protects_target_and_broken_link_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("secret");
        std::fs::write(&target, "secret").unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join("alias.pem")).unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("linked")).unwrap();
        let pattern = dir.path().join("**/*.pem").to_string_lossy().into_owned();
        let found = scan(std::slice::from_ref(&pattern), "pre_launch", &|| false).unwrap();
        assert_eq!(found[&pattern].len(), 2);
        assert!(found[&pattern].contains(&std::fs::canonicalize(&target).unwrap()));
        std::fs::remove_file(target).unwrap();
        let error = scan(&[pattern], "post_command", &|| false).unwrap_err();
        assert_eq!(error.code, "glob_scan_io_error");
        assert_eq!(error.phase, "post_command");
    }

    #[test]
    fn match_budget_is_shared_across_patterns() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..=MAX_MATCHES {
            let extension = if i % 2 == 0 { "pem" } else { "key" };
            std::fs::write(dir.path().join(format!("{i}.{extension}")), "").unwrap();
        }
        let patterns: Vec<_> = ["*.pem", "*.key"]
            .iter()
            .map(|p| dir.path().join(p).to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            scan(&patterns, "pre_launch", &|| false).unwrap_err().code,
            "glob_scan_match_limit"
        );
    }

    #[test]
    fn pattern_budget_is_bounded_but_duplicate_patterns_are_free() {
        let dir = tempfile::tempdir().unwrap();
        let pattern = dir.path().join("*.pem").to_string_lossy().into_owned();
        assert!(scan(&vec![pattern; MAX_PATTERNS + 1], "test", &|| false).is_ok());
        let patterns = (0..=MAX_PATTERNS)
            .map(|i| {
                dir.path()
                    .join(format!("{i}*.pem"))
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            scan(&patterns, "test", &|| false).unwrap_err().code,
            "glob_scan_pattern_limit"
        );
    }

    /// Explicit large-fixture acceptance, not part of every unit-test run.
    #[test]
    #[ignore = "creates over 100000 entries for large-workspace acceptance"]
    fn large_workspace_exceeds_old_node_limit_without_failing() {
        let dir = tempfile::tempdir().unwrap();
        let dependencies = dir.path().join("node_modules");
        std::fs::create_dir(&dependencies).unwrap();
        for i in 0..100_010 {
            std::fs::write(dependencies.join(format!("{i}.txt")), "").unwrap();
        }
        std::fs::write(dir.path().join(".env.local"), "secret").unwrap();
        let shallow = dir.path().join(".env.*").to_string_lossy().into_owned();
        let (_, visited) =
            scan_with_budget(&[shallow], "pre_launch", &|| false, SCAN_TIME).unwrap();
        assert_eq!(visited, 3);
        let patterns: Vec<_> = [".env.*", "**/*.pem", "**/*.key", "**/*.p12", "**/id_rsa*"]
            .iter()
            .map(|p| dir.path().join(p).to_string_lossy().into_owned())
            .collect();
        for label in ["first", "repeat"] {
            let start = Instant::now();
            let (_, visited) =
                scan_with_budget(&patterns, "pre_launch", &|| false, SCAN_TIME).unwrap();
            assert!(visited > 100_000);
            eprintln!(
                "{label}: visited={visited}, elapsed_ms={}",
                start.elapsed().as_millis()
            );
        }
    }
}
