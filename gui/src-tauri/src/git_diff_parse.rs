//! Shared parsing of `git diff` output: splitting a unified patch into per-file
//! sections and parsing `--numstat`. Used by both the working-tree review
//! (`git_review`) and the shadow-repo diff (`shadow_review::diff`), which
//! previously each carried their own slightly-divergent copies.

use std::collections::HashMap;

/// One `--numstat` row. `binary` is true when git reported `-` for the counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NumstatRow {
    pub additions: i64,
    pub deletions: i64,
    pub path: String,
    pub binary: bool,
}

/// Parse `--numstat` output. Each line is `<add>\t<del>\t<path>`; `-` counts
/// mark a binary file. The path is kept verbatim (rename arrows like
/// `a/{b => c}/d` are the caller's to normalize). Malformed lines are skipped.
pub(crate) fn parse_numstat(text: &str) -> Vec<NumstatRow> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let add = parts.next()?;
            let del = parts.next()?;
            let path = parts.next()?;
            Some(NumstatRow {
                additions: add.parse().unwrap_or(0),
                deletions: del.parse().unwrap_or(0),
                path: path.to_string(),
                binary: add == "-" || del == "-",
            })
        })
        .collect()
}

/// Normalize a `--numstat` path to the post-image (new) path, resolving git's
/// rename/copy arrow forms (`old => new`, `foo/{a => b}.rs`, `{a => b}/c.rs`) so
/// it can be keyed against `--name-status` paths. Non-rename paths pass through.
pub(crate) fn normalize_numstat_path(path: &str) -> String {
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

/// Split a unified patch into `new-path -> patch text`. The post-image path is
/// taken from the `diff --git a/<x> b/<y>` header and overridden by a later
/// `+++ b/<path>` line (authoritative for the new name). Section bodies are
/// trimmed of trailing whitespace.
pub(crate) fn split_unified_patch_by_path(patch: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut current_path: Option<String> = None;
    let mut current = String::new();

    fn flush(path: &Option<String>, body: &str, map: &mut HashMap<String, String>) {
        if let Some(path) = path {
            if !body.is_empty() {
                map.insert(path.clone(), body.trim_end().to_string());
            }
        }
    }

    for line in patch.lines() {
        if line.starts_with("diff --git ") {
            flush(&current_path, &current, &mut map);
            current = String::new();
            current_path = diff_git_new_path(line);
        }
        if let Some(stripped) = line.strip_prefix("+++ b/") {
            current_path = Some(stripped.to_string());
        }
        current.push_str(line);
        current.push('\n');
    }
    flush(&current_path, &current, &mut map);
    map
}

/// Extract the post-image path from a `diff --git a/<x> b/<y>` header.
pub(crate) fn diff_git_new_path(line: &str) -> Option<String> {
    let rest = line.strip_prefix("diff --git ")?;
    let idx = rest.find(" b/")?;
    Some(rest[idx + 3..].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numstat_parses_text_and_binary() {
        let rows = parse_numstat("5\t2\tsrc/a.rs\n-\t-\timg.png\n");
        assert_eq!(
            rows,
            vec![
                NumstatRow {
                    additions: 5,
                    deletions: 2,
                    path: "src/a.rs".to_string(),
                    binary: false,
                },
                NumstatRow {
                    additions: 0,
                    deletions: 0,
                    path: "img.png".to_string(),
                    binary: true,
                },
            ]
        );
    }

    #[test]
    fn numstat_skips_malformed_lines() {
        let rows = parse_numstat("oops\n3\t1\tsrc/b.rs\n");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, "src/b.rs");
    }

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

    #[test]
    fn split_maps_by_new_path() {
        let patch = "diff --git a/src/a.rs b/src/a.rs\n\
                     index 111..222 100644\n\
                     --- a/src/a.rs\n\
                     +++ b/src/a.rs\n\
                     @@ -1 +1 @@\n\
                     -old\n\
                     +new\n\
                     diff --git a/b.txt b/b.txt\n\
                     --- a/b.txt\n\
                     +++ b/b.txt\n\
                     @@ -0,0 +1 @@\n\
                     +hi\n";
        let map = split_unified_patch_by_path(patch);
        assert!(map.contains_key("src/a.rs"));
        assert!(map.contains_key("b.txt"));
        assert!(map["src/a.rs"].contains("+new"));
    }

    #[test]
    fn diff_git_header_new_path() {
        assert_eq!(
            diff_git_new_path("diff --git a/src/x.rs b/src/x.rs").as_deref(),
            Some("src/x.rs")
        );
    }
}
