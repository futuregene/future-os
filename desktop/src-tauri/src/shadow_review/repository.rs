//! Shadow git repository: paths, isolated git invocation, init + config, real-repo
//! object/index seeding, persisted index, and ref management (§5.1, §5.2).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use crate::AppError;

fn review_root() -> Result<PathBuf, AppError> {
    let app_dir = crate::store::app_data_path()?.app_dir;
    Ok(PathBuf::from(app_dir).join("review"))
}

// ── per-Workspace serialization (§12.1) ─────────────────────────────────────

static WORKSPACE_LOCKS: OnceLock<Mutex<HashMap<String, Weak<Mutex<()>>>>> = OnceLock::new();

fn workspace_lock(workspace_id: &str) -> Arc<Mutex<()>> {
    let map = WORKSPACE_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().unwrap_or_else(|poison| poison.into_inner());
    guard.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = guard.get(workspace_id).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    guard.insert(workspace_id.to_string(), Arc::downgrade(&lock));
    lock
}

/// Run `f` while holding the Workspace's shadow lock. Snapshot writes for the
/// same Workspace are serialized; different Workspaces run concurrently. The
/// lock guards storage integrity only, not semantic attribution (§12.5).
pub fn with_workspace_lock<T>(
    workspace_id: &str,
    f: impl FnOnce() -> Result<T, AppError>,
) -> Result<T, AppError> {
    let lock = workspace_lock(workspace_id);
    let _guard = lock.lock().unwrap_or_else(|poison| poison.into_inner());
    f()
}

// ── shadow repository handle ────────────────────────────────────────────────

/// A handle to one Workspace's shadow repository. Cheap to construct;
/// `open` lazily initializes the repo (init + config + seed) on first use.
pub struct ShadowRepo {
    pub workspace_id: String,
    pub workspace_path: PathBuf,
    pub is_git_workspace: bool,
    git_dir: PathBuf,
    index_path: PathBuf,
}

impl ShadowRepo {
    pub fn open(
        workspace_id: &str,
        workspace_path: &Path,
        is_git_workspace: bool,
    ) -> Result<Self, AppError> {
        let root = review_root()?.join(workspace_id);
        let indexes = root.join("indexes");
        let locks = root.join("locks");
        fs::create_dir_all(&indexes)?;
        fs::create_dir_all(&locks)?;
        restrict_permissions(&root);

        let repo = Self {
            workspace_id: workspace_id.to_string(),
            workspace_path: workspace_path.to_path_buf(),
            is_git_workspace,
            git_dir: root.join("repo.git"),
            index_path: indexes.join("index"),
        };
        // Initialize under the workspace lock so two concurrent opens (e.g. a
        // live Run and startup recovery) can't both run `git init` on the same
        // shadow repo. Callers always `open` outside the lock, so this never
        // nests. The lock is released before the caller takes it for snapshots.
        with_workspace_lock(workspace_id, || repo.ensure_initialized())?;
        Ok(repo)
    }

    /// A handle for GIT_DIR-only operations (ref deletion, gc, commit existence)
    /// that don't need the work tree or initialization. Used by maintenance.
    pub fn open_bare(workspace_id: &str) -> Result<Self, AppError> {
        let root = review_root()?.join(workspace_id);
        Ok(Self {
            workspace_id: workspace_id.to_string(),
            workspace_path: PathBuf::new(),
            is_git_workspace: false,
            git_dir: root.join("repo.git"),
            index_path: root.join("indexes").join("index"),
        })
    }

    // ── git invocation (isolated via env) ───────────────────────────────────

    fn base_command(&self, index: Option<&Path>) -> Command {
        use crate::proc::NoWindow;
        let mut cmd = Command::new("git");
        cmd.no_window();
        cmd.env("GIT_DIR", &self.git_dir);
        // Bare handles (open_bare) have no work tree; only set it when present.
        if !self.workspace_path.as_os_str().is_empty() {
            cmd.env("GIT_WORK_TREE", &self.workspace_path);
        }
        if let Some(index) = index {
            cmd.env("GIT_INDEX_FILE", index);
        }
        cmd
    }

