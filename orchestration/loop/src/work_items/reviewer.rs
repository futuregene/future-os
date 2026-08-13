//! Reviewer recommendation for the `pr_review_queue` capability (P2-3②):
//! path-owner mapping as the starting signal — CODEOWNERS / OWNERS-style
//! mapping files plus a recent-commit heuristic.
//!
//! Everything is a deterministic pure function over parsed content (testable
//! without git); the only side-effecting helper is `git_recent_commits`,
//! which shells out to `git log` and degrades to an empty heuristic source
//! on any failure.

use std::collections::BTreeMap;
use std::path::Path;

/// One CODEOWNERS rule: a gitignore-style path pattern + its owners
/// (last matching rule wins — GitHub CODEOWNERS semantics).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerRule {
    pub pattern: String,
    pub owners: Vec<String>,
}

/// One ranked reviewer candidate.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReviewerCandidate {
    pub reviewer: String,
    pub score: u32,
    pub sources: Vec<String>,
}

/// Parse a CODEOWNERS file: `<pattern> <owner>…` per line, `#` comments and
/// blank lines skipped. Patterns keep their verbatim shape (`/src/`, `*.rs`,
/// `docs/**`).
pub fn parse_codeowners(content: &str) -> Vec<OwnerRule> {
    let mut rules = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let pattern = parts
            .next()
            .expect("a non-empty trimmed line always yields a pattern token");
        let owners: Vec<String> = parts.map(|owner| owner.to_string()).collect();
        if owners.is_empty() {
            continue;
        }
        rules.push(OwnerRule {
            pattern: pattern.to_string(),
            owners,
        });
    }
    rules
}

/// Parse an OWNERS file: one owner per line, `#` comments and blank lines
/// skipped (email/handle tokens kept verbatim).
pub fn parse_owners(content: &str) -> Vec<String> {
    let mut owners = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        owners.push(trimmed.to_string());
    }
    owners
}

/// `*` within a path segment matches any run not crossing `/`; `?` one
/// character (gitignore subset used by CODEOWNERS).
fn segment_matches(pattern: &[char], segment: &[char]) -> bool {
    match pattern.first() {
        None => segment.is_empty(),
        Some('*') => {
            let mut rest = pattern;
            while rest.first() == Some(&'*') {
                rest = &rest[1..];
            }
            (0..=segment.len()).any(|skip| segment_matches(rest, &segment[skip..]))
        }
        Some('?') => !segment.is_empty() && segment_matches(&pattern[1..], &segment[1..]),
        Some(ch) => segment.first() == Some(ch) && segment_matches(&pattern[1..], &segment[1..]),
    }
}

/// Segment-wise match with `**` crossing segments.
fn path_segments_match(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.first() {
        None => path.is_empty(),
        Some(segment) if *segment == "**" => {
            path_segments_match(&pattern[1..], path)
                || (!path.is_empty() && path_segments_match(pattern, &path[1..]))
        }
        Some(segment) => {
            !path.is_empty()
                && segment_matches(
                    &segment.chars().collect::<Vec<_>>(),
                    &path[0].chars().collect::<Vec<_>>(),
                )
                && path_segments_match(&pattern[1..], &path[1..])
        }
    }
}

