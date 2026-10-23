//! Shared instance-lock conflict diagnostics.
//!
//! The Agent publishes `agent-instance.json` next to its lock while it runs.
//! Both the Agent itself (on a failed `acquire`) and any client about to start
//! a local Agent (the preflight in [`crate::connection::check_instance_lock`])
//! must explain *who* holds the lock — crucial on shared-home setups (HPC login
//! and compute nodes share one FutureOS home over NFS), where the conflicting
//! process is on another machine and `kill` cannot reach it.

use std::path::Path;

/// The host name of the machine running this process, for instance metadata
/// and lock-conflict diagnostics. Empty/unavailable resolves to `None`; the
/// diagnostics degrade gracefully without it.
pub fn host_name() -> Option<String> {
    host_name_from_parts(hostname_raw(), std::env::var("COMPUTERNAME").ok())
}

#[cfg(unix)]
fn hostname_raw() -> Option<String> {
    let mut buffer = [0u8; 256];
    let length = unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) };
    if length != 0 {
        return None;
    }
    let end = buffer
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(buffer.len());
    Some(String::from_utf8_lossy(&buffer[..end]).trim().to_string())
}

#[cfg(windows)]
fn hostname_raw() -> Option<String> {
    use windows_sys::core::BOOL;
    use windows_sys::Win32::System::WindowsProgramming::GetComputerNameW;
    let mut buffer = [0u16; 256];
    let mut size = buffer.len() as u32;
    let ok: BOOL = unsafe { GetComputerNameW(buffer.as_mut_ptr(), &mut size) };
    if ok == 0 {
        return None;
    }
    Some(
        String::from_utf16_lossy(&buffer[..size as usize])
            .trim()
            .to_string(),
    )
}

/// Combine the OS host name and (on Windows) `COMPUTERNAME` into one
/// diagnostic value, dropping whitespace-only results. Split from
/// [`host_name`] so the composition is testable without touching the
/// environment or the OS.
fn host_name_from_parts(os_name: Option<String>, env_name: Option<String>) -> Option<String> {
    let os_name = os_name
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let env_name = env_name
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    os_name.or(env_name)
}

