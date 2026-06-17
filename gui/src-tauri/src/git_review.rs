use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::store;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitReview {
    is_git_workspace: bool,
    workspace_path: String,
    branch: Option<String>,
    upstream: Option<String>,
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

pub fn get_git_review(workspace_id: String) -> Result<GitReview, String> {
    let workspace = store::get_workspace(&workspace_id)?
        .ok_or_else(|| "Workspace could not be loaded.".to_string())?;
    let workspace_path = PathBuf::from(&workspace.path);
    if !is_git_workspace(&workspace_path) {
        return Ok(GitReview {
            is_git_workspace: false,
            workspace_path: workspace.path,
            branch: None,
            upstream: None,
            additions: 0,
            deletions: 0,
            files: Vec::new(),
        });
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

    let status_by_path = git_status_by_path(&workspace_path);
    let mut files = tracked_diff_files(&workspace_path, &status_by_path);
    append_untracked_files(&workspace_path, &mut files, &status_by_path);
    files.sort_by(|left, right| left.path.cmp(&right.path));

    let additions = files.iter().map(|file| file.additions).sum();
    let deletions = files.iter().map(|file| file.deletions).sum();

    Ok(GitReview {
        is_git_workspace: true,
        workspace_path: workspace.path,
        branch,
        upstream,
        additions,
        deletions,
        files,
    })
}

pub fn is_git_workspace(path: &Path) -> bool {
    let Ok(root) = git_output(path, ["rev-parse", "--show-toplevel"]) else {
        return false;
    };
    let root = canonical_or_raw(root.trim());
    let workspace = canonical_or_raw(path);
    root == workspace
}

fn canonical_or_raw(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn tracked_diff_files(
    workspace_path: &Path,
    status_by_path: &HashMap<String, String>,
) -> Vec<GitReviewFile> {
    let numstat =
        git_output(workspace_path, ["diff", "--numstat", "HEAD", "--"]).unwrap_or_default();
    let diff = git_output(
        workspace_path,
        ["diff", "--no-color", "--unified=80", "HEAD", "--"],
    )
    .unwrap_or_default();
    let diff_by_path = split_git_diff_by_path(&diff);

    numstat
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let additions = parse_numstat(parts.next()?);
            let deletions = parse_numstat(parts.next()?);
            let path = parts.next()?.to_string();
            let normalized_path = normalize_numstat_path(&path);
            Some(GitReviewFile {
                status: status_by_path
                    .get(&normalized_path)
                    .cloned()
                    .unwrap_or_else(|| "modified".to_string()),
                additions,
                deletions,
                diff: diff_by_path
                    .get(&normalized_path)
                    .cloned()
                    .unwrap_or_default(),
                path: normalized_path,
            })
        })
        .collect()
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

fn split_git_diff_by_path(diff: &str) -> HashMap<String, String> {
    let mut chunks = HashMap::new();
    let mut current_path: Option<String> = None;
    let mut current = Vec::new();

    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            flush_diff_chunk(&mut chunks, current_path.take(), &mut current);
            current_path = diff_path_from_header(line);
        } else if let Some(path) = line.strip_prefix("+++ b/") {
            current_path = Some(path.to_string());
        }
        current.push(line.to_string());
    }
    flush_diff_chunk(&mut chunks, current_path, &mut current);
    chunks
}

fn flush_diff_chunk(
    chunks: &mut HashMap<String, String>,
    path: Option<String>,
    current: &mut Vec<String>,
) {
    if let Some(path) = path {
        if !current.is_empty() {
            chunks.insert(path, current.join("\n"));
        }
    }
    current.clear();
}

fn diff_path_from_header(line: &str) -> Option<String> {
    line.split(" b/")
        .nth(1)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
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

fn normalize_numstat_path(path: &str) -> String {
    if !path.contains(" => ") {
        return path.to_string();
    }

    if let Some(open_brace) = path.find('{') {
        if let Some(close_brace) = path[open_brace + 1..].find('}') {
            let close_brace = open_brace + 1 + close_brace;
            let before = &path[..open_brace];
            let inside = &path[open_brace + 1..close_brace];
            let after = &path[close_brace + 1..];
            if let Some((_, next)) = inside.rsplit_once(" => ") {
                return format!("{before}{next}{after}");
            }
        }
    }

    path.rsplit_once(" => ")
        .map(|(_, next)| next)
        .unwrap_or(path)
        .to_string()
}

fn parse_numstat(value: &str) -> i64 {
    value.parse::<i64>().unwrap_or(0)
}

fn git_output<const N: usize>(workspace_path: &Path, args: [&str; N]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_path)
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::normalize_numstat_path;

    #[test]
    fn normalize_numstat_path_keeps_plain_paths() {
        assert_eq!(normalize_numstat_path("src/main.rs"), "src/main.rs");
    }

    #[test]
    fn normalize_numstat_path_handles_simple_rename() {
        assert_eq!(normalize_numstat_path("old.txt => new.txt"), "new.txt");
    }

    #[test]
    fn normalize_numstat_path_handles_brace_rename() {
        assert_eq!(
            normalize_numstat_path("dir/{old => new}/file.txt"),
            "dir/new/file.txt",
        );
    }
}