/// Match a CODEOWNERS pattern against a repo-relative path. Leading `/`
/// anchors the pattern at the repo root (paths are compared root-relative,
/// so it is informational); a trailing `/` matches the directory and
/// everything under it; `**` crosses path segments.
pub fn path_matches(pattern: &str, path: &str) -> bool {
    let pattern_raw = pattern.trim();
    let dir_pattern = pattern_raw.ends_with('/');
    // GitHub CODEOWNERS semantics: a pattern without `/` (and without a
    // trailing `/`) matches the basename of the path at any depth, e.g.
    // `*.md` matches `docs/readme.md`. Anchored forms (`/src/`, `src/`,
    // `src/**`) match from the root as usual.
    let basename_pattern =
        !dir_pattern && !pattern_raw.starts_with('/') && !pattern_raw.contains('/');
    let pattern = pattern_raw.trim_start_matches('/').trim_end_matches('/');
    let path = path.trim().trim_start_matches('/').trim_end_matches('/');
    if pattern.is_empty() {
        return true;
    }
    let pattern_segments: Vec<&str> = pattern.split('/').collect();
    let path_segments: Vec<&str> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let segment_match = |p: &str, s: &str| {
        segment_matches(
            &p.chars().collect::<Vec<_>>(),
            &s.chars().collect::<Vec<_>>(),
        )
    };
    if basename_pattern {
        return path_segments
            .last()
            .is_some_and(|segment| segment_match(pattern_segments[0], segment));
    }
    if pattern_segments.contains(&"**") {
        if dir_pattern && pattern_segments.len() == 1 {
            // `**/` — everything under the root.
            return true;
        }
        if dir_pattern {
            // `dir/**/` — the prefix before `**` must match, then anything
            // underneath follows.
            let prefix: Vec<&str> = pattern_segments
                .iter()
                .take_while(|segment| **segment != "**")
                .copied()
                .collect();
            return path_segments.len() >= prefix.len()
                && prefix
                    .iter()
                    .zip(path_segments.iter())
                    .all(|(p, s)| segment_match(p, s));
        }
        return path_segments_match(&pattern_segments, &path_segments);
    }
    if dir_pattern {
        return path_segments.len() >= pattern_segments.len()
            && pattern_segments
                .iter()
                .zip(path_segments.iter())
                .all(|(p, s)| segment_match(p, s));
    }
    path_segments.len() == pattern_segments.len()
        && pattern_segments
            .iter()
            .zip(path_segments.iter())
            .all(|(p, s)| segment_match(p, s))
}

/// Resolve the owners of a path from CODEOWNERS rules (last matching rule
/// wins).
pub fn codeowners_for_path(rules: &[OwnerRule], path: &str) -> Vec<String> {
    rules
        .iter()
        .rev()
        .find(|rule| path_matches(&rule.pattern, path))
        .map(|rule| rule.owners.clone())
        .unwrap_or_default()
}

/// Normalize a directory key (no leading/trailing slash; root = "").
pub fn normalize_dir(dir: &str) -> String {
    dir.trim().trim_matches('/').to_string()
}

/// Scan a repository tree for OWNERS files (bounded depth; `.git` skipped)
/// and map each relative directory → its owners. A missing/unreadable root
/// degrades to an empty map.
pub fn scan_owners_files(root: &Path) -> BTreeMap<String, Vec<String>> {
    const MAX_DEPTH: usize = 8;
    fn visit(
        dir: &Path,
        depth: usize,
        root: &Path,
        dir_owners: &mut BTreeMap<String, Vec<String>>,
    ) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            if name == ".git" {
                continue;
            }
            if path.is_dir() && depth < MAX_DEPTH {
                visit(&path, depth + 1, root, dir_owners);
            } else if path.is_file() && name == "OWNERS" {
                let relative = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .parent()
                    .unwrap_or(Path::new(""))
                    .to_string_lossy()
                    .replace('\\', "/");
                let key = normalize_dir(&relative);
                let owners = std::fs::read_to_string(&path)
                    .ok()
                    .map(|content| parse_owners(&content))
                    .unwrap_or_default();
                if !owners.is_empty() {
                    dir_owners.insert(key, owners);
                }
            }
        }
    }
    let mut dir_owners: BTreeMap<String, Vec<String>> = BTreeMap::new();
    visit(root, 0, root, &mut dir_owners);
    dir_owners
}

/// The nearest ancestor OWNERS entry for a path: the deepest directory
/// (including the root) that has an OWNERS file.
pub fn nearest_owners<'a>(
    dir_owners: &'a BTreeMap<String, Vec<String>>,
    path: &str,
) -> Option<(String, &'a Vec<String>)> {
    let mut dir = normalize_dir(path);
    loop {
        if let Some(owners) = dir_owners.get(&dir) {
            return Some((dir, owners));
        }
        if dir.is_empty() {
            return dir_owners.get("").map(|owners| (String::new(), owners));
        }
        dir = match dir.rfind('/') {
            Some(pos) => dir[..pos].to_string(),
            None => String::new(),
        };
    }
}

