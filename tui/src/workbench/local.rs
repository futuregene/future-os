//! The surface's *local* facts: the working directory's files and the installed
//! skills.
//!
//! Neither travels over gRPC — they are things this machine knows — so they are
//! read where they live:
//!
//! * **Files** for the `@` menu come from the session's own directory, through
//!   `git ls-files` when it is a repository (which honours `.gitignore`, the
//!   reason a menu of candidates does not fill with build output) and a short,
//!   pruned walk otherwise.
//! * **Skills** are the installed ones (`~/.future/agent/skills` and the shared
//!   `~/.agents/skills`), the same directories the agent discovers: the agent's
//!   `get_state` names them, and this is the fallback that fills in their
//!   descriptions for the palette.
//!
//! Every reader is best-effort and total: a missing directory, a half-written
//! frontmatter block or an unreadable file yields fewer rows, never a panic and
//! never a notice. These are menu candidates, not the conversation.

use std::path::{Path, PathBuf};

use super::state::SkillRow;

/// A bounded list of files under `cwd`, for the `@` menu.
///
/// `git ls-files` when the directory is a repository — it is the fastest honest
/// answer and it already honours `.gitignore`, which is what keeps a menu of
/// candidates from filling with build output. Anything else (or a git that is
/// missing, or a listing too slow to be worth it) falls back to a short walk that
/// prunes the directories every project has.
pub fn workspace_files(cwd: &Path) -> Vec<String> {
    const FILE_LIMIT: usize = 4000;
    if cwd.as_os_str().is_empty() || !cwd.is_dir() {
        return Vec::new();
    }
    let args = vec![
        "-C".to_string(),
        cwd.display().to_string(),
        "ls-files".to_string(),
        "--cached".to_string(),
        "--others".to_string(),
        "--exclude-standard".to_string(),
    ];
    if let Ok((0, stdout, _)) = crate::subprocess::spawn_with_timeout(
        Path::new("git"),
        &args,
        std::time::Duration::from_secs(2),
    ) {
        let mut files: Vec<String> = stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .take(FILE_LIMIT)
            .map(str::to_string)
            .collect();
        if !files.is_empty() {
            files.sort();
            files.dedup();
            return files;
        }
    }
    let mut files = Vec::new();
    walk(cwd, cwd, 0, &mut files, FILE_LIMIT);
    files.sort();
    files
}

/// Directories a file menu never needs: version control, build output, package
/// caches. Pruned rather than listed and filtered, so the walk stays cheap.
const SKIP_DIRS: [&str; 10] = [
    ".git",
    "target",
    "node_modules",
    "dist",
    "build",
    ".venv",
    "venv",
    "__pycache__",
    ".next",
    "coverage",
];

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<String>, limit: usize) {
    if depth > 8 || out.len() >= limit {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut children: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    children.sort();
    for path in children {
        if out.len() >= limit {
            return;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            if SKIP_DIRS.contains(&name) {
                continue;
            }
            walk(root, &path, depth + 1, out, limit);
        } else if let Ok(relative) = path.strip_prefix(root) {
            out.push(relative.display().to_string());
        }
    }
}

// ─── skills ─────────────────────────────────────────────────────────────────

/// The installed skills: `~/.future/agent/skills` (app scope) first, then the
/// shared `~/.agents/skills`, deduplicated by name.
pub fn installed_skills() -> Vec<SkillRow> {
    let mut dirs = vec![crate::home::future_state_dir().join("agent").join("skills")];
    if let Some(home) = crate::home::home_dir() {
        dirs.push(home.join(".agents").join("skills"));
    }
    let mut skills: Vec<SkillRow> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if skills.iter().any(|known| known.name == name) {
                continue;
            }
            let description = std::fs::read_to_string(path.join("SKILL.md"))
                .map(|text| skill_description(&text))
                .unwrap_or_default();
            skills.push(SkillRow {
                name: name.to_string(),
                description,
            });
        }
    }
    skills
}

