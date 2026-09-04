#![cfg(target_os = "linux")]

use super::probe::BwrapIdentity;
use super::request::{HelperPhase, LinuxSandboxRequest, MountKind};
use anyhow::{anyhow, Context, Result};
use std::ffi::{CString, OsStr};
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

const HELPER_INFRA_EXIT: i32 = 125;
// `--args FD` removes the mount plan from execve's ARG_MAX accounting, but the
// FD payload still needs a deterministic resource ceiling of its own.
const MAX_BWRAP_ARGS_BYTES: usize = 16 * 1024 * 1024;
const LINUX_CAPABILITY_VERSION_3: u32 = 0x2008_0522;

/// Hidden helper entry point. It intentionally terminates the helper process
/// with the wrapped process status, so it must be dispatched before the Agent
/// singleton and runtime are initialized.
pub fn run_helper_request(reference: &str) -> ! {
    let request = if let Some(fd) = reference.strip_prefix("fd:") {
        fd.parse::<i32>()
            .map_err(|_| anyhow!("invalid helper request fd"))
            .and_then(read_request_fd)
    } else {
        LinuxSandboxRequest::decode(reference).map_err(anyhow::Error::from)
    };
    let outcome = request.and_then(run_request);
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
        if mount.kind == MountKind::MissingProtected {
            // No host source exists for this mask. Do not open/create one here.
            // IMPORTANT: bwrap itself may mkdir the destination before mounting
            // tmpfs. This needs an isolated parent view to guarantee zero host
            // residue; see LINUX_SANDBOX_FAILURE_REVIEW.md (P0, still open).
            continue;
        }
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
    let request_file = create_request_file(&request)?;
    let request_fd = request_file.as_raw_fd();
    clear_cloexec(request_fd)?;
    let current_exe = std::env::current_exe().context("resolve current helper executable")?;

    let bwrap_args = create_bwrap_args_file(
        &request,
        &opaque_directories,
        empty_fd,
        &current_exe,
        request_fd,
    )?;
    let bwrap_args_fd = bwrap_args.as_raw_fd();
    clear_cloexec(bwrap_args_fd)?;

    // Execute the already-verified inode through its inherited O_PATH fd;
    // replacing the pathname after validation cannot change this invocation.
    // All variable-size mount arguments travel through an anonymous FD so the
    // execve call cannot fail merely because the plan exceeds ARG_MAX.
    let mut command = Command::new(format!("/proc/self/fd/{bwrap_fd}"));
    command.arg("--args").arg(bwrap_args_fd.to_string());
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
    keep.push(request_fd);
    keep.push(bwrap_args_fd);
    if let Some(fd) = empty_fd {
        keep.push(fd);
    }
    inherited.push(request_file);
    inherited.push(bwrap_args);
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
    // Missing inner status means initialization never completed. Do not let
    // newly captured bwrap stderr (e.g. EPERM with exit 1) invite an unsandboxed
    // retry. Signals retain their cancellation semantics.
    let status = command_status(read_raw_status(&mut status_read), bwrap_status)?;
    if let Err(error) = report_dynamic_glob_creations(&request) {
        eprintln!("future-linux-sandbox-helper: post-command detection failed: {error:#}");
        // Detection happens after the command has completed. Never replace the
        // real command outcome with an infrastructure error or invite a retry
        // after side effects may already have occurred.
        let violation = super::violation::LinuxSandboxViolation {
            kind: super::violation::LinuxViolationKind::DynamicGlobScanFailed,
            path_provenance: "glob_rescan_failed".into(),
            policy_digest: request.policy_digest.clone(),
            detection_only: true,
            affected_count: 0,
        };
        println!("{}", super::violation::marker(&violation));
    }
    Ok(status)
}

fn command_status(inner: Option<ExitStatus>, bwrap: ExitStatus) -> Result<ExitStatus> {
    match inner {
        Some(status) => Ok(status),
        None if bwrap.signal().is_some() => Ok(bwrap),
        None => Err(anyhow!("bubblewrap did not report command status: {bwrap}")),
    }
}

fn mount_source_fd_path(mount: &super::request::MountRequest) -> Result<String> {
    let fd = mount.source_fd.context("validated mount fd disappeared")?;
    Ok(format!("/proc/self/fd/{fd}"))
}

