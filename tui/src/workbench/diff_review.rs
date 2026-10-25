//! Read-only git review for the session workspace.
//!
//! This is a working-tree diff, not a claim that an agent turn owns every
//! change. Git is the source of truth; paths are passed as argv and `-z`
//! output is parsed as bytes, so spaces, quotes and newlines never become
//! shell syntax. Nothing here stages, commits, checks out or deletes files.

use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffFile {
    pub path: String,
    pub status: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffReview {
    pub files: Vec<DiffFile>,
    pub patch: String,
}

fn git(cwd: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Paths touched by tracked edits and untracked files, in git's order.
fn changed_paths(cwd: &Path) -> Result<Vec<DiffFile>, String> {
    let bytes = git(
        cwd,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    let fields: Vec<&[u8]> = bytes
        .split(|byte| *byte == 0)
        .filter(|s| !s.is_empty())
        .collect();
    let mut files = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        let field = fields[index];
        let status = String::from_utf8_lossy(field.get(..2).unwrap_or(field))
            .trim()
            .to_string();
        let path = String::from_utf8_lossy(field.get(3..).unwrap_or_default()).into_owned();
        index += 1;
        // In -z porcelain v1 the destination comes first and the source is a
        // second NUL field. The destination is the file being reviewed.
        if status.contains('R') || status.contains('C') {
            index += 1;
        }
        files.push(DiffFile { path, status });
    }
    Ok(files)
}

fn patch_for(cwd: &Path, file: &DiffFile) -> Result<String, String> {
    if file.status == "??" {
        let absolute = cwd.join(Path::new(&file.path));
        let body = std::fs::read(&absolute).map_err(|error| error.to_string())?;
        return Ok(if body.contains(&0) {
            format!("Binary file {} (untracked)\n", file.path)
        } else {
            let body = String::from_utf8_lossy(&body);
            let lines = body.lines().count();
            let mut out = format!("diff --git a/{0} b/{0}\nnew file\n--- /dev/null\n+++ b/{0}\n@@ -0,0 +1,{lines} @@\n", file.path);
            for line in body.lines() {
                out.push('+');
                out.push_str(line);
                out.push('\n');
            }
            out
        });
    }
    // One review covers staged and unstaged edits. An unborn HEAD falls back
    // to index/worktree patches rather than pretending the first commit is empty.
    match git(
        cwd,
        &[
            "diff",
            "HEAD",
            "--no-ext-diff",
            "--no-color",
            "--",
            &file.path,
        ],
    ) {
        Ok(patch) => Ok(String::from_utf8_lossy(&patch).into_owned()),
        Err(_) => {
            let mut patch = String::from_utf8_lossy(&git(
                cwd,
                &[
                    "diff",
                    "--cached",
                    "--no-ext-diff",
                    "--no-color",
                    "--",
                    &file.path,
                ],
            )?)
            .into_owned();
            patch.push_str(&String::from_utf8_lossy(&git(
                cwd,
                &["diff", "--no-ext-diff", "--no-color", "--", &file.path],
            )?));
            Ok(patch)
        }
    }
}

pub fn load(cwd: &Path, selected: &str) -> Result<DiffReview, String> {
    let files = changed_paths(cwd)?;
    let file = files
        .iter()
        .find(|file| file.path == selected)
        .or_else(|| files.first())
        .cloned();
    let patch = match file {
        Some(file) => patch_for(cwd, &file)?,
        None => String::new(),
    };
    Ok(DiffReview { files, patch })
}

/// A bounded preview avoids retaining a multi-megabyte patch in the frame
/// model. The complete file list remains available for navigation.
pub fn bounded_patch(patch: &str) -> String {
    const MAX_ROWS: usize = 2_000;
    const MAX_BYTES: usize = 256 * 1024;
    let mut out = String::new();
    for (index, line) in patch.lines().enumerate() {
        if index >= MAX_ROWS || out.len() + line.len() > MAX_BYTES {
            out.push_str("… diff preview truncated; inspect the file or git diff for the rest …\n");
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_a_large_patch_without_cutting_a_utf8_character() {
        let patch = format!("{}\n", "中".repeat(100_000));
        let bounded = bounded_patch(&patch);
        assert!(bounded.len() <= 256 * 1024 + 90);
        assert!(bounded.contains("truncated"));
    }

    #[test]
    fn real_git_lists_edits_and_untracked_files_and_selects_their_patches() {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "tui@example.invalid"]);
        git(&["config", "user.name", "TUI test"]);
        std::fs::write(dir.path().join("tracked file.txt"), "old\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "base"]);
        std::fs::write(dir.path().join("tracked file.txt"), "new\n").unwrap();
        std::fs::write(dir.path().join("new.txt"), "hello\n").unwrap();

        let review = load(dir.path(), "new.txt").unwrap();
        assert_eq!(review.files.len(), 2);
        assert!(review.patch.contains("new file"));
        let tracked = load(dir.path(), "tracked file.txt").unwrap();
        assert!(tracked.patch.contains("-old"));
        assert!(tracked.patch.contains("+new"));

        std::fs::write(dir.path().join("tracked file.txt"), "staged\n").unwrap();
        git(&["add", "tracked file.txt"]);
        std::fs::write(
            dir.path().join("tracked file.txt"),
            "staged then unstaged\n",
        )
        .unwrap();
        let combined = load(dir.path(), "tracked file.txt").unwrap();
        assert!(combined.patch.contains("-old"));
        assert!(combined.patch.contains("+staged then unstaged"));
    }
}
