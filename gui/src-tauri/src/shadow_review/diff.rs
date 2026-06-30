//! Eager diff materialization (§7.1): a single `git diff` + a single `numstat`
//! between the before/after commits, split per file and persisted into SQLite
//! so the result no longer depends on shadow objects surviving.

use crate::git_diff_parse::{parse_numstat, split_unified_patch_by_path};
use crate::store::InsertReviewFileChangeInput;
use crate::AppError;

use super::policy::Limits;
use super::repository::ShadowRepo;

/// Per-file change rows plus the changeset-level totals for a Run (§7.3).
#[derive(Debug, Default)]
pub struct MaterializedDiff {
    pub files: Vec<InsertReviewFileChangeInput>,
    pub files_changed: i64,
    pub additions: i64,
    pub deletions: i64,
    pub binary_files: i64,
}

/// Materialize the before..after diff. `core.quotePath=false` keeps header
/// paths literal so they match the `-z` name-status paths.
pub fn materialize(
    repo: &ShadowRepo,
    before_commit: &str,
    after_commit: &str,
    limits: &Limits,
) -> Result<MaterializedDiff, AppError> {
    let entries = name_status(repo, before_commit, after_commit)?;
    let stats = numstat(repo, before_commit, after_commit)?;
    let patches = split_unified_patch_by_path(&unified_patch(repo, before_commit, after_commit)?);

    let mut out = MaterializedDiff::default();
    for (index, entry) in entries.iter().enumerate() {
        let (additions, deletions, binary) = stats.get(index).copied().unwrap_or((0, 0, false));
        let diff_text = if binary {
            None
        } else {
            patches.get(&entry.path).cloned()
        };
        let (diff, diff_truncated) = match diff_text {
            Some(text) => truncate_diff(text, limits),
            None => (None, false),
        };

        out.additions += additions;
        out.deletions += deletions;

        // Binary files carry before/after size + MIME instead of a text diff.
        let (before_size, after_size, mime) = if binary {
            out.binary_files += 1;
            let before_path = entry.previous_path.as_deref().unwrap_or(&entry.path);
            (
                blob_size(repo, before_commit, before_path),
                blob_size(repo, after_commit, &entry.path),
                guess_mime(&entry.path),
            )
        } else {
            (None, None, None)
        };

        out.files.push(InsertReviewFileChangeInput {
            path: Some(entry.path.clone()),
            previous_path: entry.previous_path.clone(),
            change_type: entry.change_type.clone(),
            diff,
            summary: None,
            additions,
            deletions,
            binary,
            before_size,
            after_size,
            mime,
            diff_truncated,
            omission_reason: if binary {
                Some("binary".to_string())
            } else {
                None
            },
        });
    }
    out.files_changed = out.files.len() as i64;
    Ok(out)
}

/// Size in bytes of `path` at `commit`, or `None` if it doesn't exist there
/// (e.g. an added file has no before blob).
fn blob_size(repo: &ShadowRepo, commit: &str, path: &str) -> Option<i64> {
    let spec = format!("{commit}:{path}");
    repo.git(&["cat-file", "-s", &spec], None)
        .ok()
        .and_then(|out| out.trim().parse().ok())
}

/// Best-effort MIME from the file extension; `None` when unknown.
fn guess_mime(path: &str) -> Option<String> {
    let ext = path.rsplit('.').next()?.to_ascii_lowercase();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "wasm" => "application/wasm",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        _ => return None,
    };
    Some(mime.to_string())
}

struct FileEntry {
    change_type: String,
    path: String,
    previous_path: Option<String>,
}

/// `--name-status -z`: authoritative file list, change types and rename pairs.
fn name_status(repo: &ShadowRepo, before: &str, after: &str) -> Result<Vec<FileEntry>, AppError> {
    let bytes = repo.git_bytes(
        &[
            "-c",
            "core.quotePath=false",
            "diff",
            "--no-color",
            "--find-renames",
            "--find-copies",
            "--name-status",
            "-z",
            before,
            after,
        ],
        None,
    )?;

    let tokens: Vec<String> = bytes
        .split(|b| *b == 0)
        .filter(|t| !t.is_empty())
        .map(|t| String::from_utf8_lossy(t).into_owned())
        .collect();
    Ok(parse_name_status(&tokens))
}