/// Build the error shown when the instance lock is already held. The
/// metadata file published by the running owner (best-effort) identifies
/// *where* the Agent runs — crucial on shared-home setups (HPC login and
/// compute nodes share one FutureOS home over NFS), where the conflicting
/// process is on another machine and `kill` cannot reach it: the only local
/// fixes are waiting for that node's job to end or using a different home.
pub fn lock_conflict_message(lock_path: &Path, future_home: &Path) -> String {
    let owner = std::fs::read(lock_path.with_file_name("agent-instance.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
    lock_conflict_message_from(
        lock_path,
        future_home,
        owner.as_ref(),
        host_name().as_deref(),
    )
}

/// [`lock_conflict_message`] with every input injected so both the same-host
/// and cross-host arms are testable without spawning processes or depending
/// on where the test runs.
fn lock_conflict_message_from(
    lock_path: &Path,
    future_home: &Path,
    owner: Option<&serde_json::Value>,
    this_host: Option<&str>,
) -> String {
    let mut message = format!(
        "Future Agent is already running for this FutureOS home (lock: {})",
        lock_path.display()
    );
    let Some(owner) = owner else {
        return message;
    };
    let pid = owner.get("pid").and_then(serde_json::Value::as_u64);
    let host = owner
        .get("host")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let executable = owner
        .get("executable")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let where_ = match (&host, pid) {
        (Some(host), Some(pid)) => format!("pid {pid} on host {host}"),
        (Some(host), None) => format!("host {host}"),
        (None, Some(pid)) => format!("pid {pid}"),
        (None, None) => String::new(),
    };
    if !where_.is_empty() {
        message.push_str(&format!("\n  held by: {where_}"));
    }
    if let Some(executable) = executable {
        message.push_str(&format!("\n  executable: {executable}"));
    }
    // A lock held on another host cannot be killed locally; only a
    // different FutureOS home (or waiting for that process to exit) helps.
    let cross_host = host
        .as_deref()
        .zip(this_host)
        .is_some_and(|(owner_host, this_host)| owner_host != this_host);
    if cross_host {
        let owner_host = host.as_deref().unwrap_or_default();
        message.push_str(&format!(
            "\n  the lock is held on a different host ({owner_host}); this machine \
             cannot stop that process — wait for it to exit, or run this Agent \
             with its own FutureOS home, e.g.:"
        ));
    } else {
        message.push_str(&format!(
            "\n  to stop it: kill {} (or close the owning app), or run this Agent \
             with its own FutureOS home, e.g.:",
            pid.map(|p| p.to_string())
                .unwrap_or_else(|| "<pid>".to_string())
        ));
    }
    message.push_str(&env_home_example(future_home));
    message.push_str(&format!(
        "\n  metadata (advisory, may be stale after a crash): {}",
        lock_path.with_file_name("agent-instance.json").display()
    ));
    message
}

/// The suggested `FUTURE_HOME` example derives a per-host sibling of the
/// current FutureOS home (`/nfs/home/.future` → `/nfs/home/.future-$(hostname)`),
/// so machines sharing one filesystem each get their own lock and state.
fn env_home_example(future_home: &Path) -> String {
    let token = host_token_example();
    let sibling = match (future_home.parent(), future_home.file_name()) {
        (Some(parent), Some(name)) => parent
            .join(format!("{}-{token}", name.to_string_lossy()))
            .to_string_lossy()
            .into_owned(),
        // No parent or file name to build a sibling from — unreachable via a
        // validated home, but the diagnostic must never fail to render.
        _ => format!("{}-{token}", future_home.display()),
    };
    if cfg!(windows) {
        // cmd cannot set a variable inline; two statements is the honest form.
        format!("\n    set FUTURE_HOME={sibling}\n    future agent")
    } else {
        format!("\n    FUTURE_HOME={sibling} future agent")
    }
}

/// Host placeholder used in the suggested `FUTURE_HOME` example. `$(hostname)`
/// is shell syntax, `%COMPUTERNAME%` is the cmd equivalent.
fn host_token_example() -> &'static str {
    if cfg!(windows) {
        "%COMPUTERNAME%"
    } else {
        "$(hostname)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_name_prefers_the_os_and_ignores_blanks() {
        assert_eq!(
            host_name_from_parts(Some("intel080".into()), None),
            Some("intel080".to_string())
        );
        assert_eq!(
            host_name_from_parts(Some("  ".into()), Some("WIN-PC".into())),
            Some("WIN-PC".to_string())
        );
        assert_eq!(host_name_from_parts(None, Some("".into())), None);
        assert_eq!(host_name_from_parts(None, None), None);
    }

    #[test]
    fn cross_host_conflict_explains_the_lock_is_out_of_reach() {
        let lock = std::path::Path::new("/nfs/home/.future/agent/agent-instance.lock");
        let home = std::path::Path::new("/nfs/home/.future");
        let owner = serde_json::json!({
            "pid": 4242,
            "host": "intel080",
            "executable": "/nfs/home/bin/future-agent",
        });
        let message = lock_conflict_message_from(lock, home, Some(&owner), Some("login01"));
        assert!(message.contains("pid 4242 on host intel080"));
        assert!(message.contains("different host"));
        assert!(message.contains("FUTURE_HOME"));
        assert!(
            !message.contains("kill 4242"),
            "must not suggest killing a remote process"
        );
    }

    #[test]
    fn same_host_conflict_suggests_killing_the_owner() {
        let lock = std::path::Path::new("/home/u/.future/agent/agent-instance.lock");
        let home = std::path::Path::new("/home/u/.future");
        let owner = serde_json::json!({"pid": 4242, "host": "intel080"});
        let message = lock_conflict_message_from(lock, home, Some(&owner), Some("intel080"));
        assert!(message.contains("held by: pid 4242 on host intel080"));
        assert!(message.contains("kill 4242"));
        assert!(message.contains("FUTURE_HOME"));
    }

    /// The suggested `FUTURE_HOME` must point at a *sibling* of the current
    /// FutureOS home (`.future-$(hostname)` next to `.future`), not the home
    /// itself with a suffix appended — `FUTURE_HOME=/h/.future.future-…` would
    /// nest the per-host home inside the shared one.
    #[test]
    fn suggested_home_is_a_per_host_sibling_of_the_current_home() {
        let home = if cfg!(windows) {
            Path::new(r"C:\nfs\home\.future")
        } else {
            Path::new("/nfs/home/.future")
        };
        let lock = home.join("agent").join("agent-instance.lock");
        let owner = serde_json::json!({"pid": 4242, "host": "intel080"});
        let message = lock_conflict_message_from(&lock, home, Some(&owner), Some("login01"));
        if cfg!(windows) {
            assert!(
                message.contains(r"set FUTURE_HOME=C:\nfs\home\.future-%COMPUTERNAME%"),
                "{message}"
            );
        } else {
            assert!(
                message.contains("FUTURE_HOME=/nfs/home/.future-$(hostname) future agent"),
                "{message}"
            );
        }
        assert!(
            !message.contains(".future.future"),
            "the suggestion must not extend the current home's name: {message}"
        );
        assert!(
            !message.contains("--home"),
            "the flag is gone; the environment variable is the only switch: {message}"
        );
    }

    #[test]
    fn owner_description_covers_host_only_pid_only_and_neither() {
        let lock = std::path::Path::new("/h/.future/agent/agent-instance.lock");
        let home = std::path::Path::new("/h/.future");

        // Host without a pid, on another host: the owner is named by host alone
        // and the cross-host wording follows (no pid is needed to explain it).
        let only_host = lock_conflict_message_from(
            lock,
            home,
            Some(&serde_json::json!({"host": "intel080"})),
            Some("login01"),
        );
        assert!(only_host.contains("held by: host intel080"), "{only_host}");
        assert!(only_host.contains("different host"), "{only_host}");

        // Pid without a host, same host as ours: the owner is named by pid, and
        // the local advice uses the pid we have.
        let only_pid = lock_conflict_message_from(
            lock,
            home,
            Some(&serde_json::json!({"pid": 7})),
            host_name().as_deref(),
        );
        assert!(only_pid.contains("held by: pid 7"), "{only_pid}");
        assert!(only_pid.contains("kill 7"), "{only_pid}");

        // Neither: there is no owner to name, so the message must not invent a
        // "held by" line — the location advice stays, with a placeholder pid.
        let neither = lock_conflict_message_from(
            lock,
            home,
            Some(&serde_json::json!({"pid": null, "host": null})),
            Some("login01"),
        );
        assert!(!neither.contains("held by:"), "{neither}");
        assert!(neither.contains("kill <pid>"), "{neither}");
    }

    #[test]
    fn missing_or_unreadable_metadata_degrades_to_the_plain_message() {
        let lock = std::path::Path::new("/h/.future/agent/agent-instance.lock");
        let home = std::path::Path::new("/h/.future");
        let message = lock_conflict_message_from(lock, home, None, Some("intel080"));
        assert_eq!(
            message,
            "Future Agent is already running for this FutureOS home (lock: /h/.future/agent/agent-instance.lock)"
        );
    }

    /// The full (non-injected) entry point reads whatever metadata sits next
    /// to the lock: a readable, owner-publishing file must be reflected in the
    /// message, and an unreadable one must degrade to the plain message.
    #[test]
    fn lock_conflict_message_reads_the_owner_metadata_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("agent-instance.lock");
        let metadata = dir.path().join("agent-instance.json");
        std::fs::write(
            &metadata,
            serde_json::json!({"pid": 4242, "host": "some-other-host"}).to_string(),
        )
        .unwrap();
        let message = lock_conflict_message(&lock, dir.path());
        assert!(
            message.contains("held by: pid 4242 on host some-other-host"),
            "{message}"
        );

        std::fs::write(&metadata, b"not json").unwrap();
        let degraded = lock_conflict_message(&lock, dir.path());
        assert!(
            degraded.starts_with("Future Agent is already running"),
            "{degraded}"
        );
        assert!(!degraded.contains("held by"), "{degraded}");
    }
}