/// Aggregate reviewer candidates for the changed paths.
///
/// Scoring (deterministic, highest wins; ties broken by name):
/// - CODEOWNERS owner of a matching rule: +3 (credited once — an ownership
///   signal is one signal, not one per changed path)
/// - OWNERS of the nearest ancestor directory: +2 (credited once)
/// - author of a recent commit touching the path or a parent dir: +1
///   (per commit — each commit is independent evidence of activity)
pub fn recommend_reviewers(
    paths: &[String],
    codeowners: Option<&[OwnerRule]>,
    dir_owners: &BTreeMap<String, Vec<String>>,
    recent_commits: &[(String, String)],
) -> Vec<ReviewerCandidate> {
    /// Credit a reviewer; ownership signals (`once = true`) count only the
    /// first time the same source kind credits that reviewer, while
    /// per-commit signals accumulate.
    fn credit(
        scores: &mut BTreeMap<String, (u32, Vec<String>)>,
        reviewer: &str,
        score: u32,
        source: String,
        once: bool,
    ) {
        let reviewer = reviewer.trim().to_string();
        if reviewer.is_empty() {
            return;
        }
        let entry = scores.entry(reviewer).or_insert_with(|| (0, Vec::new()));
        if once {
            let kind = source.split(':').next().unwrap_or_default();
            if entry.1.iter().any(|s| s.split(':').next() == Some(kind)) {
                return;
            }
        }
        entry.0 += score;
        if !entry.1.contains(&source) {
            entry.1.push(source);
        }
    }
    let mut scores: BTreeMap<String, (u32, Vec<String>)> = BTreeMap::new();
    for path in paths {
        if let Some(rules) = codeowners {
            // Every matching rule credits its owners (recommendation
            // aggregates signals; ownership *resolution* stays last-match-wins
            // in `codeowners_for_path`).
            for rule in rules {
                if !path_matches(&rule.pattern, path) {
                    continue;
                }
                for owner in &rule.owners {
                    credit(&mut scores, owner, 3, format!("codeowners:{path}"), true);
                }
            }
        }
        if let Some((dir, owners)) = nearest_owners(dir_owners, path) {
            for owner in owners {
                credit(&mut scores, owner, 2, format!("owners:{dir}"), true);
            }
        }
        for (author, commit_path) in recent_commits {
            let related = commit_path == path
                || path.starts_with(&format!("{commit_path}/"))
                || commit_path.starts_with(&format!("{path}/"));
            if related {
                credit(
                    &mut scores,
                    author,
                    1,
                    format!("recent-commit:{commit_path}"),
                    false,
                );
            }
        }
    }
    let mut candidates: Vec<ReviewerCandidate> = scores
        .into_iter()
        .map(|(reviewer, (score, sources))| ReviewerCandidate {
            reviewer,
            score,
            sources,
        })
        .collect();
    candidates.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.reviewer.cmp(&b.reviewer))
    });
    candidates
}