/// The `description:` field of a `SKILL.md` frontmatter block, on one line.
///
/// Covers the two shapes the installed skills use — an inline value and a
/// folded block (`description: >`) — and stops at the next key. A description
/// that cannot be read is empty, not an error: the palette still offers the
/// skill.
fn skill_description(text: &str) -> String {
    let mut lines = text.lines();
    let Some(first) = lines.next() else {
        return String::new();
    };
    if first.trim() != "---" {
        return String::new();
    }
    let mut collecting = false;
    let mut parts: Vec<String> = Vec::new();
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        let trimmed = line.trim_start();
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if collecting {
            if indented && !trimmed.is_empty() {
                parts.push(trimmed.to_string());
                continue;
            }
            if trimmed.is_empty() {
                continue;
            }
            collecting = false;
        }
        if let Some(value) = trimmed.strip_prefix("description:") {
            let value = value.trim();
            if value.is_empty() || value == ">" || value == "|" || value == ">-" || value == "|-" {
                collecting = true;
            } else {
                parts.push(value.to_string());
                break;
            }
        }
    }
    let joined = parts.join(" ");
    let joined = joined.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&joined, 96)
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// The `@` menu's candidates come from the working directory: a bounded
    /// list, the same one for every reader of the same tree, and the directories
    /// every project has (build output, caches) are never in it.
    #[test]
    fn workspace_files_lists_the_tree_and_prunes_build_output() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("README.md"), "x");
        write(&dir.path().join("src/main.rs"), "x");
        write(&dir.path().join("src/nested/deep.rs"), "x");
        write(&dir.path().join("target/debug/junk"), "x");
        write(&dir.path().join("node_modules/pkg/index.js"), "x");
        write(&dir.path().join(".hidden"), "x");
        let files = workspace_files(dir.path());
        // `workspace_files` reports each file relative to `cwd` in the platform's
        // own form (`src\main.rs` on Windows), because that is what the menu
        // hands back to the reader to type. Join the expectation the same way
        // rather than pasting a `/` the walk never produces.
        let relative = |parts: &[&str]| {
            let mut path = std::path::PathBuf::new();
            for part in parts {
                path.push(part);
            }
            path.display().to_string()
        };
        assert!(files.contains(&relative(&["README.md"])), "{files:?}");
        assert!(files.contains(&relative(&["src", "main.rs"])), "{files:?}");
        assert!(
            files.contains(&relative(&["src", "nested", "deep.rs"])),
            "the walk descends: {files:?}"
        );
        // Compared by path components, so this is not a vacuous pass on Windows
        // (where `target\debug\junk`.starts_with("target/") is false because of
        // the separator, not because the directory was pruned).
        assert!(
            !files
                .iter()
                .any(|file| std::path::Path::new(file).starts_with("target")),
            "build output is pruned: {files:?}"
        );
        assert!(
            !files
                .iter()
                .any(|file| std::path::Path::new(file).starts_with("node_modules")),
            "{files:?}"
        );
        assert!(
            !files.iter().any(|file| file.starts_with('.')),
            "dotfiles stay out: {files:?}"
        );
        // Sorted, so the menu's order does not depend on the filesystem.
        let mut sorted = files.clone();
        sorted.sort();
        assert_eq!(files, sorted);
        // A directory that is not there is an empty list, not a panic.
        assert!(workspace_files(&dir.path().join("missing")).is_empty());
        assert!(workspace_files(Path::new("")).is_empty());
    }

    /// The fallback is only the fallback: inside a real repository the answer is
    /// `git ls-files`, so an ignored file never reaches the menu.
    #[test]
    fn a_repository_answers_with_its_tracked_and_unignored_files() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join(".gitignore"), "ignored.txt\n");
        write(&dir.path().join("kept.txt"), "x");
        write(&dir.path().join("ignored.txt"), "x");
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .output()
                .expect("run git")
        };
        let ok = git(&["init", "--quiet"]).status.success();
        assert!(ok, "git init failed");
        let files = workspace_files(dir.path());
        assert!(files.contains(&"kept.txt".to_string()), "{files:?}");
        assert!(
            !files.contains(&"ignored.txt".to_string()),
            "the ignore rules are what keep build output out: {files:?}"
        );
    }

    #[test]
    fn a_skill_description_is_one_line_from_either_frontmatter_shape() {
        let inline = "---\nname: a\ndescription: does a thing\n---\n# body\n";
        assert_eq!(skill_description(inline), "does a thing");
        let folded = "---\nname: b\ndescription: >\n  first half\n  second half\nother: x\n---\n";
        assert_eq!(skill_description(folded), "first half second half");
        assert_eq!(skill_description("no frontmatter"), "");
        let long = format!("---\ndescription: {}\n---\n", "字".repeat(400));
        assert!(skill_description(&long).chars().count() <= 96);
    }
}
