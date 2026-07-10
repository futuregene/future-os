//! Pure storage helpers: id/timestamp generation, mode/path normalization, and
//! filesystem counting. No database access lives here.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

/// Prefix each column in a `", "`-separated `*_COLUMNS` constant with a table
/// alias, e.g. `qualify_columns("r", "id, status")` → `"r.id, r.status"`. Used
/// when a JOIN makes bare column names ambiguous in a SELECT.
pub(super) fn qualify_columns(alias: &str, columns: &str) -> String {
    columns
        .split(", ")
        .map(|column| format!("{alias}.{}", column.trim()))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn normalize_mode(mode: &str) -> Result<String, crate::AppError> {
    match mode {
        "chat" | "workspace" => Ok(mode.to_string()),
        _ => Err("mode must be either 'chat' or 'workspace'."
            .to_string()
            .into()),
    }
}

pub(super) fn expand_tilde(path: &str) -> Result<PathBuf, crate::AppError> {
    if path == "~" {
        return Ok(PathBuf::from(
            crate::home_dir().ok_or("HOME/USERPROFILE environment variable is not set.")?,
        ));
    }

    if let Some(rest) = path.strip_prefix("~/") {
        return Ok(PathBuf::from(
            crate::home_dir().ok_or("HOME/USERPROFILE environment variable is not set.")?,
        )
        .join(rest));
    }

    Ok(PathBuf::from(path))
}

pub(super) fn workspace_name_from_path(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Workspace")
        .to_string()
}

pub fn create_id(prefix: &str) -> String {
    static ID_COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let counter = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{nanos}_{counter}")
}

pub(super) fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

/// Turn an "expected to exist" lookup into a hard error when the row is missing.
/// Collapses the `get_X(&id)?.ok_or_else(|| "X could not be loaded.".into())`
/// boilerplate that follows almost every insert/update read-back. `what` names
/// the row (e.g. `"Created thread"`), yielding `"<what> could not be loaded."`.
pub(super) fn loaded<T>(opt: Option<T>, what: &str) -> Result<T, crate::AppError> {
    opt.ok_or_else(|| format!("{what} could not be loaded.").into())
}

pub(super) fn count_workspace_files(path: &str) -> Result<i64, crate::AppError> {
    let root = PathBuf::from(path);
    if !root.exists() {
        return Ok(0);
    }
    if !root.is_dir() {
        return Ok(0);
    }

    let mut count = 0_i64;
    let mut visited_dirs = HashSet::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let canonical_dir = fs::canonicalize(&dir)?;
        if !visited_dirs.insert(canonical_dir) {
            continue;
        }
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                stack.push(entry.path());
            } else if file_type.is_file() {
                count += 1;
            }
        }
    }
    Ok(count)
}
