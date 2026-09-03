#![cfg(target_os = "linux")]

use super::probe::BwrapIdentity;
use super::request::{HelperPhase, LinuxSandboxRequest, MountKind};
use anyhow::{anyhow, Context, Result};
use std::ffi::CString;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

const HELPER_INFRA_EXIT: i32 = 125;

/// Hidden helper entry point. It intentionally terminates the helper process
/// with the wrapped process status, so it must be dispatched before the Agent
/// singleton and runtime are initialized.
pub fn run_encoded(encoded: &str) -> ! {
    let outcome = LinuxSandboxRequest::decode(encoded)
        .map_err(anyhow::Error::from)
        .and_then(run_request);
    match outcome {
        Ok(status) => mirror_status(status),
        Err(error) => {
            eprintln!("future-linux-sandbox-helper: {error:#}");
            std::process::exit(HELPER_INFRA_EXIT);
        }
    }
}

fn run_request(request: LinuxSandboxRequest) -> Result<ExitStatus> {
    match request.phase {
        HelperPhase::Outer => run_outer(request),
        HelperPhase::Inner => {
            let status_fd = request.status_fd.context("missing status fd")?;
            let status = run_inner(request)?;
            write_raw_status(status_fd, status)?;
            Ok(status)
        }
    }
}

fn run_outer(mut request: LinuxSandboxRequest) -> Result<ExitStatus> {
    let mut placeholders = Vec::new();
    for mount in &request.mounts {
        if mount.kind == MountKind::MissingProtected {
            std::fs::create_dir(&mount.source).with_context(|| {
                format!(
                    "create missing-path sandbox placeholder {}",
                    mount.source.display()
                )
            })?;
            let identity = identity_from_metadata(&std::fs::symlink_metadata(&mount.source)?);
            placeholders.push(MissingPlaceholder {
                path: mount.source.clone(),
                identity,
            });
        }
    }
    let bwrap = open_mount_source(&request.bwrap_path).context("open verified bwrap")?;
    if identity_from_metadata(&bwrap.metadata()?) != request.bwrap_identity {
        return Err(anyhow!("verified bwrap identity changed"));
    }
    clear_cloexec(bwrap.as_raw_fd())?;
    let bwrap_fd = bwrap.as_raw_fd();
    let (mut status_read, status_write) = create_pipe().context("create status pipe")?;
    let status_fd = status_write.as_raw_fd();
    clear_cloexec(status_fd)?;
    let mut inherited = Vec::with_capacity(request.mounts.len() + 3);
    inherited.push(bwrap);
    inherited.push(status_write);
    let mut opaque_directories = std::collections::BTreeSet::new();
    let mut needs_empty_file = false;
    for mount in &mut request.mounts {
        let file = open_mount_source(&mount.source)
            .with_context(|| format!("mount source is unavailable: {}", mount.source.display()))?;
        let fd = file.as_raw_fd();
        clear_cloexec(fd)?;
        let metadata = file.metadata()?;
        mount.expected = Some(identity_from_metadata(&metadata));
        mount.source_fd = Some(fd);
        if matches!(
            mount.kind,
            MountKind::Unreadable | MountKind::MissingProtected
        ) {
            if metadata.is_dir() {
                opaque_directories.insert(mount.target.clone());
            } else {
                needs_empty_file = true;
            }
        }
        inherited.push(file);
    }
    let empty_file = needs_empty_file
        .then(create_opaque_file)
        .transpose()
        .context("create unreadable-file source")?;
    let empty_fd = if let Some(file) = &empty_file {
        clear_cloexec(file.file.as_raw_fd())?;
        Some(file.file.as_raw_fd())
    } else {
        None
    };
    request.phase = HelperPhase::Inner;
    request.status_fd = Some(status_fd);
    let encoded = request.encode()?;
    let current_exe = std::env::current_exe().context("resolve current helper executable")?;

    // Execute the already-verified inode through its inherited O_PATH fd;
    // replacing the pathname after validation cannot change this invocation.
    let mut command = Command::new(format!("/proc/self/fd/{bwrap_fd}"));
    command.args([
        "--new-session",
        "--die-with-parent",
        "--unshare-user",
        "--unshare-pid",
        "--unshare-ipc",
        "--cap-drop",
        "ALL",
        "--ro-bind",
        "/",
        "/",
        "--dev",
        "/dev",
        "--proc",
        "/proc",
    ]);
    for mount in &request.mounts {
        let fd = mount.source_fd.context("validated mount fd disappeared")?;
        let source = format!("/proc/self/fd/{fd}");
        match mount.kind {
            MountKind::Writable => {
                command.arg("--bind").arg(source).arg(&mount.target);
            }
            MountKind::ReadOnly => {
                command.arg("--ro-bind").arg(source).arg(&mount.target);
            }
            MountKind::Unreadable | MountKind::MissingProtected => {
                if opaque_directories.contains(&mount.target) {
                    command
                        .arg("--perms")
                        .arg("000")
                        .arg("--tmpfs")
                        .arg(&mount.target);
                } else {
                    let source = format!(
                        "/proc/self/fd/{}",
                        empty_fd.context("missing unreadable-file fd")?
                    );
                    command.arg("--ro-bind").arg(source).arg(&mount.target);
                }
            }
        }
    }
    command
        .arg("--chdir")
        .arg(&request.cwd)
        .arg("--")
        .arg(&current_exe);
    command.args(super::runner::helper_args(&current_exe, encoded));
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let mut keep: Vec<i32> = request
        .mounts
        .iter()
        .filter_map(|mount| mount.source_fd)
        .collect();
    keep.push(bwrap_fd);
    keep.push(status_fd);
    if let Some(fd) = empty_fd {
        keep.push(fd);
    }
    // SAFETY: pre_exec runs after fork in the single child. close_unlisted_fds
    // only uses libc calls and stack data captured before the fork.
    unsafe {
        command.pre_exec(move || {
            mark_all_fds_cloexec();
            for fd in &keep {
                let flags = libc::fcntl(*fd, libc::F_GETFD);
                if flags < 0 || libc::fcntl(*fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    command.process_group(0);
    let mut child = command.spawn().context("start verified bubblewrap")?;
    drop(inherited);
    let bwrap_status = wait_forwarding_signals(&mut child)?;
    let status = read_raw_status(&mut status_read).unwrap_or(bwrap_status);
    report_dynamic_glob_creations(&request)?;
    drop(placeholders);
    Ok(status)
}

struct MissingPlaceholder {
    path: std::path::PathBuf,
    identity: BwrapIdentity,
}

impl Drop for MissingPlaceholder {
    fn drop(&mut self) {
        let unchanged = std::fs::symlink_metadata(&self.path)
            .map(|metadata| identity_from_metadata(&metadata) == self.identity)
            .unwrap_or(false);
        if unchanged {
            let _ = std::fs::remove_dir(&self.path);
        }
    }
}

fn report_dynamic_glob_creations(request: &LinuxSandboxRequest) -> Result<()> {
    use std::collections::BTreeSet;
    let mut created = 0usize;
    for snapshot in &request.glob_snapshots {
        let before: BTreeSet<_> = snapshot.matches.iter().collect();
        let after = super::plan::expand_glob(&snapshot.pattern)?;
        created += after.iter().filter(|path| !before.contains(path)).count();
    }
    if created > 0 {
        let violation = super::violation::LinuxSandboxViolation {
            kind: super::violation::LinuxViolationKind::DynamicGlobCreated,
            path_provenance: "glob_snapshot".into(),
            policy_digest: request.policy_digest.clone(),
            detection_only: true,
            affected_count: created,
        };
        println!("{}", super::violation::marker(&violation));
    }
    Ok(())
}

fn run_inner(request: LinuxSandboxRequest) -> Result<ExitStatus> {
    for mount in &request.mounts {
        let actual = std::fs::metadata(&mount.target)
            .with_context(|| format!("verify mounted target {}", mount.target.display()))?;
        if matches!(
            mount.kind,
            MountKind::Unreadable | MountKind::MissingProtected
        ) {
            if actual.permissions().mode() & 0o777 != 0 {
                return Err(anyhow!(
                    "protected target permissions are not empty: {}",
                    mount.target.display()
                ));
            }
        } else {
            let expected = mount.expected.as_ref().context("missing mount identity")?;
            if identity_from_metadata(&actual) != *expected {
                return Err(anyhow!(
                    "mounted target identity changed: {}",
                    mount.target.display()
                ));
            }
        }
    }
    // SAFETY: PR_SET_NO_NEW_PRIVS has no pointer arguments and is applied to
    // this dedicated helper immediately before it creates the user command.
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(std::io::Error::last_os_error()).context("set no_new_privs");
    }
    let status_fd = request.status_fd.context("missing status fd")?;
    set_cloexec(status_fd)?;
    close_unlisted_fds(&[status_fd]);
    let (program, argv) = request.argv.split_first().context("empty command argv")?;
    let mut command = Command::new(program);
    command.args(argv).current_dir(&request.cwd);
    // A separate child group lets PID 1 forward one signal to the complete
    // command tree and kill any surviving descendants after the leader exits.
    command.process_group(0);
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let mut child = command.spawn().context("start sandboxed shell")?;
    let child_pgid = child.id() as i32;
    let status = wait_forwarding_signals(&mut child)?;
    // SAFETY: the group belongs to the child just reaped; ESRCH is expected
    // when it had no descendants.
    unsafe { libc::killpg(child_pgid, libc::SIGKILL) };
    reap_all_children();
    Ok(status)
}

fn wait_forwarding_signals(child: &mut std::process::Child) -> Result<ExitStatus> {
    let target = Arc::new(AtomicI32::new(child.id() as i32));
    let mut signals = signal_hook::iterator::Signals::new([
        libc::SIGTERM,
        libc::SIGINT,
        libc::SIGHUP,
        libc::SIGQUIT,
    ])?;
    let handle = signals.handle();
    let signal_target = target.clone();
    let thread = std::thread::spawn(move || {
        for signal in signals.forever() {
            let pid = signal_target.load(Ordering::SeqCst);
            if pid > 0 {
                // SAFETY: forwarding an integer signal to our direct child's
                // process group; failure only means it already exited.
                unsafe { libc::killpg(pid, signal) };
            }
        }
    });
    let status = child.wait().context("wait for sandbox child")?;
    target.store(0, Ordering::SeqCst);
    handle.close();
    let _ = thread.join();
    Ok(status)
}

struct OpaqueFile {
    file: File,
    path: std::path::PathBuf,
}

impl Drop for OpaqueFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn create_opaque_file() -> Result<OpaqueFile> {
    static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    for _ in 0..100 {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            ".future-sandbox-unreadable-{}-{id}",
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o0)
            .open(&path)
        {
            Ok(file) => return Ok(OpaqueFile { file, path }),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(anyhow!("could not allocate unreadable-file source"))
}

fn create_pipe() -> Result<(File, File)> {
    let mut fds = [-1; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: pipe2 returned two owned descriptors.
    Ok(unsafe { (File::from_raw_fd(fds[0]), File::from_raw_fd(fds[1])) })
}

fn write_raw_status(fd: i32, status: ExitStatus) -> Result<()> {
    let bytes = status.into_raw().to_ne_bytes();
    // SAFETY: this helper exclusively owns the inherited status descriptor.
    let mut file = unsafe { File::from_raw_fd(fd) };
    file.write_all(&bytes).context("write command status")
}

fn read_raw_status(file: &mut File) -> Option<ExitStatus> {
    let mut bytes = [0; std::mem::size_of::<i32>()];
    file.read_exact(&mut bytes).ok()?;
    Some(ExitStatus::from_raw(i32::from_ne_bytes(bytes)))
}

fn open_mount_source(path: &std::path::Path) -> Result<File> {
    let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| anyhow!("NUL in path"))?;
    // O_PATH pins the inode without requiring read permission and works for
    // files and directories. CLOEXEC is cleared only for this explicit mount
    // allowlist before bwrap is spawned.
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: fd was returned by open and ownership is transferred to File.
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn clear_cloexec(fd: i32) -> Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) } < 0 {
        return Err(std::io::Error::last_os_error()).context("make mount fd inheritable");
    }
    Ok(())
}

fn set_cloexec(fd: i32) -> Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(std::io::Error::last_os_error()).context("mark status fd close-on-exec");
    }
    Ok(())
}

fn close_unlisted_fds(keep: &[i32]) {
    let max = open_max();
    for fd in 3..max {
        if !keep.contains(&fd) {
            unsafe { libc::close(fd) };
        }
    }
}

fn mark_all_fds_cloexec() {
    const CLOSE_RANGE_CLOEXEC: libc::c_uint = 1 << 2;
    let result =
        unsafe { libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, CLOSE_RANGE_CLOEXEC) };
    if result == 0 {
        return;
    }
    for fd in 3..open_max() {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if flags >= 0 {
            unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) };
        }
    }
}