    /// Run a shadow git command without checking exit status. `stdin` is piped
    /// when present (used for `--pathspec-from-file=-`).
    pub fn run(
        &self,
        args: &[&str],
        index: Option<&Path>,
        stdin: Option<&[u8]>,
    ) -> Result<Output, AppError> {
        use std::io::Write;

        let mut cmd = self.base_command(index);
        cmd.args(args);
        if stdin.is_some() {
            cmd.stdin(Stdio::piped());
        }
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|error| format!("shadow git {args:?} failed to spawn: {error}"))?;
        if let Some(bytes) = stdin {
            child
                .stdin
                .take()
                .ok_or_else(|| "shadow git stdin unavailable".to_string())?
                .write_all(bytes)
                .map_err(|error| format!("shadow git stdin write failed: {error}"))?;
        }
        child
            .wait_with_output()
            .map_err(|error| format!("shadow git {args:?} failed: {error}").into())
    }

    /// Run a shadow git command, returning trimmed stdout on success.
    pub fn git(&self, args: &[&str], index: Option<&Path>) -> Result<String, AppError> {
        let output = self.run(args, index, None)?;
        check_status(args, &output)?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Run a shadow git command, returning raw stdout bytes on success
    /// (binary-safe — used for diff output).
    pub fn git_bytes(&self, args: &[&str], index: Option<&Path>) -> Result<Vec<u8>, AppError> {
        let output = self.run(args, index, None)?;
        check_status(args, &output)?;
        Ok(output.stdout)
    }

    // ── tree / commit / ref ─────────────────────────────────────────────────

    /// `commit-tree` a snapshot tree with the FutureOS Snapshot identity (§5.3).
    pub fn commit_tree(&self, tree_id: &str, message: &str) -> Result<String, AppError> {
        let mut cmd = self.base_command(None);
        cmd.env("GIT_AUTHOR_NAME", "FutureOS Snapshot")
            .env("GIT_AUTHOR_EMAIL", "snapshot@futureos.local")
            .env("GIT_COMMITTER_NAME", "FutureOS Snapshot")
            .env("GIT_COMMITTER_EMAIL", "snapshot@futureos.local")
            .args(["commit-tree", tree_id, "-m", message])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output = cmd
            .output()
            .map_err(|error| format!("shadow commit-tree failed: {error}"))?;
        check_status(&["commit-tree"], &output)?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    pub fn update_ref(&self, name: &str, commit: &str) -> Result<(), AppError> {
        self.git(&["update-ref", name, commit], None)?;
        Ok(())
    }

    pub fn snapshot_ref(thread_id: &str, run_id: &str, phase: &str) -> String {
        format!("refs/futureos/threads/{thread_id}/runs/{run_id}/{phase}")
    }

    /// Whether a commit object exists in the shadow repo (or via alternates).
    pub fn commit_exists(&self, commit: &str) -> bool {
        self.run(
            &["cat-file", "-e", &format!("{commit}^{{commit}}")],
            None,
            None,
        )
        .map(|output| output.status.success())
        .unwrap_or(false)
    }

    /// Delete a ref (no-op if it doesn't exist).
    pub fn delete_ref(&self, name: &str) -> Result<(), AppError> {
        // `update-ref -d` errors if the ref is already gone — tolerate it.
        let _ = self.run(&["update-ref", "-d", name], None, None)?;
        Ok(())
    }

    /// Let git decide whether a gc is actually warranted (loose-object
    /// threshold); cheap to call after every retention pass (§12.3). Best-effort.
    pub fn gc_auto(&self) {
        let _ = self.run(&["gc", "--auto", "--quiet"], None, None);
    }

    // ── persisted index handling (§5.4) ─────────────────────────────────────

    /// Copy the persisted shadow index to an isolated temp index and return its
    /// path. When no persisted index exists yet (first snapshot), the temp path
    /// is returned uncreated — git creates it on first write.
    pub fn prepare_temp_index(&self, tag: &str) -> Result<PathBuf, AppError> {
        let temp = self.git_dir.join(format!("index.tmp.{tag}"));
        let _ = fs::remove_file(&temp);
        if self.index_path.exists() {
            fs::copy(&self.index_path, &temp)?;
        }
        Ok(temp)
    }

    /// Atomically promote a temp index to the persisted index (same filesystem).
    pub fn commit_temp_index(&self, temp: &Path) -> Result<(), AppError> {
        if temp.exists() {
            fs::rename(temp, &self.index_path)?;
        }
        Ok(())
    }

    /// Merge extra lines into the shadow repo's `info/exclude` (§5.5 boundary 1
    /// and oversized-untracked handling). `base` is the real repo's
    /// `info/exclude` content (empty for non-git Workspaces).
    pub fn write_info_exclude(&self, lines: &[String]) -> Result<(), AppError> {
        let info = self.git_dir.join("info");
        fs::create_dir_all(&info)?;
        let body = lines.join("\n");
        let body = if body.is_empty() {
            String::new()
        } else {
            format!("{body}\n")
        };
        fs::write(info.join("exclude"), body)?;
        Ok(())
    }

    /// Real repo's `info/exclude` content, if the Workspace is a git repo.
    pub fn real_repo_info_exclude(&self) -> Option<String> {
        if !self.is_git_workspace {
            return None;
        }
        let common = self.real_git(&["rev-parse", "--git-common-dir"]).ok()?;
        let path = self
            .workspace_path
            .join(common.trim())
            .join("info")
            .join("exclude");
        fs::read_to_string(path).ok()
    }

    // ── initialization ──────────────────────────────────────────────────────

    fn ensure_initialized(&self) -> Result<(), AppError> {
        if self.git_dir.join("HEAD").exists() {
            return Ok(());
        }
        fs::create_dir_all(&self.git_dir)?;
        self.git_init()?;
        self.configure()?;
        if self.is_git_workspace {
            self.seed_from_real_repo()?;
        }
        Ok(())
    }

    fn git_init(&self) -> Result<(), AppError> {
        let output = self
            .base_command(None)
            .args(["init", "--quiet"])
            .output()
            .map_err(|error| format!("shadow git init failed to spawn: {error}"))?;
        check_status(&["init"], &output)
    }

    /// Real empty file for `core.excludesFile`, so the shadow repo never
    /// inherits the user's global git excludes (§5.5). A file rather than the
    /// null device: git-for-Windows rejects `NUL` outright ("fatal: cannot use
    /// NUL as an exclude file"), while an empty file behaves the same
    /// everywhere. Lives inside the shadow git dir, which `ensure_initialized`
    /// creates before this runs.
    fn ensure_empty_excludes_file(&self) -> Result<PathBuf, AppError> {
        let path = self.git_dir.join("empty-excludes");
        if !path.exists() {
            fs::write(&path, b"")?;
        }
        Ok(path)
    }

    fn configure(&self) -> Result<(), AppError> {
        // §5.2: autocrlf/symlinks are correctness; the rest are large-repo perf.
        // Forward slashes in the excludes path: unquoted backslashes are
        // escape characters in git config syntax, and git accepts `/` paths
        // on every platform (incl. git-for-Windows).
        let excludes_file = self
            .ensure_empty_excludes_file()?
            .to_string_lossy()
            .replace('\\', "/");
        let pairs: [(&str, &str); 8] = [
            ("core.autocrlf", "false"),
            ("core.symlinks", "true"),
            ("core.fsmonitor", "false"),
            ("core.untrackedCache", "true"),
            ("core.excludesFile", excludes_file.as_str()),
            ("feature.manyFiles", "true"),
            ("index.version", "4"),
            ("index.threads", "true"),
        ];
        for (key, value) in pairs {
            self.git(&["config", key, value], None)?;
        }
        Ok(())
    }

    /// Share the real repo's object DB (alternates) and seed the shadow index
    /// from its index, so a huge repo's first snapshot avoids a full re-hash
    /// (§5.2). Best-effort: any failure just falls back to a full snapshot.
    fn seed_from_real_repo(&self) -> Result<(), AppError> {
        let common =
            match self.real_git(&["rev-parse", "--path-format=absolute", "--git-common-dir"]) {
                Ok(value) => value.trim().to_string(),
                Err(_) => return Ok(()),
            };
        let source = PathBuf::from(&common);
        if !source.exists() {
            return Ok(());
        }

        let source_objects = source.join("objects");
        let mut alternates: Vec<String> = Vec::new();
        if source_objects.exists() {
            alternates.push(source_objects.display().to_string());
        }
        if let Ok(text) = fs::read_to_string(source_objects.join("info").join("alternates")) {
            for line in text.lines() {
                let line = line.trim();
                if !line.is_empty() && Path::new(line).exists() {
                    alternates.push(line.to_string());
                }
            }
        }
        if alternates.is_empty() {
            return Ok(());
        }

        let info = self.git_dir.join("objects").join("info");
        fs::create_dir_all(&info)?;
        fs::write(
            info.join("alternates"),
            format!("{}\n", alternates.join("\n")),
        )?;

        // Seed the index so already-hashed entries are reused (pairs with
        // alternates — see §5.2). Best-effort.
        let source_index = source.join("index");
        if source_index.exists() {
            let _ = fs::copy(&source_index, &self.index_path);
        }
        Ok(())
    }

    /// Run git against the user's *real* repo (no GIT_DIR override), used only
    /// for read-only seeding lookups.
    fn real_git(&self, args: &[&str]) -> Result<String, AppError> {
        use crate::proc::NoWindow;
        let output = Command::new("git")
            .no_window()
            .arg("-C")
            .arg(&self.workspace_path)
            .args(args)
            .output()
            .map_err(|error| format!("real git {args:?} failed to spawn: {error}"))?;
        check_status(args, &output)?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

fn check_status(args: &[&str], output: &Output) -> Result<(), AppError> {
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "shadow git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into())
    }
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_store::test_support::HomeGuard;

    fn git_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "futureos-shadow-repo-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["init", "-q"])
            .output()
            .unwrap();
        assert!(out.status.success());
        std::fs::write(dir.join("a.txt"), "hello\n").unwrap();
        let file = std::fs::File::open(dir.join("a.txt")).unwrap();
        file.set_modified(
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000),
        )
        .unwrap();
        for args in [
            &["config", "user.email", "t@e.com"][..],
            &["config", "user.name", "T"][..],
            &["add", "a.txt"][..],
            &["commit", "-qm", "init"][..],
        ] {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}");
        }
        dir
    }

    #[test]
    fn workspace_lock_is_reused_and_serialized() {
        let a = workspace_lock("ws-lock");
        let b = workspace_lock("ws-lock");
        assert!(Arc::ptr_eq(&a, &b), "same id reuses the same lock");
        let other = workspace_lock("ws-lock-2");
        assert!(!Arc::ptr_eq(&a, &other));
        with_workspace_lock("ws-lock", || Ok::<_, AppError>(42)).unwrap();
    }

    #[test]
    fn open_and_bare_initialize_git_and_non_git() {
        let _home = HomeGuard::new("shadow-repo-open");
        crate::store::initialize_app_store().unwrap();

        // Non-git workspace: init + configure, no seeding.
        let plain = std::env::temp_dir().join(format!(
            "futureos-shadow-plain-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&plain).unwrap();
        let repo = ShadowRepo::open("ws-plain", &plain, false).unwrap();
        assert_eq!(repo.workspace_id, "ws-plain");
        assert!(!repo.real_repo_info_exclude().is_some());

        // Git workspace: seeds from the real repo.
        let src = git_dir();
        let repo = ShadowRepo::open("ws-git", &src, true).unwrap();
        assert!(repo.real_repo_info_exclude().is_some());

        // Re-open short-circuits on the existing HEAD.
        let again = ShadowRepo::open("ws-git", &src, true).unwrap();
        let _ = again;

        // Bare handle for GIT_DIR-only ops.
        let bare = ShadowRepo::open_bare("ws-git").unwrap();
        assert!(bare.workspace_path.as_os_str().is_empty());
    }

    #[test]
    fn git_ops_commit_ref_and_index() {
        let _home = HomeGuard::new("shadow-repo-ops");
        crate::store::initialize_app_store().unwrap();
        let src = git_dir();
        let repo = ShadowRepo::open("ws-ops", &src, true).unwrap();

        // write-info-exclude with empty and non-empty line sets.
        repo.write_info_exclude(&[]).unwrap();
        repo.write_info_exclude(&["foo.log".to_string()]).unwrap();

        // Snapshot ref name helper.
        let ref_name = ShadowRepo::snapshot_ref("t1", "r1", "before");
        assert_eq!(ref_name, "refs/futureos/threads/t1/runs/r1/before");

        // Prepare temp index when none exists yet, then promote it.
        let temp = repo.prepare_temp_index("r1.before").unwrap();
        let tree = repo.git(&["write-tree"], Some(&temp)).unwrap();
        let commit = repo.commit_tree(&tree, "snapshot").unwrap();
        assert_eq!(commit.len(), 40);
        repo.update_ref(&ref_name, &commit).unwrap();
        assert!(repo.commit_exists(&commit));
        assert!(!repo.commit_exists("0000000000000000000000000000000000000000"));
        repo.commit_temp_index(&temp).unwrap();
        // A second prepare now copies the persisted index.
        let temp2 = repo.prepare_temp_index("r1.after").unwrap();
        assert!(temp2.exists());
        repo.commit_temp_index(&temp2).unwrap();
        // delete-ref tolerates a missing ref and removes an existing one.
        repo.delete_ref("refs/futureos/nope").unwrap();
        repo.delete_ref(&ref_name).unwrap();
        // The commit object itself survives ref deletion (loose object) — the
        // ref is what's gone.
        repo.gc_auto();
        // git_bytes round-trips raw stdout (binary-safe) on a valid query.
        let _ = repo.git_bytes(&["rev-parse", "--git-dir"], None).unwrap();
        // run() with stdin piped (pathspec form) round-trips.
        let out = repo.run(&["rev-parse", "--git-dir"], None, Some(b"")).unwrap();
        assert!(out.status.success());
    }

    #[test]
    fn real_git_error_and_check_status() {
        let _home = HomeGuard::new("shadow-repo-err");
        crate::store::initialize_app_store().unwrap();
        let plain = std::env::temp_dir().join(format!(
            "futureos-shadow-plain-err-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&plain).unwrap();
        let repo = ShadowRepo::open("ws-err", &plain, true).unwrap();
        // real_git against a non-repo fails cleanly.
        assert!(repo.real_git(&["rev-parse", "--git-common-dir"]).is_err());
        // check_status passes a successful output and surfaces a failing one.
        let ok = std::process::Output {
            status: std::process::ExitStatus::default(),
            stdout: Vec::new(),
            stderr: Vec::new(),
        };
        assert!(check_status(&["ok"], &ok).is_ok());
        let fail = std::process::Command::new("sh")
            .arg("-c")
            .arg("exit 3")
            .output()
            .unwrap();
        let err = check_status(&["rev-parse"], &fail).unwrap_err();
        assert!(err.to_string().contains("shadow git"));
    }
}
