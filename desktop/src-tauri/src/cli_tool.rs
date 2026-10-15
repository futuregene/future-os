//! CLI tool discovery is separate from Agent ownership. Never execute an empty
//! externalBin placeholder, search the current directory or invoke a shell.
use std::path::{Path, PathBuf};

fn usable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !path.is_absolute() || !metadata.is_file() || metadata.len() == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return false;
        }
    }
    true
}

pub(crate) fn resolve() -> Result<PathBuf, crate::AppError> {
    let executable = std::env::current_exe()?;
    let parent = executable
        .parent()
        .ok_or("Desktop executable has no parent directory")?;
    let name = if cfg!(windows) {
        "future.exe"
    } else {
        "future"
    };
    resolve_from(parent, name, std::env::var_os("PATH").as_deref())
        .ok_or_else(|| "No usable future CLI found. Install the matching CLI on PATH or beside Desktop; CLI-dependent skills and maintenance are unavailable.".into())
}

fn resolve_from(parent: &Path, name: &str, path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let mut candidates = vec![parent.join(name)];
    // Compile-time checkout paths are development-only, never a packaged fallback.
    #[cfg(all(debug_assertions, not(test)))]
    {
        if let Some(target) = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
        {
            candidates.push(target.join("debug").join(name));
        }
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        candidates.push(manifest.join("../../target/debug").join(name));
        candidates.push(manifest.join("../../target/release").join(name));
        let ext = if cfg!(windows) { ".exe" } else { "" };
        candidates.push(
            manifest
                .join("binaries")
                .join(format!("future-{}{ext}", env!("FUTURE_TARGET_TRIPLE"))),
        );
    }
    if let Some(path) = path {
        candidates.extend(
            std::env::split_paths(path)
                .filter(|p| p.is_absolute())
                .map(|p| p.join(name)),
        );
    }
    candidates.into_iter().find(|p| usable(p))
}

/// Called only before the host starts threads, after login-shell hydration.
pub(crate) fn prepare_path() -> Result<(), crate::AppError> {
    let cli = match resolve() {
        Ok(cli) => cli,
        Err(error) => {
            eprintln!("FutureOS: {error}");
            return Ok(());
        }
    };
    let parent = cli
        .parent()
        .ok_or("CLI executable has no parent directory")?;
    let mut directories = vec![parent.to_path_buf()];
    if let Some(path) = std::env::var_os("PATH") {
        directories.extend(std::env::split_paths(&path).filter(|p| p != parent));
    }
    std::env::set_var(
        "PATH",
        std::env::join_paths(directories).map_err(|e| e.to_string())?,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // These fixtures are inspected as files, never executed (including Windows).
    fn tool(directory: &Path) -> PathBuf {
        std::fs::create_dir_all(directory).unwrap();
        let path = directory.join("future");
        std::fs::write(&path, b"CLI discovery fixture").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    #[test]
    fn bundled_cli_is_preferred_over_path_cli() {
        let root = tempfile::tempdir().unwrap();
        let bundled = tool(&root.path().join("bundle"));
        let installed = tool(&root.path().join("installed"));
        let path = std::env::join_paths([installed.parent().unwrap()]).unwrap();
        assert_eq!(
            resolve_from(bundled.parent().unwrap(), "future", Some(&path)),
            Some(bundled)
        );
    }

    #[test]
    fn placeholders_and_directories_do_not_hide_a_usable_path_cli() {
        let root = tempfile::tempdir().unwrap();
        let bundle = root.path().join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(bundle.join("future"), []).unwrap();
        let directory = root.path().join("directory");
        std::fs::create_dir_all(directory.join("future")).unwrap();
        let installed = tool(&root.path().join("installed"));
        let invalid_path = std::env::join_paths([directory.as_path()]).unwrap();
        assert_eq!(resolve_from(&bundle, "future", Some(&invalid_path)), None);
        let path =
            std::env::join_paths([directory.as_path(), installed.parent().unwrap()]).unwrap();
        assert_eq!(
            resolve_from(&bundle, "future", Some(&path)),
            Some(installed)
        );
    }

    #[test]
    fn relative_path_entries_are_never_searched() {
        let root = tempfile::tempdir().unwrap();
        let installed = tool(&root.path().join("installed"));
        let relative = std::env::join_paths([Path::new("."), Path::new("installed")]).unwrap();
        assert_eq!(resolve_from(root.path(), "future", Some(&relative)), None);
        let path = std::env::join_paths([Path::new("."), installed.parent().unwrap()]).unwrap();
        assert_eq!(
            resolve_from(root.path(), "future", Some(&path)),
            Some(installed)
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_executable_bundle_does_not_hide_a_usable_path_cli() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let bundled = tool(&root.path().join("bundle"));
        std::fs::set_permissions(&bundled, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            resolve_from(bundled.parent().unwrap(), "future", None),
            None
        );
        let installed = tool(&root.path().join("installed"));
        let path = std::env::join_paths([installed.parent().unwrap()]).unwrap();
        assert_eq!(
            resolve_from(bundled.parent().unwrap(), "future", Some(&path)),
            Some(installed)
        );
    }
}