/// Parse `--name-status -z` tokens. R/C records carry old+new paths; everything
/// else carries a single path.
fn parse_name_status(tokens: &[String]) -> Vec<FileEntry> {
    let mut entries = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let code = tokens[i].chars().next().unwrap_or('M');
        if matches!(code, 'R' | 'C') {
            if i + 2 >= tokens.len() {
                break;
            }
            entries.push(FileEntry {
                change_type: code.to_string(),
                previous_path: Some(tokens[i + 1].clone()),
                path: tokens[i + 2].clone(),
            });
            i += 3;
        } else {
            if i + 1 >= tokens.len() {
                break;
            }
            entries.push(FileEntry {
                change_type: code.to_string(),
                previous_path: None,
                path: tokens[i + 1].clone(),
            });
            i += 2;
        }
    }
    entries
}

/// `--numstat` (line-based), parsed positionally to match `name_status` order.
/// Only the counts matter here; `-` marks a binary file.
fn numstat(
    repo: &ShadowRepo,
    before: &str,
    after: &str,
) -> Result<Vec<(i64, i64, bool)>, AppError> {
    let text = repo.git(
        &[
            "diff",
            "--no-color",
            "--find-renames",
            "--find-copies",
            "--numstat",
            before,
            after,
        ],
        None,
    )?;
    // The shadow pipeline matches stats to `name_status` entries positionally and
    // only needs the counts + binary flag, so drop the row paths here.
    Ok(parse_numstat(&text)
        .into_iter()
        .map(|row| (row.additions, row.deletions, row.binary))
        .collect())
}

fn unified_patch(repo: &ShadowRepo, before: &str, after: &str) -> Result<String, AppError> {
    let bytes = repo.git_bytes(
        &[
            "-c",
            "core.quotePath=false",
            "diff",
            "--no-color",
            "--find-renames",
            "--find-copies",
            "--unified=3",
            before,
            after,
        ],
        None,
    )?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn truncate_diff(text: String, limits: &Limits) -> (Option<String>, bool) {
    let too_many_lines = text.lines().count() > limits.max_diff_lines;
    let too_many_bytes = text.len() > limits.max_diff_bytes;
    if !too_many_lines && !too_many_bytes {
        return (Some(text), false);
    }
    let mut kept: String = text
        .lines()
        .take(limits.max_diff_lines)
        .collect::<Vec<_>>()
        .join("\n");
    if kept.len() > limits.max_diff_bytes {
        // Truncate on a char boundary — String::truncate panics if the cut lands
        // mid-codepoint (multibyte text in the diff).
        let mut cut = limits.max_diff_bytes;
        while cut > 0 && !kept.is_char_boundary(cut) {
            cut -= 1;
        }
        kept.truncate(cut);
    }
    (Some(kept), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_status_parses_renames_and_plain() {
        let tokens: Vec<String> = ["M", "src/a.rs", "A", "src/b.rs", "R100", "old.rs", "new.rs"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let entries = parse_name_status(&tokens);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].change_type, "M");
        assert_eq!(entries[0].path, "src/a.rs");
        assert_eq!(entries[2].change_type, "R");
        assert_eq!(entries[2].previous_path.as_deref(), Some("old.rs"));
        assert_eq!(entries[2].path, "new.rs");
    }

    #[test]
    fn truncate_marks_oversized() {
        let limits = Limits {
            max_diff_lines: 2,
            ..Limits::default()
        };
        let (text, truncated) = truncate_diff("a\nb\nc\nd".to_string(), &limits);
        assert!(truncated);
        assert_eq!(text.as_deref(), Some("a\nb"));
    }
}
