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
    walk_workspace_files_up_to(root, MAX_WALK_ENTRIES)
}

/// The walk, parameterized on the entry cap so tests can exercise the
/// truncation break without materializing 20k files.
fn walk_workspace_files_up_to(root: &Path, max_entries: usize) -> Vec<WalkedFile> {
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
        if files.len() >= max_entries {
            break;
        }
        if !entry
            .file_type()
            .is_some_and(|file_type| file_type.is_file())
        {
            continue;
        }
        // Single-root walker entries are root-joined by construction, so the
        // strip can't fail.
        let relative = entry
            .path()
            .strip_prefix(root)
            .expect("walker entries are under the walked root");
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
    let query_lower = query.to_lowercase();
    let mut scored: Vec<(u8, i64, SystemTime, WalkedFile)> = files
        .into_iter()
        .filter_map(|file| {
            // Match-quality bucket (lower = better). A hit on the file *name*
            // outranks a hit elsewhere in the path, which outranks a gap-tolerant
            // fuzzy hit — so the obvious best match stays on top instead of a deep
            // path that merely fuzzy-contains the query. Substring tests are
            // case-folded; the fuzzy score (smart-case) is only a tie-break.
            let name = file_name(&file.rel).to_lowercase();
            let bucket = if name == query_lower {
                0
            } else if name.starts_with(&query_lower) {
                1
            } else if name.contains(&query_lower) {
                2
            } else if file.rel.to_lowercase().contains(&query_lower) {
                3
            } else if matcher.fuzzy_match(&file.rel, query).is_some() {
                4
            } else {
                return None;
            };
            let fuzzy = matcher.fuzzy_match(&file.rel, query).unwrap_or(0);
            Some((bucket, fuzzy, file.modified, file))
        })
        .collect();
    // Bucket first, then tighter fuzzy score, then recency, then shorter /
    // lexical path as a stable final tie-break.
    scored.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| right.1.cmp(&left.1))
            .then_with(|| right.2.cmp(&left.2))
            .then_with(|| left.3.rel.len().cmp(&right.3.rel.len()))
            .then_with(|| left.3.rel.cmp(&right.3.rel))
    });
    scored
        .into_iter()
        .take(limit)
        .map(|(_, _, _, file)| to_result(file))
        .collect()
}

/// Last path component of a POSIX-style relative path (the part after the final `/`).
fn file_name(rel: &str) -> &str {
    match rel.rfind('/') {
        Some(index) => &rel[index + 1..],
        None => rel,
    }
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

    #[test]
    fn name_match_beats_path_only_and_fuzzy() {
        let tree = TempTree::new("rank-buckets");
        tree.write("docs/report.md", ""); // name substring  -> bucket 2
        tree.write("reports/notes.md", ""); // path substring  -> bucket 3
        tree.write("src/r_e_port.rs", ""); // fuzzy only      -> bucket 4

        let results = rank_files(walk_workspace_files(&tree.0), "report", 10);
        let p = paths(&results);
        assert_eq!(p.first(), Some(&"docs/report.md"));
        assert_eq!(p.get(1), Some(&"reports/notes.md"));
        assert_eq!(p.get(2), Some(&"src/r_e_port.rs"));
    }

    #[test]
    fn exact_and_prefix_name_matches_outrank_substring() {
        let walked = |rel: &str| WalkedFile {
            rel: rel.to_string(),
            modified: SystemTime::UNIX_EPOCH,
        };
        let files = vec![
            walked("docs/my-report.md"), // name substring -> bucket 2
            walked("docs/report-final.md"), // name prefix   -> bucket 1
            walked("docs/report.md"),    // name prefix (tighter fuzzy score)
        ];
        let results = rank_files(files, "report", 10);
        assert_eq!(
            paths(&results),
            vec!["docs/report.md", "docs/report-final.md", "docs/my-report.md"]
        );

        // A query equal to the full file name hits the exact-match bucket 0.
        let exact = rank_files(vec![walked("docs/report.md")], "report.md", 10);
        assert_eq!(paths(&exact), vec!["docs/report.md"]);
    }

    #[test]
    fn walk_over_a_single_file_root_yields_nothing() {
        // The walker yields the root itself; with a file root, stripping the
        // root prefix leaves an empty relative path, which is skipped.
        let tree = TempTree::new("walk-file-root");
        tree.write("solo.txt", "");
        assert!(walk_workspace_files(&tree.0.join("solo.txt")).is_empty());
    }

    #[test]
    fn walk_truncates_at_the_entry_cap() {
        let tree = TempTree::new("walk-cap");
        tree.write("a.txt", "");
        tree.write("b.txt", "");
        tree.write("c.txt", "");
        assert_eq!(walk_workspace_files_up_to(&tree.0, 2).len(), 2);
    }

    /// Initialized in-test database with one workspace row pointing at `path`.
    fn workspace_conn(path: &Path) -> (super::super::db::PooledConnection, String) {
        let conn = connect().expect("connect");
        super::super::db::apply_schema(&conn).expect("apply schema");        conn.execute(
            "INSERT INTO workspaces (
                 id, name, kind, path, cleanup_status, created_at, updated_at
             ) VALUES ('ws_search', 'WS', 'user', ?1, 'active', 1, 1)",
            params![path.to_string_lossy().into_owned()],
        )
        .expect("insert workspace");
        (conn, "ws_search".to_string())
    }

    #[test]
    fn search_with_missing_workspace_is_empty() {
        let _home = crate::auth_store::test_support::HomeGuard::new("wf_missing_ws");
        let conn = connect().expect("connect");
        super::super::db::apply_schema(&conn).expect("apply schema");
        drop(conn);
        let results = search_workspace_files(WorkspaceFileSearchInput {
            workspace_id: "ghost".to_string(),
            query: None,
            limit: None,
        })
        .expect("search");
        assert!(results.is_empty());
    }

    #[test]
    fn search_with_non_directory_workspace_path_is_empty() {
        let _home = crate::auth_store::test_support::HomeGuard::new("wf_nondir");
        let missing = std::env::temp_dir().join(format!(
            "futureos-wf-nondir-{}",
            std::process::id()
        ));
        let (conn, id) = workspace_conn(&missing);
        drop(conn);
        let results = search_workspace_files(WorkspaceFileSearchInput {
            workspace_id: id,
            query: None,
            limit: None,
        })
        .expect("search");
        assert!(results.is_empty());
    }

    #[test]
    fn search_walks_the_workspace_and_clamps_the_limit() {
        let _home = crate::auth_store::test_support::HomeGuard::new("wf_search");
        let tree = TempTree::new("wf-search");
        tree.write("alpha.md", "");
        tree.write("beta.md", "");
        let (conn, id) = workspace_conn(&tree.0);
        drop(conn);

        // limit clamps into 1..=100: 0 becomes 1 result.
        let one = search_workspace_files(WorkspaceFileSearchInput {
            workspace_id: id.clone(),
            query: Some("md".to_string()),
            limit: Some(0),
        })
        .expect("search");
        assert_eq!(one.len(), 1);

        let all = search_workspace_files(WorkspaceFileSearchInput {
            workspace_id: id,
            query: None,
            limit: Some(500),
        })
        .expect("search");
        assert_eq!(all.len(), 2);
    }
}
