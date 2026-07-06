//! Path normalization for sandbox boundary decisions (SANDBOX_PLAN.md §3.5).
//!
//! The application-layer boundary checks (write/edit tools, approval shapes)
//! and the OS sandbox (Seatbelt/bwrap) must agree on what a path *really* is.
//! These helpers resolve `~`, symlinks, non-existent targets, and macOS
//! case-insensitivity so both layers reach the same verdict.

use std::path::{Component, Path, PathBuf};

/// Expand a leading `~` / `~/` to the user's real home directory.
///
/// Note: the legacy behavior joined `~/x` onto the *workspace*, which
/// contradicts what the OS sandbox enforces (real `$HOME/x`). Boundary
/// decisions must use this function instead.
pub fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

/// Resolve `path` to an absolute path, using `base` for relative paths and
/// expanding a leading `~` to the real home directory.
pub fn resolve_against(base: &Path, path: &str) -> PathBuf {
    let expanded = expand_tilde(path);
    if expanded.is_absolute() {
        expanded
    } else {
        base.join(expanded)
    }
}

/// Lexically remove `.` and `..` components (no filesystem access).
fn normalize_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

/// Canonicalize a path that may not exist yet: canonicalize the nearest
/// existing ancestor (resolving symlinks), then re-append the non-existent
/// remainder lexically.
///
/// This makes "write to a new file behind a symlinked directory" resolve to
/// the symlink *target*, matching what the OS sandbox will enforce.
pub fn canonicalize_lenient(path: &Path) -> PathBuf {
    let path = normalize_lexically(path);
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }
    // Walk up to the nearest existing ancestor.
    let mut ancestor = path.as_path();
    let mut remainder: Vec<std::ffi::OsString> = Vec::new();
    loop {
        match ancestor.parent() {
            Some(parent) => {
                if let Some(name) = ancestor.file_name() {
                    remainder.push(name.to_os_string());
                }
                if let Ok(canonical) = parent.canonicalize() {
                    let mut result = canonical;
                    for part in remainder.iter().rev() {
                        result.push(part);
                    }
                    return result;
                }
                ancestor = parent;
            }
            None => return path,
        }
    }
}

/// Whether `path` is `root` itself or lies under `root`.
///
/// Both sides must already be canonicalized (see [`canonicalize_lenient`]).
/// On macOS the default APFS volume is case-insensitive, so the comparison
/// ignores ASCII case there; other platforms compare exactly.
pub fn path_within(path: &Path, root: &Path) -> bool {
    let path_parts: Vec<&std::ffi::OsStr> = path.iter().collect();
    let root_parts: Vec<&std::ffi::OsStr> = root.iter().collect();
    if root_parts.len() > path_parts.len() {
        return false;
    }
    path_parts
        .iter()
        .zip(root_parts.iter())
        .all(|(a, b)| component_eq(a, b))
}

#[cfg(target_os = "macos")]
fn component_eq(a: &std::ffi::OsStr, b: &std::ffi::OsStr) -> bool {
    a == b
        || a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
}

#[cfg(not(target_os = "macos"))]
fn component_eq(a: &std::ffi::OsStr, b: &std::ffi::OsStr) -> bool {
    a == b
}

/// Resolve + canonicalize a candidate path and test it against a set of
/// writable roots (already canonicalized).
pub fn within_any_root(base: &Path, candidate: &str, roots: &[PathBuf]) -> bool {
    let resolved = canonicalize_lenient(&resolve_against(base, candidate));
    roots.iter().any(|root| path_within(&resolved, root))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("futureos-sandbox-paths-{name}-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    #[test]
    fn tilde_expands_to_real_home_not_workspace() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(expand_tilde("~/x.txt"), home.join("x.txt"));
        assert_eq!(expand_tilde("~"), home);
    }

    #[test]
    fn resolve_relative_against_base() {
        let base = temp_dir("base");
        assert_eq!(resolve_against(&base, "a/b.txt"), base.join("a/b.txt"));
        assert_eq!(resolve_against(&base, "/abs/x"), PathBuf::from("/abs/x"));
    }

    #[test]
    fn canonicalize_lenient_resolves_nonexistent_tail() {
        let dir = temp_dir("lenient");
        let missing = dir.join("no/such/file.txt");
        let resolved = canonicalize_lenient(&missing);
        assert!(path_within(&resolved, &dir));
        assert!(resolved.ends_with("no/such/file.txt"));
    }

    #[cfg(unix)]
    #[test]
    fn canonicalize_lenient_follows_symlinked_dir_for_new_files() {
        let target = temp_dir("symlink-target");
        let holder = temp_dir("symlink-holder");
        let link = holder.join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        // A new file "behind" the symlink must resolve into the target dir.
        let resolved = canonicalize_lenient(&link.join("new.txt"));
        assert!(
            path_within(&resolved, &target),
            "expected {resolved:?} within {target:?}"
        );
        assert!(!path_within(&resolved, &holder.join("link")) || target == holder);
    }

    #[test]
    fn dotdot_cannot_escape_root_check() {
        let dir = temp_dir("dotdot");
        let sneaky = dir.join("sub/../../outside.txt");
        let resolved = canonicalize_lenient(&sneaky);
        assert!(!path_within(&resolved, &dir));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_prefix_check_is_case_insensitive() {
        let dir = temp_dir("case");
        let upper = PathBuf::from(dir.to_string_lossy().to_uppercase());
        assert!(path_within(&dir.join("f.txt"), &upper));
    }

    #[test]
    fn prefix_check_requires_whole_components() {
        // /a/bc is NOT within /a/b
        assert!(!path_within(Path::new("/a/bc"), Path::new("/a/b")));
        assert!(path_within(Path::new("/a/b/c"), Path::new("/a/b")));
        assert!(path_within(Path::new("/a/b"), Path::new("/a/b")));
    }

    #[test]
    fn within_any_root_matches_roots() {
        let ws = temp_dir("roots-ws");
        let tmp = temp_dir("roots-tmp");
        let roots = vec![ws.clone(), tmp.clone()];
        assert!(within_any_root(&ws, "inside.txt", &roots));
        assert!(within_any_root(
            &ws,
            tmp.join("t.txt").to_string_lossy().as_ref(),
            &roots
        ));
        assert!(!within_any_root(&ws, "/etc/hosts", &roots));
    }
}