fn create_bwrap_args_file(
    request: &LinuxSandboxRequest,
    opaque_directories: &std::collections::BTreeSet<std::path::PathBuf>,
    empty_fd: Option<i32>,
    current_exe: &std::path::Path,
    request_fd: i32,
) -> Result<File> {
    let mut file = tempfile::tempfile().context("create anonymous bubblewrap arguments")?;
    let mut written = 0usize;
    for arg in [
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
    ] {
        write_bwrap_arg(&mut file, &mut written, OsStr::new(arg))?;
    }
    for mount in &request.mounts {
        match mount.kind {
            MountKind::Writable | MountKind::ReadOnly => {
                let operation = if mount.kind == MountKind::Writable {
                    "--bind"
                } else {
                    "--ro-bind"
                };
                let source = mount_source_fd_path(mount)?;
                write_bwrap_arg(&mut file, &mut written, OsStr::new(operation))?;
                write_bwrap_arg(&mut file, &mut written, OsStr::new(&source))?;
                write_bwrap_arg(&mut file, &mut written, mount.target.as_os_str())?;
            }
            MountKind::Unreadable if !opaque_directories.contains(&mount.target) => {
                let source = format!(
                    "/proc/self/fd/{}",
                    empty_fd.context("missing unreadable-file fd")?
                );
                write_bwrap_arg(&mut file, &mut written, OsStr::new("--ro-bind"))?;
                write_bwrap_arg(&mut file, &mut written, OsStr::new(&source))?;
                write_bwrap_arg(&mut file, &mut written, mount.target.as_os_str())?;
            }
            MountKind::Unreadable | MountKind::MissingProtected => {
                // MissingProtected has no host source FD. This argument alone
                // does NOT guarantee namespace-only target creation: a writable
                // host bind underneath can be mutated by bwrap's mkdir. The
                // isolated-parent design remains a release blocker (review doc).
                for arg in ["--perms", "000", "--tmpfs"] {
                    write_bwrap_arg(&mut file, &mut written, OsStr::new(arg))?;
                }
                write_bwrap_arg(&mut file, &mut written, mount.target.as_os_str())?;
            }
        }
    }
    write_bwrap_arg(&mut file, &mut written, OsStr::new("--chdir"))?;
    write_bwrap_arg(&mut file, &mut written, request.cwd.as_os_str())?;
    write_bwrap_arg(&mut file, &mut written, OsStr::new("--"))?;
    write_bwrap_arg(&mut file, &mut written, current_exe.as_os_str())?;
    for arg in super::runner::helper_args(current_exe, format!("fd:{request_fd}")) {
        write_bwrap_arg(&mut file, &mut written, OsStr::new(&arg))?;
    }
    file.rewind().context("rewind bubblewrap arguments")?;
    Ok(file)
}

fn write_bwrap_arg(file: &mut File, written: &mut usize, arg: &OsStr) -> Result<()> {
    let bytes = arg.as_bytes();
    if bytes.contains(&0) {
        return Err(anyhow!("NUL in bubblewrap argument"));
    }
    let next = written
        .checked_add(bytes.len() + 1)
        .context("bubblewrap argument size overflow")?;
    if next > MAX_BWRAP_ARGS_BYTES {
        return Err(anyhow!(
            "bubblewrap argument payload exceeds {MAX_BWRAP_ARGS_BYTES} bytes"
        ));
    }
    file.write_all(bytes)?;
    file.write_all(&[0])?;
    *written = next;
    Ok(())
}

fn report_dynamic_glob_creations(request: &LinuxSandboxRequest) -> Result<()> {
    use std::collections::BTreeSet;
    let mut created = 0usize;
    let patterns = request
        .glob_snapshots
        .iter()
        .map(|snapshot| snapshot.pattern.clone())
        .collect::<Vec<_>>();
    let expanded = super::plan::expand_globs(&patterns, "post_command", &|| false)?;
    for snapshot in &request.glob_snapshots {
        let before: BTreeSet<_> = snapshot.matches.iter().collect();
        let after = &expanded[&snapshot.pattern];
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
        } else if mount.kind != MountKind::MissingProtected {
            let expected = mount.expected.as_ref().context("missing mount identity")?;
            // Source O_PATH FDs pin these inodes through this verification.
            // Mutable directory size/mtime (notably /tmp and an active repo)
            // are not mount identity; unrelated host writes must not reject
            // an otherwise correct bind. The bwrap executable check above
            // deliberately retains its stricter full identity comparison.
            if !same_mount_inode(expected, &identity_from_metadata(&actual)) {
                return Err(anyhow!(
                    "mounted target identity changed: {}",
                    mount.target.display()
                ));
            }
        }
    }
    // `--cap-drop ALL` is part of the outer bwrap contract. Verify the result
    // inside the namespace instead of trusting only command construction.
    verify_no_effective_or_permitted_capabilities()?;
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

