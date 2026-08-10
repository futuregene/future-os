//! `future init` — 1:1 port of cli/src/commands/init.ts.
//!
//! Installs built-in skills (via skills::install_builtin_skills), then on
//! macOS/Linux links `future` and (when present) its sibling `future-agent`
//! into `~/.future/bin/`. Options mirror the TS `InitOptions` so the Rust
//! unit tests can inject a fake install hook, home dir, executable path, and
//! platform exactly like the TS tests do.

use crate::commands::skills::install_builtin_skills;
use crate::output::Output;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

/// `InitOptions` from init.ts — every field optional.
#[derive(Default)]
pub struct InitOptions {
    pub executable_path: Option<PathBuf>,
    pub home_dir: Option<PathBuf>,
    /// `installBuiltins` hook; defaults to the real skill installer.
    pub install_builtins: Option<InstallBuiltinsFn>,
    /// Node `os.platform()` string: "darwin" | "linux" | "win32".
    pub platform: Option<&'static str>,
}

/// `() => Promise<void>` — async hook used by tests.
pub type InstallBuiltinsFn =
    Box<dyn Fn(&Output) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// The command entry — `future init` with default options.
pub async fn init_command(out: &Output) -> Result<(), String> {
    init(InitOptions::default(), out).await
}

/// Node `os.platform()` string for the current target.
#[cfg(target_os = "macos")]
const DEFAULT_PLATFORM: &str = "darwin";
/// Windows value of [`DEFAULT_PLATFORM`].
#[cfg(windows)]
const DEFAULT_PLATFORM: &str = "win32";
/// Linux value of [`DEFAULT_PLATFORM`].
#[cfg(not(any(target_os = "macos", windows)))]
const DEFAULT_PLATFORM: &str = "linux";

/// `init(options = {})` — full port.
pub async fn init(options: InitOptions, out: &Output) -> Result<(), String> {
    // `const installBuiltins = options.installBuiltins ?? installBuiltinSkills;`
    // The default hook clones the (cheap, Arc-backed) Output so the future is
    // 'static, keeping the hook type simple.
    let install_builtins = options.install_builtins.unwrap_or_else(|| {
        Box::new(|out: &Output| {
            let out = out.clone();
            Box::pin(async move { install_builtin_skills(&out).await })
        })
    });
    install_builtins(out).await;

    // `const platform = options.platform ?? osPlatform();` — cfg-gated (not
    // cfg!) so the off-platform arms are never compiled into this target.
    let platform = options.platform.unwrap_or(DEFAULT_PLATFORM);
    if platform != "darwin" && platform != "linux" {
        return Ok(());
    }

    // `const homeDir = options.homeDir ?? homedir();`
    let home_dir = options
        .home_dir
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default());

    // `const executablePath = await realpath(options.executablePath ?? process.execPath);`
    let executable_path = realpath(
        &options
            .executable_path
            .unwrap_or_else(|| std::env::current_exe().unwrap_or_default()),
    )
    .await?;
    let executable_name = basename(&executable_path);
    if executable_name != "future" {
        return Err(format!(
            "Cannot initialize command links from {}. Run the standalone future executable.",
            executable_path.display()
        ));
    }

    // `const expectedAgentPath = join(executableDir, "future-agent");`
    let executable_dir = executable_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let expected_agent_path = executable_dir.join("future-agent");
    // `try { agentPath = await realpath(expectedAgentPath); } catch (error) {
    //   if (error.code !== "ENOENT") throw error; }`
    let agent_path = match realpath(&expected_agent_path).await {
        Ok(path) => Some(path),
        Err(e) if is_not_found(&e) => None,
        Err(e) => return Err(e),
    };

    // `const binDir = join(homeDir, ".future", "bin");`
    let bin_dir = home_dir.join(".future").join("bin");
    tokio::fs::create_dir_all(&bin_dir)
        .await
        .map_err(|e| e.to_string())?;

    ensure_symlink(&executable_path, bin_dir.join("future")).await?;
    if let Some(agent_path) = &agent_path {
        ensure_symlink(agent_path, bin_dir.join("future-agent")).await?;
    }

    out.log(&format!(
        "Linked future{} into {}.",
        if agent_path.is_some() {
            " and future-agent"
        } else {
            ""
        },
        bin_dir.display()
    ));
    out.log("You can add ~/.future/bin/ to your PATH:");
    out.log("  export PATH=\"$HOME/.future/bin:$PATH\"");
    Ok(())
}

