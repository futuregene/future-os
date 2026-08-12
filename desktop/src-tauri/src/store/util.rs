//! Pure storage helpers: id/timestamp generation, mode/path normalization, and
//! filesystem counting. No database access lives here.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use rand::RngCore;

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
    let now = chrono::Local::now();
    let ts = now.format("%Y%m%d-%H%M%S").to_string();
    let mut rng = rand::thread_rng();
    let mut buf = [0u8; 3];
    rng.fill_bytes(&mut buf);
    let hex: String = buf.iter().map(|b| format!("{:02x}", b)).collect();
    format!("{prefix}-{ts}-{hex}")
}

pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

/// Recover a poisoned mutex guard. The store's in-memory caches/buffers are
/// derived state (the Agent journal / SQLite are authoritative), so a panic
/// while a guard was held must not permanently degrade later reads — the
/// contents are still structurally valid. Shared so each call site stays a
/// single `unwrap_or_else(unpoison)` line (a per-site closure's zero-count
/// region would strand the line).
pub(super) fn unpoison<T>(error: std::sync::PoisonError<T>) -> T {
    error.into_inner()
}

/// Set by store write paths that change the remote directory snapshot (the
/// thread/workspace lists, or a run's streaming state). The remote presence
/// heartbeat drains this flag to publish an immediate full snapshot. Correctness
/// does NOT depend on it — the heartbeat also recomputes the snapshot signature
/// every 20s — so a missed mark only delays propagation by up to one heartbeat.
static CATALOG_DIRTY: AtomicBool = AtomicBool::new(false);

pub fn mark_catalog_dirty() {
    CATALOG_DIRTY.store(true, Ordering::Release);
}

pub fn take_catalog_dirty() -> bool {
    CATALOG_DIRTY.swap(false, Ordering::AcqRel)
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

    count_files_under(vec![root])
}

/// Iterative walk over `stack`, counting regular files. Split from
/// [`count_workspace_files`] so tests can seed a stack that aliases one
/// directory twice (real symlinked dirs are never pushed — entries report
/// symlinks by their own type — so the canonical-dedup guard is otherwise
/// unreachable from the public entry point).
fn count_files_under(mut stack: Vec<PathBuf>) -> Result<i64, crate::AppError> {
    let mut count = 0_i64;
    let mut visited_dirs = HashSet::new();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualify_columns_prefixes_each_column() {
        assert_eq!(qualify_columns("r", "id, status"), "r.id, r.status");
    }

    #[test]
    fn normalize_mode_accepts_known_modes() {
        assert_eq!(normalize_mode("chat").unwrap(), "chat");
        assert_eq!(normalize_mode("workspace").unwrap(), "workspace");
    }

    #[test]
    fn normalize_mode_rejects_unknown_mode() {
        let error = normalize_mode("party").unwrap_err();
        assert!(error.to_string().contains("'chat' or 'workspace'"));
    }

    #[test]
    fn expand_tilde_handles_bare_tilde() {
        let _home = crate::auth_store::test_support::HomeGuard::new("util_tilde");
        let home = std::env::var("HOME").unwrap();
        assert_eq!(expand_tilde("~").unwrap(), PathBuf::from(home));
    }

    #[test]
    fn expand_tilde_joins_suffix() {
        let _home = crate::auth_store::test_support::HomeGuard::new("util_tilde_join");
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            expand_tilde("~/docs/x.md").unwrap(),
            PathBuf::from(home).join("docs/x.md")
        );
    }

    #[test]
    fn expand_tilde_passes_other_paths_through() {
        assert_eq!(expand_tilde("/abs/path").unwrap(), PathBuf::from("/abs/path"));
        assert_eq!(expand_tilde("rel/path").unwrap(), PathBuf::from("rel/path"));
    }

    #[test]
    fn workspace_name_from_path_uses_last_component() {
        assert_eq!(
            workspace_name_from_path(Path::new("/home/u/projects/demo")),
            "demo"
        );
    }

    #[test]
    fn workspace_name_from_path_falls_back_when_no_file_name() {
        assert_eq!(workspace_name_from_path(Path::new("/")), "Workspace");
    }

    #[test]
    fn catalog_dirty_flag_marks_and_drains() {
        // Drain any mark left by earlier tests in this process.
        let _ = take_catalog_dirty();
        mark_catalog_dirty();
        assert!(take_catalog_dirty(), "first take observes the mark");
        assert!(!take_catalog_dirty(), "second take sees the drained flag");
    }

    #[test]
    fn unpoison_recovers_a_poisoned_guard() {
        let mutex = std::sync::Mutex::new(7_i32);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = mutex.lock().expect("lock");
            panic!("intentional: poison the mutex for the recovery test");
        }));
        let guard = mutex.lock().unwrap_or_else(unpoison);
        assert_eq!(*guard, 7, "contents survive the poison");
    }

    #[test]
    fn loaded_unwraps_some_and_errors_on_none() {
        assert_eq!(loaded(Some(7), "Seven").unwrap(), 7);
        let error = loaded::<i32>(None, "Created thread").unwrap_err();
        assert_eq!(error.to_string(), "Created thread could not be loaded.");
    }

    #[test]
    fn count_workspace_files_missing_path_is_zero() {
        let missing = std::env::temp_dir().join(format!(
            "futureos-util-missing-{}",
            std::process::id()
        ));
        assert_eq!(count_workspace_files(missing.to_str().unwrap()).unwrap(), 0);
    }

    #[test]
    fn count_workspace_files_plain_file_is_zero() {
        let dir = std::env::temp_dir().join(format!("futureos-util-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("one.txt");
        fs::write(&file, b"x").unwrap();
        assert_eq!(count_workspace_files(file.to_str().unwrap()).unwrap(), 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn count_workspace_files_walks_dirs_skipping_symlinks_and_cycles() {
        let dir = std::env::temp_dir().join(format!("futureos-util-tree-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let sub = dir.join("sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(dir.join("a.txt"), b"a").unwrap();
        fs::write(sub.join("b.txt"), b"b").unwrap();
        // A symlink to a file is skipped, not counted.
        std::os::unix::fs::symlink(dir.join("a.txt"), dir.join("link-file")).unwrap();
        // A symlinked directory is skipped entirely (its own files uncounted)…
        std::os::unix::fs::symlink(&sub, dir.join("link-dir")).unwrap();
        // …and a self-referential symlinked dir can't loop the walk. Hard links
        // to dirs don't exist, so exercise the canonical-dedup continue via a
        // symlink chain is not possible; instead ensure the count is exact.
        assert_eq!(count_workspace_files(dir.to_str().unwrap()).unwrap(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn count_files_under_dedupes_aliased_dirs() {
        let dir = std::env::temp_dir().join(format!("futureos-util-dup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.txt"), b"a").unwrap();
        // The same directory twice on the stack: the canonical-dedup guard
        // must count its files once.
        assert_eq!(
            count_files_under(vec![dir.clone(), dir.clone()]).unwrap(),
            1
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