fn verify_no_effective_or_permitted_capabilities() -> Result<()> {
    let mut header = [LINUX_CAPABILITY_VERSION_3, 0];
    let mut sets = [[0_u32; 3]; 2];
    // SAFETY: capability ABI v3 uses a [version, pid] header followed by two
    // [effective, permitted, inheritable] u32 records.
    let result = unsafe { libc::syscall(libc::SYS_capget, header.as_mut_ptr(), sets.as_mut_ptr()) };
    if result < 0 {
        return Err(std::io::Error::last_os_error()).context("verify sandbox capabilities");
    }
    if sets
        .into_iter()
        .any(|[effective, permitted, _]| effective != 0 || permitted != 0)
    {
        return Err(anyhow!(
            "sandbox retained effective or permitted Linux capabilities"
        ));
    }
    Ok(())
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

fn create_request_file(request: &LinuxSandboxRequest) -> Result<File> {
    let mut file = tempfile::tempfile().context("create anonymous helper request")?;
    file.write_all(&request.to_json_bytes()?)
        .context("write helper request")?;
    file.rewind().context("rewind helper request")?;
    Ok(file)
}

fn read_request_fd(fd: i32) -> Result<LinuxSandboxRequest> {
    if fd < 3 {
        return Err(anyhow!("invalid helper request fd"));
    }
    // SAFETY: the helper takes ownership of the dedicated inherited request fd.
    let file = unsafe { File::from_raw_fd(fd) };
    let mut bytes = Vec::new();
    file.take((super::request::MAX_REQUEST_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .context("read helper request")?;
    LinuxSandboxRequest::from_json_bytes(&bytes).map_err(anyhow::Error::from)
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

fn same_mount_inode(expected: &BwrapIdentity, actual: &BwrapIdentity) -> bool {
    expected.device == actual.device && expected.inode == actual.inode
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
    fn mount_identity_ignores_mutation_but_rejects_inode_replacement() {
        let expected = BwrapIdentity {
            device: 1,
            inode: 2,
            size: 3,
            modified_nanos: 4,
        };
        let mutated = BwrapIdentity {
            size: 99,
            modified_nanos: 100,
            ..expected.clone()
        };
        assert!(same_mount_inode(&expected, &mutated));
        assert!(!same_mount_inode(
            &expected,
            &BwrapIdentity {
                inode: 5,
                ..mutated.clone()
            }
        ));
        assert!(!same_mount_inode(
            &expected,
            &BwrapIdentity {
                device: 5,
                ..mutated
            }
        ));
    }

    #[test]
    fn missing_inner_status_is_infrastructure_not_a_command_denial() {
        assert!(command_status(None, ExitStatus::from_raw(1 << 8)).is_err());
        assert!(command_status(None, ExitStatus::from_raw(0)).is_err());
        assert_eq!(
            command_status(None, ExitStatus::from_raw(libc::SIGTERM))
                .unwrap()
                .signal(),
            Some(libc::SIGTERM)
        );
        assert_eq!(
            command_status(
                Some(ExitStatus::from_raw(23 << 8)),
                ExitStatus::from_raw(1 << 8)
            )
            .unwrap()
            .code(),
            Some(23)
        );
    }

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

    #[test]
    fn bwrap_argument_writer_uses_nul_separators_and_enforces_nul_free_input() {
        let mut file = tempfile::tempfile().unwrap();
        let mut written = 0;
        write_bwrap_arg(&mut file, &mut written, OsStr::new("--ro-bind")).unwrap();
        write_bwrap_arg(&mut file, &mut written, OsStr::new("/a path")).unwrap();
        file.rewind().unwrap();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"--ro-bind\0/a path\0");
        assert_eq!(written, bytes.len());

        assert!(write_bwrap_arg(&mut file, &mut written, OsStr::from_bytes(b"bad\0arg")).is_err());

        let mut exhausted = MAX_BWRAP_ARGS_BYTES;
        assert!(write_bwrap_arg(&mut file, &mut exhausted, OsStr::new("x")).is_err());
    }
}