/// Load recent commits touching paths via `git log` (author + changed path
/// pairs). Any failure (missing repo, no git) degrades to an empty heuristic
/// source — the owner mapping still stands.
pub fn git_recent_commits(
    repo: &Path,
    since_days: u32,
    max_entries: usize,
) -> Vec<(String, String)> {
    // Author lines are marked with an ASCII record separator so the parse
    // never confuses an author name with a changed path.
    let separator = '\u{1e}';
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .arg("log")
        .arg(format!("--since={since_days} days ago"))
        .arg(format!("--format={separator}%an"))
        .arg("--name-only")
        .arg("-n")
        .arg(max_entries.to_string())
        .output();
    let Ok(output) = output else { return vec![] };
    if !output.status.success() {
        return vec![];
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut commits: Vec<(String, String)> = Vec::new();
    let mut current_author = String::new();
    for line in stdout.lines() {
        if line.starts_with(separator) {
            current_author = line.trim_start_matches(separator).trim().to_string();
        } else if !line.trim().is_empty() && !current_author.is_empty() {
            commits.push((current_author.clone(), line.trim().to_string()));
        }
    }
    commits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codeowners_parsing_skips_comments_and_blanks() {
        let content = "# top-level comment\n\n/src/core.rs @alice @bob\n*.md docs-team\n\ninvalid-line-without-owner\n";
        let rules = parse_codeowners(content);
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].pattern, "/src/core.rs");
        assert_eq!(rules[0].owners, vec!["@alice", "@bob"]);
        assert_eq!(rules[1].owners, vec!["docs-team"]);
    }

    #[test]
    fn owners_parsing_takes_one_owner_per_line() {
        let content = "# reviewers\n\nalice@example.com\nbob\n";
        assert_eq!(parse_owners(content), vec!["alice@example.com", "bob"]);
    }

    #[test]
    fn path_matching_covers_file_dir_and_glob_patterns() {
        // exact file
        assert!(path_matches("/src/core.rs", "src/core.rs"));
        assert!(!path_matches("/src/core.rs", "src/other.rs"));
        // directory prefix
        assert!(path_matches("docs/", "docs/guide/intro.md"));
        assert!(path_matches("docs/", "docs/README.md"));
        assert!(!path_matches("docs/", "other/README.md"));
        // in-segment wildcard
        assert!(path_matches("src/*.rs", "src/main.rs"));
        assert!(!path_matches("src/*.rs", "src/sub/main.rs"));
        // `**` crosses segments
        assert!(path_matches("src/**", "src/a/b/c.rs"));
        assert!(path_matches("**/*.rs", "any/deep/main.rs"));
        // slash-less patterns match the basename at any depth (GitHub
        // CODEOWNERS semantics)
        assert!(path_matches("*.md", "docs/readme.md"));
        assert!(path_matches("*.md", "README.md"));
        assert!(!path_matches("*.md", "docs/readme.rst"));
        assert!(!path_matches("*.rs", "src/deep/main.py"));
        // empty pattern matches everything
        assert!(path_matches("/", "anything/at/all.rs"));
    }

    #[test]
    fn codeowners_last_matching_rule_wins() {
        let rules = parse_codeowners("* @global\nsrc/ @core\nsrc/*.rs @lang");
        assert_eq!(
            codeowners_for_path(&rules, "src/main.rs"),
            vec!["@lang".to_string()]
        );
        assert_eq!(
            codeowners_for_path(&rules, "src/util/build.py"),
            vec!["@core".to_string()]
        );
        assert_eq!(
            codeowners_for_path(&rules, "README.md"),
            vec!["@global".to_string()]
        );
    }

    #[test]
    fn nearest_owners_walks_ancestors_to_the_root() {
        let mut dirs: BTreeMap<String, Vec<String>> = BTreeMap::new();
        dirs.insert("".to_string(), vec!["root-owner".to_string()]);
        dirs.insert("src".to_string(), vec!["src-owner".to_string()]);
        dirs.insert("src/deep".to_string(), vec!["deep-owner".to_string()]);
        assert_eq!(
            nearest_owners(&dirs, "src/deep/file.rs"),
            Some(("src/deep".to_string(), &vec!["deep-owner".to_string()]))
        );
        assert_eq!(
            nearest_owners(&dirs, "src/main.rs"),
            Some(("src".to_string(), &vec!["src-owner".to_string()]))
        );
        assert_eq!(
            nearest_owners(&dirs, "README.md"),
            Some(("".to_string(), &vec!["root-owner".to_string()]))
        );
        assert_eq!(nearest_owners(&BTreeMap::new(), "x.rs"), None);
    }

    #[test]
    fn recommendation_aggregates_all_three_sources() {
        let rules = parse_codeowners("src/** @core");
        let mut dirs: BTreeMap<String, Vec<String>> = BTreeMap::new();
        dirs.insert("src".to_string(), vec!["alice".to_string()]);
        let commits = vec![
            ("bob".to_string(), "src/core.rs".to_string()),
            ("unrelated".to_string(), "docs/readme.md".to_string()),
        ];
        let candidates =
            recommend_reviewers(&["src/core.rs".to_string()], Some(&rules), &dirs, &commits);
        let by_name: BTreeMap<String, &ReviewerCandidate> =
            candidates.iter().map(|c| (c.reviewer.clone(), c)).collect();
        assert_eq!(by_name["@core"].score, 3);
        assert_eq!(by_name["alice"].score, 2);
        assert_eq!(by_name["bob"].score, 1);
        assert!(!by_name.contains_key("unrelated"));
        // ranked by score desc
        let names: Vec<&str> = candidates.iter().map(|c| c.reviewer.as_str()).collect();
        assert_eq!(names, vec!["@core", "alice", "bob"]);
        // sources dedupe and record where the signal came from
        assert!(by_name["@core"]
            .sources
            .iter()
            .any(|s| s == "codeowners:src/core.rs"));
        assert!(by_name["alice"].sources.iter().any(|s| s == "owners:src"));
        assert!(by_name["bob"]
            .sources
            .iter()
            .any(|s| s == "recent-commit:src/core.rs"));
    }

    #[test]
    fn same_reviewer_scores_accumulate_across_sources() {
        let rules = parse_codeowners("src/** @core");
        let mut dirs: BTreeMap<String, Vec<String>> = BTreeMap::new();
        dirs.insert("src".to_string(), vec!["alice".to_string()]);
        let commits = vec![
            ("alice".to_string(), "src/core.rs".to_string()),
            ("alice".to_string(), "src/util.rs".to_string()),
        ];
        let candidates = recommend_reviewers(
            &["src/core.rs".to_string(), "src/util.rs".to_string()],
            Some(&rules),
            &dirs,
            &commits,
        );
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].reviewer, "alice");
        assert_eq!(candidates[0].score, 2 + 1 + 1);
        // tie on score broken by name
        let rules2 = parse_codeowners("src/** @alpha\nsrc/** @beta");
        let candidates = recommend_reviewers(
            &["src/core.rs".to_string()],
            Some(&rules2),
            &BTreeMap::new(),
            &[],
        );
        assert_eq!(candidates[0].reviewer, "@alpha");
        assert_eq!(candidates[1].reviewer, "@beta");
    }

    #[test]
    fn path_matching_covers_question_mark_and_double_star_dir() {
        // in-segment single-char wildcard
        assert!(path_matches("src/file?.rs", "src/file1.rs"));
        assert!(!path_matches("src/file?.rs", "src/file12.rs"));
        assert!(!path_matches("src/file?.rs", "src/file.rs"));
        // `**/` matches everything under the root
        assert!(path_matches("**/", "any/deep/path.rs"));
        // `dir/**/` requires the prefix then anything underneath
        assert!(path_matches("src/**/", "src/a/b/c.rs"));
        assert!(!path_matches("src/**/", "other/a.rs"));
    }

    #[test]
    fn scan_owners_files_walks_tree_and_skips_git() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join("OWNERS"), "root-owner\n").unwrap();
        std::fs::write(root.join("src/OWNERS"), "src-owner\n").unwrap();
        // A `.git` OWNERS file must be skipped (git internals).
        std::fs::write(root.join(".git/OWNERS"), "git-owner\n").unwrap();
        let dirs = scan_owners_files(root);
        assert_eq!(dirs.get(""), Some(&vec!["root-owner".to_string()]));
        assert_eq!(dirs.get("src"), Some(&vec!["src-owner".to_string()]));
        assert!(!dirs.contains_key(".git"));
        // An unreadable root degrades to an empty map.
        assert!(scan_owners_files(std::path::Path::new("/nonexistent/repo-xyz")).is_empty());
    }

    #[test]
    fn empty_reviewer_names_are_skipped() {
        let mut dirs: BTreeMap<String, Vec<String>> = BTreeMap::new();
        dirs.insert("".to_string(), vec!["   ".to_string(), "alice".to_string()]);
        let candidates = recommend_reviewers(&["src/x.rs".to_string()], None, &dirs, &[]);
        // only alice is credited; the whitespace-only reviewer is dropped.
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].reviewer, "alice");
        assert_eq!(candidates[0].score, 2);
    }

    #[test]
    fn git_recent_commits_degrades_on_missing_repo() {
        let commits = git_recent_commits(Path::new("/nonexistent/repo-xyz"), 30, 50);
        assert!(commits.is_empty());
    }

    #[test]
    fn git_recent_commits_parses_real_log_output() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(repo)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "a@b.c"]);
        git(&["config", "user.name", "Test Author"]);
        std::fs::write(repo.join("file.rs"), "fn main() {}").unwrap();
        git(&["add", "file.rs"]);
        git(&["commit", "-q", "-m", "init"]);
        let commits = git_recent_commits(repo, 30, 50);
        assert!(
            commits
                .iter()
                .any(|(author, path)| author == "Test Author" && path == "file.rs"),
            "got: {commits:?}"
        );
    }
}
