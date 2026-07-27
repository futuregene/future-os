use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::store;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitReview {
    is_git_workspace: bool,
    workspace_path: String,
    branch: Option<String>,
    upstream: Option<String>,
    diff_base: Option<String>,
    diff_base_label: Option<String>,
    additions: i64,
    deletions: i64,
    files: Vec<GitReviewFile>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitReviewFile {
    path: String,
    status: String,
    additions: i64,
    deletions: i64,
    diff: String,
}

/// Cache of the last computed review per (workspace, base, custom_base), keyed
/// by a cheap fingerprint. The Review tab polls every 1.5s; an uncached fetch
/// spawns 6+ git processes (incl. the full `--unified=80` patch) and reads
/// every untracked file into memory. The fingerprint costs two small spawns
/// (rev-parse + status) plus a metadata stat per changed file, and skips the
/// whole computation while nothing relevant moved.
/// (workspace_id, base, custom_base) → (fingerprint, review).
type ReviewCacheKey = (String, String, String);
type ReviewCacheEntry = (String, GitReview);

static REVIEW_CACHE: std::sync::LazyLock<
    std::sync::Mutex<HashMap<ReviewCacheKey, ReviewCacheEntry>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// Bound the cache (one entry per workspace/base combo in practice).
const REVIEW_CACHE_MAX: usize = 32;

pub fn get_git_review(
    workspace_id: String,
    base: Option<String>,
    custom_base: Option<String>,
) -> Result<GitReview, crate::AppError> {
    let workspace = store::get_workspace(&workspace_id)?
        .ok_or_else(|| "Workspace could not be loaded.".to_string())?;
    let workspace_path = PathBuf::from(&workspace.path);
    if !is_git_workspace(&workspace_path) {
        return Ok(GitReview {
            is_git_workspace: false,
            workspace_path: workspace.path,
            branch: None,
            upstream: None,
            diff_base: None,
            diff_base_label: None,
            additions: 0,
            deletions: 0,
            files: Vec::new(),
        });
    }

    let cache_key = (
        workspace_id.clone(),
        base.clone().unwrap_or_default(),
        custom_base.clone().unwrap_or_default(),
    );
    let fingerprint = review_fingerprint(&workspace_path);
    if let Ok(cache) = REVIEW_CACHE.lock() {
        if let Some((cached_fingerprint, review)) = cache.get(&cache_key) {
            if *cached_fingerprint == fingerprint {
                return Ok(review.clone());
            }
        }
    }

    let branch = git_output(&workspace_path, ["branch", "--show-current"])
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| git_output(&workspace_path, ["rev-parse", "--short", "HEAD"]).ok())
        .map(|value| value.trim().to_string());
    let upstream = git_output(
        &workspace_path,
        ["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    )
    .ok()
    .filter(|value| !value.trim().is_empty())
    .map(|value| value.trim().to_string());
    let diff_base = resolve_diff_base(
        &workspace_path,
        base.as_deref(),
        custom_base.as_deref(),
        upstream.as_deref(),
    );

    let status_by_path = git_status_by_path(&workspace_path);
    let mut files = tracked_diff_files(&workspace_path, &status_by_path, &diff_base.reference);
    append_untracked_files(&workspace_path, &mut files, &status_by_path);
    files.sort_by(|left, right| left.path.cmp(&right.path));

    let additions = files.iter().map(|file| file.additions).sum();
    let deletions = files.iter().map(|file| file.deletions).sum();

    let review = GitReview {
        is_git_workspace: true,
        workspace_path: workspace.path,
        branch,
        upstream,
        diff_base: Some(diff_base.reference),
        diff_base_label: Some(diff_base.label),
        additions,
        deletions,
        files,
    };

    if let Ok(mut cache) = REVIEW_CACHE.lock() {
        if cache.len() >= REVIEW_CACHE_MAX {
            cache.clear();
        }
        cache.insert(cache_key, (fingerprint, review.clone()));
    }
    Ok(review)
}

/// A cheap staleness signal for the review cache. Components:
/// - HEAD sha (commits, checkouts to a different commit);
/// - the `.git/HEAD` file content (`git checkout -b` switches branches at the
///   SAME commit — the sha is unchanged but the branch label in the review
///   must update);
/// - the status output (staging, new/deleted files);
/// - the git index mtime (re-staging identical content);
/// - FETCH_HEAD / packed-refs mtimes (a `git fetch` moves upstream refs
///   without touching HEAD, index, or the working tree — the upstream and
///   merge-base diff bases would otherwise serve stale results);
/// - the size+mtime of every file the status lists (edits to an
///   already-modified file don't change the status text, so without this the
///   main "agent keeps editing a file" flow would serve stale diffs).
///
/// Two small git spawns + a few metadata stats — no patch generation, no file
/// content reads. Known gap: a force-updated LOOSE ref (`git branch -f`) that
/// touches neither packed-refs nor FETCH_HEAD stays stale until any other
/// input moves; acceptable for a 1.5s-refresh UI cache.
fn review_fingerprint(workspace_path: &Path) -> String {
    use std::fmt::Write;
    let head = git_output(workspace_path, ["rev-parse", "HEAD"]).unwrap_or_default();
    let status = git_output(
        workspace_path,
        ["status", "--short", "--untracked-files=all"],
    )
    .unwrap_or_default();
    let git_dir = workspace_path.join(".git");
    let mut fingerprint = String::with_capacity(head.len() + status.len() + 256);
    fingerprint.push_str(head.trim());
    fingerprint.push('\n');
    fingerprint.push_str(&status);

    if let Ok(head_ref) = fs::read_to_string(git_dir.join("HEAD")) {
        let _ = write!(fingerprint, "\nHEAD:{}", head_ref.trim());
    }
    for meta_file in ["index", "FETCH_HEAD", "packed-refs"] {
        if let Ok(mtime) = fs::metadata(git_dir.join(meta_file)).and_then(|meta| meta.modified()) {
            let _ = write!(fingerprint, "\n{meta_file}:{mtime:?}");
        }
    }

    for path in status_paths(&status) {
        if let Ok(meta) = fs::metadata(workspace_path.join(&path)) {
            let mtime = meta.modified().ok();
            let _ = write!(fingerprint, "\n{path}:{}:{mtime:?}", meta.len());
        }
    }
    fingerprint
}

/// The paths listed by `git status --short` (renames resolve to the new path).
fn status_paths(status: &str) -> Vec<String> {
    status
        .lines()
        .filter_map(|line| {
            if line.len() < 4 {
                return None;
            }
            let raw_path = line[3..].trim();
            Some(
                raw_path
                    .rsplit_once(" -> ")
                    .map(|(_, next)| next)
                    .unwrap_or(raw_path)
                    .to_string(),
            )
        })
        .collect()
}

/// Short-TTL cache for [`is_git_workspace`]: the check forks `git rev-parse`
/// and sits on the artifact-persist path (every write/edit tool_end) plus the
/// review poll. A workspace's git-ness changes only via an external
/// `git init`/deletion of `.git`, so 30s of staleness is harmless.
static GIT_WORKSPACE_CACHE: std::sync::LazyLock<
    std::sync::Mutex<HashMap<PathBuf, (bool, std::time::Instant)>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

const GIT_WORKSPACE_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(30);
const GIT_WORKSPACE_CACHE_MAX: usize = 64;

pub fn is_git_workspace(path: &Path) -> bool {
    let key = canonical_or_raw(path);
    if let Ok(cache) = GIT_WORKSPACE_CACHE.lock() {
        if let Some((result, at)) = cache.get(&key) {
            if at.elapsed() < GIT_WORKSPACE_CACHE_TTL {
                return *result;
            }
        }
    }
    let result = is_git_workspace_uncached(path);
    if let Ok(mut cache) = GIT_WORKSPACE_CACHE.lock() {
        if cache.len() >= GIT_WORKSPACE_CACHE_MAX {
            cache.clear();
        }
        cache.insert(key, (result, std::time::Instant::now()));
    }
    result
}

fn is_git_workspace_uncached(path: &Path) -> bool {
    let Ok(root) = git_output(path, ["rev-parse", "--show-toplevel"]) else {
        return false;
    };
    let root = canonical_or_raw(root.trim());
    let workspace = canonical_or_raw(path);
    root == workspace
}

pub(crate) fn canonical_or_raw(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn tracked_diff_files(
    workspace_path: &Path,
    status_by_path: &HashMap<String, String>,
    base_ref: &str,
) -> Vec<GitReviewFile> {
    let numstat =
        git_output(workspace_path, ["diff", "--numstat", base_ref, "--"]).unwrap_or_default();
    let diff = git_output(
        workspace_path,
        ["diff", "--no-color", "--unified=80", base_ref, "--"],
    )
    .unwrap_or_default();
    let diff_by_path = crate::git_diff_parse::split_unified_patch_by_path(&diff);

    crate::git_diff_parse::parse_numstat(&numstat)
        .into_iter()
        .map(|row| {
            let normalized_path = crate::git_diff_parse::normalize_numstat_path(&row.path);
            GitReviewFile {
                status: status_by_path
                    .get(&normalized_path)
                    .cloned()
                    .unwrap_or_else(|| "modified".to_string()),
                additions: row.additions,
                deletions: row.deletions,
                diff: diff_by_path
                    .get(&normalized_path)
                    .cloned()
                    .unwrap_or_default(),
                path: normalized_path,
            }
        })
        .collect()
}

struct DiffBase {
    label: String,
    reference: String,
}

fn resolve_diff_base(
    workspace_path: &Path,
    base: Option<&str>,
    custom_base: Option<&str>,
    upstream: Option<&str>,
) -> DiffBase {
    match base.unwrap_or("head") {
        "upstream" => upstream
            .filter(|value| !value.trim().is_empty())
            .map(|reference| DiffBase {
                label: format!("upstream ({reference})"),
                reference: reference.to_string(),
            })
            .unwrap_or_else(head_diff_base),
        "merge-base" => upstream
            .and_then(|reference| {
                git_output(workspace_path, ["merge-base", "HEAD", reference])
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
            })
            .map(|reference| DiffBase {
                label: "merge-base".to_string(),
                reference,
            })
            .unwrap_or_else(head_diff_base),
        "custom" => custom_base
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .and_then(|reference| {
                git_output(
                    workspace_path,
                    ["rev-parse", "--verify", &format!("{reference}^{{commit}}")],
                )
                .ok()
                .map(|resolved| DiffBase {
                    // Diff the resolved commit SHA, not the raw ref — an annotated
                    // tag's object differs from the commit it points at.
                    label: format!("custom ({reference})"),
                    reference: resolved.trim().to_string(),
                })
            })
            .unwrap_or_else(head_diff_base),
        _ => head_diff_base(),
    }
}

fn head_diff_base() -> DiffBase {
    DiffBase {
        label: "HEAD".to_string(),
        reference: "HEAD".to_string(),
    }
}

fn append_untracked_files(
    workspace_path: &Path,
    files: &mut Vec<GitReviewFile>,
    status_by_path: &HashMap<String, String>,
) {
    let known_paths: HashSet<String> = files.iter().map(|file| file.path.clone()).collect();
    let untracked = git_output(
        workspace_path,
        ["ls-files", "--others", "--exclude-standard"],
    )
    .unwrap_or_default();

    for path in untracked.lines().filter(|line| !line.trim().is_empty()) {
        if known_paths.contains(path) {
            continue;
        }
        let full_path = workspace_path.join(path);
        let content = fs::read_to_string(&full_path).unwrap_or_default();
        let additions = content.lines().count() as i64;
        files.push(GitReviewFile {
            path: path.to_string(),
            status: status_by_path
                .get(path)
                .cloned()
                .unwrap_or_else(|| "untracked".to_string()),
            additions,
            deletions: 0,
            diff: pseudo_added_file_diff(path, &content),
        });
    }
}

fn git_status_by_path(workspace_path: &Path) -> HashMap<String, String> {
    let output = git_output(
        workspace_path,
        ["status", "--short", "--untracked-files=all"],
    )
    .unwrap_or_default();
    output
        .lines()
        .filter_map(|line| {
            if line.len() < 4 {
                return None;
            }
            let code = &line[..2];
            let raw_path = line[3..].trim();
            let path = raw_path
                .rsplit_once(" -> ")
                .map(|(_, next)| next)
                .unwrap_or(raw_path)
                .to_string();
            Some((path, status_label(code)))
        })
        .collect()
}

fn status_label(code: &str) -> String {
    if code.contains("??") {
        return "untracked".to_string();
    }
    if code.contains('A') {
        return "added".to_string();
    }
    if code.contains('D') {
        return "deleted".to_string();
    }
    if code.contains('R') {
        return "renamed".to_string();
    }
    if code.contains('C') {
        return "copied".to_string();
    }
    "modified".to_string()
}

fn pseudo_added_file_diff(path: &str, content: &str) -> String {
    let lines: Vec<&str> = content.lines().take(300).collect();
    let mut diff = vec![
        format!("diff --git a/{path} b/{path}"),
        "new file mode 100644".to_string(),
        "--- /dev/null".to_string(),
        format!("+++ b/{path}"),
        format!("@@ -0,0 +1,{} @@", lines.len()),
    ];
    diff.extend(lines.into_iter().map(|line| format!("+{line}")));
    diff.join("\n")
}

fn git_output<const N: usize>(
    workspace_path: &Path,
    args: [&str; N],
) -> Result<String, crate::AppError> {
    use crate::proc::NoWindow;
    let output = Command::new("git")
        .no_window()
        .arg("-C")
        .arg(workspace_path)
        // Disable path quoting so non-ASCII filenames come back literal and line
        // up with the numstat/diff path maps (shadow_review/diff.rs does the
        // same). Harmless for non-diff subcommands.
        .args(["-c", "core.quotePath=false"])
        .args(args)
        .output()?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr)
            .trim()
            .to_string()
            .into());
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::status_paths;

    #[test]
    fn status_paths_extracts_plain_and_renamed_paths() {
        let status = " M src/a.ts\n?? src/new file.md\nR  old.ts -> src/renamed.ts\nA  added.rs\n";
        assert_eq!(
            status_paths(status),
            vec![
                "src/a.ts".to_string(),
                "src/new file.md".to_string(),
                "src/renamed.ts".to_string(),
                "added.rs".to_string(),
            ]
        );
    }

    #[test]
    fn status_paths_skips_short_and_empty_lines() {
        assert!(status_paths("").is_empty());
        assert!(status_paths("##\n M").is_empty());
    }
}
