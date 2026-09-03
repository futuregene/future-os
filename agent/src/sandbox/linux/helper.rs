#![cfg(target_os = "linux")]

use super::probe::BwrapIdentity;
use super::request::{HelperPhase, LinuxSandboxRequest, MountKind};
use anyhow::{anyhow, Context, Result};
use std::ffi::CString;
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;
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
        HelperPhase::Inner => run_inner(request),
    }
}

fn run_outer(mut request: LinuxSandboxRequest) -> Result<ExitStatus> {
    let bwrap = open_mount_source(&request.bwrap_path).context("open verified bwrap")?;
    if identity_from_metadata(&bwrap.metadata()?) != request.bwrap_identity {
        return Err(anyhow!("verified bwrap identity changed"));
    }
    clear_cloexec(bwrap.as_raw_fd())?;
    let bwrap_fd = bwrap.as_raw_fd();
    let mut inherited = Vec::with_capacity(request.mounts.len() + 1);
    inherited.push(bwrap);
    for mount in &mut request.mounts {
        let file = open_mount_source(&mount.source)
            .with_context(|| format!("mount source is unavailable: {}", mount.source.display()))?;
        let fd = file.as_raw_fd();
        clear_cloexec(fd)?;
        mount.expected = Some(identity_from_metadata(&file.metadata()?));
        mount.source_fd = Some(fd);
        inherited.push(file);
    }
    request.phase = HelperPhase::Inner;
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
            MountKind::Writable => command.arg("--bind"),
            MountKind::ReadOnly | MountKind::Unreadable => command.arg("--ro-bind"),
        };
        command.arg(source).arg(&mount.target);
        if mount.kind == MountKind::Unreadable {
            command.arg("--chmod").arg("000").arg(&mount.target);
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
    wait_forwarding_signals(&mut child)
}

fn run_inner(request: LinuxSandboxRequest) -> Result<ExitStatus> {
    for mount in &request.mounts {
        let actual = std::fs::metadata(&mount.target)
            .with_context(|| format!("verify mounted target {}", mount.target.display()))?;
        let expected = mount.expected.as_ref().context("missing mount identity")?;
        if identity_from_metadata(&actual) != *expected {
            return Err(anyhow!(
                "mounted target identity changed: {}",
                mount.target.display()
            ));
        }
    }
    // SAFETY: PR_SET_NO_NEW_PRIVS has no pointer arguments and is applied to
    // this dedicated helper immediately before it creates the user command.
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(std::io::Error::last_os_error()).context("set no_new_privs");
    }
    close_unlisted_fds(&[]);
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
    use std::os::unix::process::ExitStatusExt;
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
