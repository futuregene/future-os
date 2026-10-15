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

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Descriptor {
    pub path: PathBuf,
    pub source: String,
    pub version: String,
    pub capabilities: future_rpc::cli_capabilities::CliCapabilities,
}

#[derive(Clone, Debug, PartialEq)]
struct Identity {
    path: PathBuf,
    length: u64,
    modified: Option<std::time::SystemTime>,
    created: Option<std::time::SystemTime>,
    #[cfg(unix)]
    inode: (u64, u64, i64, i64),
}
impl Identity {
    fn read(path: &Path) -> std::io::Result<Self> {
        let metadata = path.metadata()?;
        Ok(Self {
            path: path.canonicalize()?,
            length: metadata.len(),
            modified: metadata.modified().ok(),
            created: metadata.created().ok(),
            #[cfg(unix)]
            inode: {
                use std::os::unix::fs::MetadataExt;
                (
                    metadata.dev(),
                    metadata.ino(),
                    metadata.ctime(),
                    metadata.ctime_nsec(),
                )
            },
        })
    }
}
struct CachedProbe {
    identity: Identity,
    checked_at: std::time::Instant,
    result: Result<Descriptor, String>,
}
static CACHE: std::sync::Mutex<Option<CachedProbe>> = std::sync::Mutex::new(None);

pub(crate) fn resolve() -> Result<PathBuf, crate::AppError> {
    discover(None).map(|tool| tool.path)
}

pub(crate) fn resolve_for(capability: &str) -> Result<PathBuf, crate::AppError> {
    discover(Some(capability)).map(|tool| tool.path)
}

pub(crate) fn discover(capability: Option<&str>) -> Result<Descriptor, crate::AppError> {
    let executable = std::env::current_exe()?;
    let parent = executable
        .parent()
        .ok_or("Desktop executable has no parent directory")?;
    let name = if cfg!(windows) {
        "future.exe"
    } else {
        "future"
    };
    let path = resolve_from(parent, name, std::env::var_os("PATH").as_deref())
        .ok_or("No usable future CLI found. Install the matching CLI on PATH or beside Desktop; CLI-dependent skills and maintenance are unavailable.")?;
    let identity = Identity::read(&path)?;
    let mut cache = CACHE.lock().unwrap_or_else(|error| error.into_inner());
    let source = if path == parent.join(name) {
        "bundled"
    } else if path.parent().is_some_and(|p| {
        std::env::var_os("PATH")
            .is_some_and(|value| std::env::split_paths(&value).any(|entry| entry == p))
    }) {
        "path"
    } else {
        "development"
    };
    if cache.as_ref().is_none_or(|cached| {
        cached.identity != identity
            || (cached.result.is_err()
                && cached.checked_at.elapsed() >= std::time::Duration::from_secs(5))
    }) {
        let result = probe(&path, source).and_then(|descriptor| {
            if Identity::read(&path).ok().as_ref() != Some(&identity) {
                return Err(
                    "CLI changed during discovery; retry after the build or installation completes"
                        .into(),
                );
            }
            Ok(descriptor)
        });
        *cache = Some(CachedProbe {
            identity,
            checked_at: std::time::Instant::now(),
            result,
        });
    }
    let descriptor = cache.as_ref().unwrap().result.clone().map_err(|error| format!("CLI {}: {error}. Install a matching CLI beside Desktop or correct PATH; no fallback CLI was executed.", path.display()))?;
    descriptor
        .capabilities
        .validate(capability)
        .map_err(|error| {
            format!(
                "CLI {} ({}): {error}. Install the matching CLI.",
                descriptor.path.display(),
                descriptor.version
            )
        })?;
    Ok(descriptor)
}

fn decode_probe(path: &Path, source: &str, bytes: &[u8]) -> Result<Descriptor, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Reply {
        version: String,
        desktop_capabilities: future_rpc::cli_capabilities::CliCapabilities,
    }
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("Unsupported version/capability response: {error}"))?;
    let reported = value
        .get("version")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let reply: Reply = serde_json::from_value(value)
        .map_err(|error| format!("version {reported}: unsupported capability response: {error}"))?;
    if reply.version.trim().is_empty() {
        return Err("CLI returned an empty display version".into());
    }
    reply
        .desktop_capabilities
        .validate(None)
        .map_err(|error| format!("version {}: {error}", reply.version))?;
    Ok(Descriptor {
        path: path.into(),
        source: source.into(),
        version: reply.version,
        capabilities: reply.desktop_capabilities,
    })
}

fn probe(path: &Path, source: &str) -> Result<Descriptor, String> {
    use std::{
        io::{Read, Seek},
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    const LIMIT: u64 = 64 * 1024;
    // File-backed capture avoids a blocked pipe or an unbounded reader thread
    // if a broken CLI inherits its output handles into another process.
    #[cfg(windows)]
    {
        let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut header = [0; 64];
        file.read_exact(&mut header)
            .map_err(|_| "CLI is not a Windows PE executable")?;
        let offset = u32::from_le_bytes(header[60..64].try_into().unwrap()) as u64;
        file.seek(std::io::SeekFrom::Start(offset))
            .map_err(|e| e.to_string())?;
        let mut signature = [0; 4];
        file.read_exact(&mut signature)
            .map_err(|_| "CLI has no PE header")?;
        if &header[..2] != b"MZ" || &signature != b"PE\0\0" {
            return Err("CLI is not a Windows PE executable".into());
        }
    }
    let mut stdout = tempfile::tempfile().map_err(|e| e.to_string())?;
    let stderr = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut command = Command::new(path);
    command
        .args(["version", "--json"])
        .stdin(Stdio::null())
        .stdout(stdout.try_clone().map_err(|e| e.to_string())?)
        .stderr(stderr.try_clone().map_err(|e| e.to_string())?);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("Cannot probe CLI: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(3);
    let status = loop {
        let oversized = stdout.metadata().map(|m| m.len() > LIMIT).unwrap_or(true)
            || stderr.metadata().map(|m| m.len() > LIMIT).unwrap_or(true);
        if oversized || Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(if oversized {
                "CLI discovery exceeded its output limit"
            } else {
                "CLI discovery timed out"
            }
            .into());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.to_string());
            }
        }
    };
    if stderr.metadata().map_err(|e| e.to_string())?.len() > LIMIT {
        return Err("CLI discovery exceeded its output limit".into());
    }
    if !status.success() {
        return Err(format!("CLI version discovery exited with {status}"));
    }
    stdout.rewind().map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    stdout
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > LIMIT {
        return Err("CLI discovery exceeded its output limit".into());
    }
    decode_probe(path, source, &bytes)
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

    #[test]
    fn structured_discovery_rejects_legacy_and_incompatible_tools() {
        let path = Path::new("/fixture/future");
        assert!(decode_probe(path, "bundled", br#"{"version":"old"}"#).is_err());
        assert!(decode_probe(path, "bundled", b"not JSON").is_err());
        let mut capabilities = future_rpc::cli_capabilities::CliCapabilities::default();
        let response =
            serde_json::json!({"version": "display-version", "desktopCapabilities": capabilities});
        let tool = decode_probe(path, "bundled", &serde_json::to_vec(&response).unwrap()).unwrap();
        assert_eq!(tool.path, path);
        assert_eq!(tool.source, "bundled");
        capabilities.storage_schema += 1;
        let response =
            serde_json::json!({"version": "old-version", "desktopCapabilities": capabilities});
        let error =
            decode_probe(path, "bundled", &serde_json::to_vec(&response).unwrap()).unwrap_err();
        assert!(error.contains("old-version"));
        assert!(error.contains("incompatible"));
    }

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
