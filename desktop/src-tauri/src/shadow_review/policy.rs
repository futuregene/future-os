//! Snapshot inclusion policy: per-round size limits, the non-git default
//! excludes, sensitive-path handling, and the non-git volume red line
//! (§5.5, §6.7, §13).

use std::fs;
use std::path::{Path, PathBuf};

/// Per-round candidate-set limits (§5.5). These constrain *this round's*
/// changed candidate set and the bytes actually read/hashed — NOT the whole
/// tree, so a large monorepo with small changes is never marked `partial`.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_file_bytes: u64,
    pub max_candidate_files: usize,
    pub max_total_bytes: u64,
    pub max_diff_bytes: usize,
    pub max_diff_lines: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_bytes: 20 * 1024 * 1024,    // 20 MiB
            max_candidate_files: 50_000,         // per-round candidate set
            max_total_bytes: 1024 * 1024 * 1024, // 1 GiB read this round
            max_diff_bytes: 2 * 1024 * 1024,     // 2 MiB shown per file
            max_diff_lines: 10_000,
        }
    }
}

/// Workspace-level volume gate for non-git Workspaces (§6.7). A directory over
/// either threshold disables change preview rather than blocking every prompt.
#[derive(Debug, Clone, Copy)]
pub struct VolumeRedline {
    pub max_files: usize,
    pub max_bytes: u64,
}

impl Default for VolumeRedline {
    fn default() -> Self {
        Self {
            max_files: 20_000,
            max_bytes: 512 * 1024 * 1024, // 512 MiB
        }
    }
}

/// Directories never worth snapshotting, and a built-in fallback for non-git
/// Workspaces that lack a `.gitignore` (§5.5).
pub const NON_GIT_DEFAULT_EXCLUDES: &[&str] = &[
    "node_modules/",
    ".venv/",
    "venv/",
    "target/",
    "dist/",
    "build/",
    ".cache/",
    "coverage/",
];

/// Always excluded regardless of Workspace kind (§5.5).
const ALWAYS_EXCLUDES: &[&str] = &[".git/", ".future/"];

/// Whether a workspace-relative path is a sensitive credential file (§13):
/// `.env`, `.env.*`, `*.pem`, `*.key`, `id_rsa`, `id_ed25519`. Detected but
/// never stored as blob/diff; Phase 1 keeps it a basic exclusion (the file is
/// counted as omitted), richer metadata rows land in Phase 2.
pub fn is_sensitive(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    if matches!(name, "id_rsa" | "id_ed25519" | ".env") {
        return true;
    }
    name.starts_with(".env.") || name.ends_with(".pem") || name.ends_with(".key")
}

/// How a candidate file should be treated when building the snapshot tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Stage the file into the tree.
    Include,
    /// Omit the blob: file exceeds the per-file size limit.
    Oversized,
    /// Omit the blob: sensitive credential file.
    Sensitive,
}

/// Classify a changed candidate. `size` is the work-tree file size in bytes
/// (use 0 for deletions, which are always included so the removal is captured).
pub fn classify(path: &str, size: u64, limits: &Limits) -> Disposition {
    if is_sensitive(path) {
        return Disposition::Sensitive;
    }
    if size > limits.max_file_bytes {
        return Disposition::Oversized;
    }
    Disposition::Include
}

/// Build the shadow repo's `info/exclude` contents (§5.5): the real repo's
/// `info/exclude` (boundary 1), built-in defaults for non-git Workspaces, plus
/// any extra paths (e.g. oversized untracked files, boundary 3).
pub fn build_info_exclude(
    is_git_workspace: bool,
    real_repo_info_exclude: Option<&str>,
    extra: &[String],
) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for entry in ALWAYS_EXCLUDES {
        lines.push((*entry).to_string());
    }
    if let Some(real) = real_repo_info_exclude {
        for line in real.lines() {
            let line = line.trim_end();
            if !line.is_empty() {
                lines.push(line.to_string());
            }
        }
    }
    if !is_git_workspace {
        for entry in NON_GIT_DEFAULT_EXCLUDES {
            lines.push((*entry).to_string());
        }
    }
    for entry in extra {
        // Anchor extras to the work-tree root so only the exact path is excluded.
        lines.push(format!("/{}", entry.trim_start_matches('/')));
    }
    lines
}

/// Result of the non-git volume gate (§6.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeVerdict {
    Ok,
    TooLarge,
}

/// Walk the Workspace counting files/bytes with an early exit once either
/// threshold is crossed (§6.7). Skips always-excluded and default-excluded
/// directories so junk doesn't dominate the count. Approximate by design — it
/// is a gate, not the snapshot itself.
pub fn evaluate_volume(workspace_path: &Path, redline: &VolumeRedline) -> VolumeVerdict {
    let mut files = 0usize;
    let mut bytes = 0u64;
    let mut stack: Vec<PathBuf> = vec![workspace_path.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if file_type.is_dir() {
                if is_excluded_dir(&name) {
                    continue;
                }
                stack.push(path);
            } else if file_type.is_file() {
                files += 1;
                if let Ok(meta) = entry.metadata() {
                    bytes += meta.len();
                }
                if files > redline.max_files || bytes > redline.max_bytes {
                    return VolumeVerdict::TooLarge;
                }
            }
        }
    }
    VolumeVerdict::Ok
}

fn is_excluded_dir(name: &str) -> bool {
    if name == ".git" || name == ".future" {
        return true;
    }
    NON_GIT_DEFAULT_EXCLUDES
        .iter()
        .any(|entry| entry.trim_end_matches('/') == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_matches_credentials() {
        assert!(is_sensitive(".env"));
        assert!(is_sensitive("config/.env.production"));
        assert!(is_sensitive("certs/server.pem"));
        assert!(is_sensitive("deploy/id_rsa"));
        assert!(!is_sensitive("src/main.rs"));
        assert!(!is_sensitive("environment.ts"));
    }

    #[test]
    fn classify_respects_size_and_sensitivity() {
        let limits = Limits::default();
        assert_eq!(classify("a.txt", 10, &limits), Disposition::Include);
        assert_eq!(
            classify("big.bin", limits.max_file_bytes + 1, &limits),
            Disposition::Oversized
        );
        assert_eq!(classify(".env", 5, &limits), Disposition::Sensitive);
    }

    #[test]
    fn exclude_lines_include_defaults_for_non_git() {
        let lines = build_info_exclude(false, None, &["huge.bin".to_string()]);
        assert!(lines.iter().any(|l| l == ".git/"));
        assert!(lines.iter().any(|l| l == "node_modules/"));
        assert!(lines.iter().any(|l| l == "/huge.bin"));
        // Git Workspaces don't get the non-git defaults.
        let git_lines = build_info_exclude(true, None, &[]);
        assert!(!git_lines.iter().any(|l| l == "node_modules/"));
    }
}