fn open_max() -> i32 {
    let max = unsafe { libc::sysconf(libc::_SC_OPEN_MAX) };
    if max <= 0 {
        4096
    } else {
        max.min(65_536) as i32
    }
}

fn identity_from_metadata(metadata: &std::fs::Metadata) -> BwrapIdentity {
    use std::os::unix::fs::MetadataExt;
    let modified_nanos = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_nanos());
    BwrapIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        size: metadata.len(),
        modified_nanos,
    }
}

fn reap_all_children() {
    loop {
        let result = unsafe { libc::waitpid(-1, std::ptr::null_mut(), libc::WNOHANG) };
        if result <= 0 {
            break;
        }
    }
}

fn mirror_status(status: ExitStatus) -> ! {
    if let Some(signal) = status.signal() {
        unsafe {
            libc::signal(signal, libc::SIG_DFL);
            libc::raise(signal);
        }
        std::process::exit(128 + signal);
    }
    std::process::exit(status.code().unwrap_or(HELPER_INFRA_EXIT));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_new_privs_can_be_applied_in_a_test_process() {
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            "grep -q '^NoNewPrivs:[[:space:]]*1' /proc/self/status",
        ]);
        unsafe {
            command.pre_exec(|| {
                if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.status().unwrap();
        assert!(child.success());
    }
}