/// `ensureSymlink(source, destination)` — idempotent symlink creation.
async fn ensure_symlink(source: &Path, destination: PathBuf) -> Result<(), String> {
    // `try { const destinationStat = await lstat(destination); ... }`
    match tokio::fs::symlink_metadata(&destination).await {
        Ok(meta) => {
            if !meta.file_type().is_symlink() {
                return Err(format!(
                    "Cannot create command link: {} already exists and is not a symbolic link.",
                    destination.display()
                ));
            }
            // `const currentTarget = await readlink(destination);`
            let current_target = tokio::fs::read_link(&destination)
                .await
                .map_err(|e| e.to_string())?;
            // `resolve(dirname(destination), currentTarget) === source`
            let resolved = if current_target.is_absolute() {
                current_target
            } else {
                destination
                    .parent()
                    .map(|p| p.join(&current_target))
                    .unwrap_or(current_target)
            };
            if resolved == source {
                return Ok(());
            }
            tokio::fs::remove_file(&destination)
                .await
                .map_err(|e| e.to_string())?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }

    create_symlink(source, &destination)
        .await
        .map_err(|e| e.to_string())
}

/// `fs.symlink` (Node) — file symlink creation. `tokio::fs::symlink` is
/// unix-only; init never reaches this on Windows (guarded by the platform
/// check in `init_command`), but the code must still compile there.
#[cfg(unix)]
async fn create_symlink(source: &Path, destination: &Path) -> std::io::Result<()> {
    tokio::fs::symlink(source, destination).await
}

#[cfg(windows)]
async fn create_symlink(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(source, destination)
}

/// `fs.realpath` equivalent (canonicalize); ENOENT is reported as `Err` with
/// a distinguishable marker via [`is_not_found`].
async fn realpath(path: &Path) -> Result<PathBuf, String> {
    tokio::fs::canonicalize(path)
        .await
        .map_err(|e| format!("{e}"))
}

fn is_not_found(err: &str) -> bool {
    // tokio's canonicalize error Display includes the OS message, e.g.
    // "No such file or directory (os error 2)".
    err.contains("os error 2") || err.to_lowercase().contains("no such file")
}

/// `basename(path)` — last path component.
fn basename(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::Output;
    use std::sync::Arc;

    /// `createUnixFixture` from init.test.ts — app/future + app/future-agent.
    async fn create_unix_fixture(root: &Path) -> (PathBuf, PathBuf) {
        let executable_dir = root.join("app");
        let home_dir = root.join("home");
        tokio::fs::create_dir_all(&executable_dir).await.unwrap();
        tokio::fs::create_dir_all(&home_dir).await.unwrap();
        let executable_path = executable_dir.join("future");
        tokio::fs::write(&executable_path, "").await.unwrap();
        tokio::fs::write(executable_dir.join("future-agent"), "")
            .await
            .unwrap();
        (executable_path, home_dir)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn realpath_non_enoent_error_propagates() {
        let _guard = crate::test_env::lock_env().await;
        let root = tempfile::tempdir().unwrap();
        let (executable_path, home_dir) = create_unix_fixture(root.path()).await;
        // Replace future-agent with a symlink LOOP → realpath fails ELOOP
        // (not ENOENT) → the error propagates instead of defaulting.
        let agent = executable_path.parent().unwrap().join("future-agent");
        tokio::fs::remove_file(&agent).await.unwrap();
        let loop_a = root.path().join("loop-a");
        let loop_b = root.path().join("loop-b");
        tokio::fs::symlink(&loop_b, &loop_a).await.unwrap();
        tokio::fs::symlink(&loop_a, &loop_b).await.unwrap();
        tokio::fs::symlink(&loop_a, &agent).await.unwrap();
        let install_count = Arc::new(std::sync::atomic::AtomicI32::new(0));
        let (code, _, _) = run_init(&executable_path, &home_dir, install_count, "darwin").await;
        assert_eq!(code, 1);
    }

    async fn run_init(
        executable_path: &Path,
        home_dir: &Path,
        install_count: Arc<std::sync::atomic::AtomicI32>,
        platform: &'static str,
    ) -> (i32, String, String) {
        let (out, cap) = Output::memory();
        let hook = {
            let install_count = install_count.clone();
            move |_out: &Output| {
                let install_count = install_count.clone();
                Box::pin(async move {
                    install_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }) as Pin<Box<dyn Future<Output = ()> + Send>>
            }
        };
        let options = InitOptions {
            executable_path: Some(executable_path.to_path_buf()),
            home_dir: Some(home_dir.to_path_buf()),
            install_builtins: Some(Box::new(hook)),
            platform: Some(platform),
        };
        let result = init(options, &out).await;
        let code = match &result {
            Ok(()) => 0,
            Err(msg) => {
                // The real dispatcher's catch() does console.error(message);
                // replicate it so tests can assert on stderr.
                out.log_err(msg);
                1
            }
        };
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        (code, stdout, stderr)
    }

    #[tokio::test]
    async fn installs_builtins_and_creates_idempotent_macos_links() {
        let _guard = crate::test_env::lock_env().await;
        let root = tempfile::tempdir().unwrap();
        let (executable_path, home_dir) = create_unix_fixture(root.path()).await;
        let install_count = Arc::new(std::sync::atomic::AtomicI32::new(0));

        let (code, stdout, _) =
            run_init(&executable_path, &home_dir, install_count.clone(), "darwin").await;
        assert_eq!(code, 0);
        assert!(stdout.contains("Linked future and future-agent into"));
        assert_eq!(install_count.load(std::sync::atomic::Ordering::Relaxed), 1);

        let bin_dir = home_dir.join(".future").join("bin");
        let canonical_exe = tokio::fs::canonicalize(&executable_path).await.unwrap();
        let canonical_agent = tokio::fs::canonicalize(root.path().join("app").join("future-agent"))
            .await
            .unwrap();
        assert_eq!(
            tokio::fs::read_link(bin_dir.join("future")).await.unwrap(),
            canonical_exe
        );
        assert_eq!(
            tokio::fs::read_link(bin_dir.join("future-agent"))
                .await
                .unwrap(),
            canonical_agent
        );

        // Second run via the linked path — idempotent.
        let (code, _, _) = run_init(
            &bin_dir.join("future"),
            &home_dir,
            install_count.clone(),
            "darwin",
        )
        .await;
        assert_eq!(code, 0);
        assert_eq!(install_count.load(std::sync::atomic::Ordering::Relaxed), 2);
        assert_eq!(
            tokio::fs::read_link(bin_dir.join("future")).await.unwrap(),
            canonical_exe
        );
        assert_eq!(
            tokio::fs::read_link(bin_dir.join("future-agent"))
                .await
                .unwrap(),
            canonical_agent
        );
    }

    #[tokio::test]
    async fn installs_builtins_and_creates_links_on_linux() {
        let _guard = crate::test_env::lock_env().await;
        let root = tempfile::tempdir().unwrap();
        let (executable_path, home_dir) = create_unix_fixture(root.path()).await;
        let install_count = Arc::new(std::sync::atomic::AtomicI32::new(0));
        let (code, _, _) =
            run_init(&executable_path, &home_dir, install_count.clone(), "linux").await;
        assert_eq!(code, 0);
        assert_eq!(install_count.load(std::sync::atomic::Ordering::Relaxed), 1);
        let bin_dir = home_dir.join(".future").join("bin");
        assert_eq!(
            tokio::fs::read_link(bin_dir.join("future")).await.unwrap(),
            tokio::fs::canonicalize(&executable_path).await.unwrap()
        );
        assert_eq!(
            tokio::fs::read_link(bin_dir.join("future-agent"))
                .await
                .unwrap(),
            tokio::fs::canonicalize(root.path().join("app").join("future-agent"))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn links_future_when_sibling_agent_is_missing() {
        let _guard = crate::test_env::lock_env().await;
        let root = tempfile::tempdir().unwrap();
        let (executable_path, home_dir) = create_unix_fixture(root.path()).await;
        tokio::fs::remove_file(root.path().join("app").join("future-agent"))
            .await
            .unwrap();
        let install_count = Arc::new(std::sync::atomic::AtomicI32::new(0));
        let (code, stdout, _) =
            run_init(&executable_path, &home_dir, install_count, "darwin").await;
        assert_eq!(code, 0);
        assert!(stdout.contains("Linked future into"));
        let bin_dir = home_dir.join(".future").join("bin");
        assert_eq!(
            tokio::fs::read_link(bin_dir.join("future")).await.unwrap(),
            tokio::fs::canonicalize(&executable_path).await.unwrap()
        );
        assert!(tokio::fs::symlink_metadata(bin_dir.join("future-agent"))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn installs_builtins_without_links_on_windows() {
        let _guard = crate::test_env::lock_env().await;
        let root = tempfile::tempdir().unwrap();
        let (executable_path, home_dir) = create_unix_fixture(root.path()).await;
        let install_count = Arc::new(std::sync::atomic::AtomicI32::new(0));
        let (code, _, _) =
            run_init(&executable_path, &home_dir, install_count.clone(), "win32").await;
        assert_eq!(code, 0);
        assert_eq!(install_count.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert!(
            tokio::fs::symlink_metadata(home_dir.join(".future").join("bin").join("future"))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn does_not_overwrite_an_existing_regular_command_file() {
        let _guard = crate::test_env::lock_env().await;
        let root = tempfile::tempdir().unwrap();
        let (executable_path, home_dir) = create_unix_fixture(root.path()).await;
        let bin_dir = home_dir.join(".future").join("bin");
        let existing_command = bin_dir.join("future");
        tokio::fs::create_dir_all(&bin_dir).await.unwrap();
        tokio::fs::write(&existing_command, "keep me")
            .await
            .unwrap();
        let install_count = Arc::new(std::sync::atomic::AtomicI32::new(0));
        let (code, _, stderr) =
            run_init(&executable_path, &home_dir, install_count, "darwin").await;
        assert_eq!(code, 1);
        assert!(stderr.contains("already exists and is not a symbolic link"));
        assert_eq!(
            tokio::fs::read_to_string(&existing_command).await.unwrap(),
            "keep me"
        );
    }

    #[tokio::test]
    async fn rejects_an_interpreter_path_instead_of_linking_it() {
        let _guard = crate::test_env::lock_env().await;
        let root = tempfile::tempdir().unwrap();
        let (_, home_dir) = create_unix_fixture(root.path()).await;
        let interpreter_path = root.path().join("app").join("bun");
        tokio::fs::write(&interpreter_path, "").await.unwrap();
        let install_count = Arc::new(std::sync::atomic::AtomicI32::new(0));
        let (code, _, stderr) =
            run_init(&interpreter_path, &home_dir, install_count, "darwin").await;
        assert_eq!(code, 1);
        assert!(stderr.contains("Run the standalone future executable"));
    }

    #[tokio::test]
    async fn init_command_with_defaults_errors_on_test_binary() {
        let _guard = crate::test_env::lock_env().await;
        // Isolated HOME with the platform pointed at a dead port so the
        // default install hook (install_builtin_skills) fails fast offline.
        let _home = crate::test_env::EnvGuard::temp_home();
        let auth = crate::constants::auth_file();
        tokio::fs::create_dir_all(auth.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &auth,
            "{\"future\": {\"base_url\": \"http://127.0.0.1:1\"}}",
        )
        .await
        .unwrap();
        let (out, _cap) = Output::memory();
        // The test binary is not named "future" → the command refuses.
        let err = init_command(&out).await.unwrap_err();
        assert!(
            err.contains("Cannot initialize command links from"),
            "err: {err}"
        );
        // The default hook ran and failed (catalog unreachable) → exit code 1.
        assert_eq!(out.exit_code(), 1);
    }

    #[tokio::test]
    async fn existing_symlink_to_other_target_is_repointed() {
        let _guard = crate::test_env::lock_env().await;
        let root = tempfile::tempdir().unwrap();
        let (executable_path, home_dir) = create_unix_fixture(root.path()).await;
        let bin_dir = home_dir.join(".future").join("bin");
        tokio::fs::create_dir_all(&bin_dir).await.unwrap();
        // Stale symlink: future → some OTHER binary.
        let other = root.path().join("old-future");
        tokio::fs::write(&other, "").await.unwrap();
        #[cfg(unix)]
        tokio::fs::symlink(&other, bin_dir.join("future"))
            .await
            .unwrap();
        let install_count = Arc::new(std::sync::atomic::AtomicI32::new(0));
        let (code, _, _) = run_init(&executable_path, &home_dir, install_count, "darwin").await;
        assert_eq!(code, 0);
        assert_eq!(
            tokio::fs::read_link(bin_dir.join("future")).await.unwrap(),
            tokio::fs::canonicalize(&executable_path).await.unwrap()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn existing_relative_symlink_is_resolved_and_recreated() {
        let _guard = crate::test_env::lock_env().await;
        let root = tempfile::tempdir().unwrap();
        let (executable_path, home_dir) = create_unix_fixture(root.path()).await;
        let bin_dir = home_dir.join(".future").join("bin");
        tokio::fs::create_dir_all(&bin_dir).await.unwrap();
        // A RELATIVE symlink target exercises the readlink-relative resolution
        // path; it does not match the new source, so it is recreated absolute.
        tokio::fs::symlink("old-future", bin_dir.join("future"))
            .await
            .unwrap();
        let install_count = Arc::new(std::sync::atomic::AtomicI32::new(0));
        let (code, _, stderr) =
            run_init(&executable_path, &home_dir, install_count, "darwin").await;
        assert_eq!(code, 0, "stderr: {stderr}");
        assert_eq!(
            tokio::fs::read_link(bin_dir.join("future")).await.unwrap(),
            tokio::fs::canonicalize(&executable_path).await.unwrap()
        );
    }

    #[tokio::test]
    async fn ensure_symlink_metadata_error_beyond_not_found() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("future");
        tokio::fs::write(&source, "x").await.unwrap();
        // destination's PARENT is a regular file → symlink_metadata fails
        // with ENOTDIR (not NotFound) → propagated.
        let blocker = dir.path().join("blocker");
        tokio::fs::write(&blocker, "x").await.unwrap();
        let destination = blocker.join("future");
        let err = ensure_symlink(&source, destination).await.unwrap_err();
        assert!(!err.is_empty());
    }
}
