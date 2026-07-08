//! Workspace file search backing the composer's `@`-mention picker. Walks the
//! workspace directory (respecting `.gitignore`, skipping hidden/VCS/`.future`
//! and heavy vendor dirs) and fuzzy-ranks files against the query. A file the
//! user picks becomes a plain markdown path link (`[name](./relative-path)`),
//! resolved back to a display path by [`super::markdown_refs`].

use std::path::Path;
use std::time::SystemTime;

use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use ignore::WalkBuilder;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::db::connect;

/// Upper bound on files walked, so a pathological tree can't stall the picker.
const MAX_WALK_ENTRIES: usize = 20_000;
/// Results returned when the caller doesn't specify a limit.
const DEFAULT_LIMIT: usize = 20;
/// Directories always skipped, even when `.gitignore` doesn't list them.
const ALWAYS_SKIP: &[&str] = &[".git", ".future", "node_modules", "target"];

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileSearchInput {
    pub workspace_id: String,
    pub query: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileResult {
    /// Path relative to the workspace root (POSIX-style separators).
    pub path: String,
    /// Last path component, for display emphasis.
    pub name: String,
}

struct WalkedFile {
    rel: String,
    modified: SystemTime,
}

/// Search files under a workspace by fuzzy-matching `query` against their
/// workspace-relative paths. An empty query returns the most-recently-modified
/// files. A missing workspace, or a path that isn't a directory on disk (e.g. a
/// cleaned temporary workspace), yields an empty list rather than an error.
pub fn search_workspace_files(
    input: WorkspaceFileSearchInput,
) -> Result<Vec<WorkspaceFileResult>, crate::AppError> {
    let conn = connect()?;
    let workspace_path: Option<String> = conn
        .query_row(
            "SELECT path FROM workspaces WHERE id = ?1",
            params![input.workspace_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(crate::AppError::from)?;

    let Some(root) = workspace_path else {
        return Ok(Vec::new());
    };
    let root = Path::new(&root);
    if !root.is_dir() {
        return Ok(Vec::new());
    }

    let limit = input
        .limit
        .map(|value| value.clamp(1, 100) as usize)
        .unwrap_or(DEFAULT_LIMIT);
    let query = input.query.unwrap_or_default();

    Ok(rank_files(walk_workspace_files(root), query.trim(), limit))
}

/// Collect workspace files (relative path + mtime), honoring ignore rules.
fn walk_workspace_files(root: &Path) -> Vec<WalkedFile> {
    let walker = WalkBuilder::new(root)
        .hidden(true) // skip dotfiles/dirs
        .parents(true) // honor ignore files in parent dirs too
        .require_git(false) // apply .gitignore even outside a git repo
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            !ALWAYS_SKIP.contains(&name.as_ref())
        })
        .build();

    let mut files = Vec::new();
    for entry in walker.flatten() {
        if files.len() >= MAX_WALK_ENTRIES {
            break;
        }
        if !entry
            .file_type()
            .is_some_and(|file_type| file_type.is_file())
        {
            continue;
        }
        let Ok(relative) = entry.path().strip_prefix(root) else {
            continue;
        };
        let rel = relative.to_string_lossy().replace('\\', "/");
        if rel.is_empty() {
            continue;
        }
        let modified = entry
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        files.push(WalkedFile { rel, modified });
    }
    files
}

/// Empty query → most-recently-modified first; otherwise fuzzy-rank by path.
fn rank_files(files: Vec<WalkedFile>, query: &str, limit: usize) -> Vec<WorkspaceFileResult> {
    if query.is_empty() {
        let mut files = files;
        files.sort_by_key(|file| std::cmp::Reverse(file.modified));
        return files.into_iter().take(limit).map(to_result).collect();
    }

    let matcher = SkimMatcherV2::default();
    let mut scored: Vec<(i64, WalkedFile)> = files
        .into_iter()
        .filter_map(|file| {
            matcher
                .fuzzy_match(&file.rel, query)
                .map(|score| (score, file))
        })
        .collect();
    // Higher score first; tie-break on shorter path, then lexical order.
    scored.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.rel.len().cmp(&right.1.rel.len()))
            .then_with(|| left.1.rel.cmp(&right.1.rel))
    });
    scored
        .into_iter()
        .take(limit)
        .map(|(_, file)| to_result(file))
        .collect()
}

fn to_result(file: WalkedFile) -> WorkspaceFileResult {
    let name = Path::new(&file.rel)
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| file.rel.clone());
    WorkspaceFileResult {
        path: file.rel,
        name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// Unique temp dir for one test; cleaned up on drop.
    struct TempTree(PathBuf);

    impl TempTree {
        fn new(tag: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("futureos-fs-{}-{}", tag, std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("create temp dir");
            TempTree(dir)
        }

        fn write(&self, rel: &str, contents: &str) {
            let path = self.0.join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent");
            }
            fs::write(path, contents).expect("write file");
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn paths(results: &[WorkspaceFileResult]) -> Vec<&str> {
        results.iter().map(|r| r.path.as_str()).collect()
    }

    #[test]
    fn respects_gitignore_and_always_skip_dirs() {
        let tree = TempTree::new("ignore");
        tree.write(".gitignore", "ignored.txt\ndist/\n");
        tree.write("keep.md", "hi");
        tree.write("ignored.txt", "no");
        tree.write("dist/bundle.js", "no");
        tree.write("node_modules/pkg/index.js", "no");
        tree.write(".future/agent/settings.json", "no");
        tree.write(".hidden", "no");

        let results = rank_files(walk_workspace_files(&tree.0), "", 50);
        let found = paths(&results);
        assert!(found.contains(&"keep.md"), "keep.md present: {found:?}");
        assert!(!found.contains(&"ignored.txt"), "gitignore file excluded");
        assert!(
            !found.iter().any(|p| p.starts_with("dist/")),
            "gitignore dir excluded"
        );
        assert!(
            !found.iter().any(|p| p.starts_with("node_modules/")),
            "node_modules excluded"
        );
        assert!(
            !found.iter().any(|p| p.starts_with(".future/")),
            ".future excluded"
        );
        assert!(!found.contains(&".hidden"), "hidden file excluded");
    }

    #[test]
    fn fuzzy_ranks_and_returns_name() {
        let tree = TempTree::new("fuzzy");
        tree.write("src/composer.tsx", "");
        tree.write("src/deep/other.ts", "");
        tree.write("README.md", "");

        let results = rank_files(walk_workspace_files(&tree.0), "composer", 10);
        assert_eq!(
            results.first().map(|r| r.path.as_str()),
            Some("src/composer.tsx")
        );
        assert_eq!(results[0].name, "composer.tsx");
        assert!(
            !paths(&results).contains(&"README.md"),
            "non-match filtered out"
        );
    }

    #[test]
    fn empty_query_orders_by_recency() {
        let tree = TempTree::new("recency");
        tree.write("old.md", "");
        tree.write("new.md", "");
        // Make new.md strictly newer than old.md regardless of write timing.
        let later = SystemTime::now() + std::time::Duration::from_secs(5);
        filetime_set(&tree.0.join("new.md"), later);

        let results = rank_files(walk_workspace_files(&tree.0), "", 10);
        assert_eq!(results.first().map(|r| r.path.as_str()), Some("new.md"));
    }

    /// Bump a file's mtime without pulling in the `filetime` crate.
    fn filetime_set(path: &Path, when: SystemTime) {
        let file = fs::OpenOptions::new().write(true).open(path).expect("open");
        file.set_modified(when).expect("set mtime");
    }
}
