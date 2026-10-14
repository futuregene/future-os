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
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(
            std::env::split_paths(&path)
                .filter(|p| p.is_absolute())
                .map(|p| p.join(name)),
        );
    }
    candidates.into_iter().find(|p| usable(p))
        .ok_or_else(|| "No usable future CLI found. Install the matching CLI on PATH or beside Desktop; CLI-dependent skills and maintenance are unavailable.".into())
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
