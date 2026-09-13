//! Agent ownership for the terminal entry point. No Tauri shell plugin needed.
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

#[derive(Default)]
pub(super) struct Agent {
    child: Option<tokio::process::Child>,
}

impl Agent {
    pub(super) async fn ensure_running(&mut self) -> Result<(), crate::AppError> {
        let address = crate::agent_bridge::raw_agent_addr();
        if reachable(&address).await {
            eprintln!("Using the existing Agent; it will not be stopped on exit.");
            return Ok(());
        }
        // An explicit endpoint belongs to its operator. Never start an agent
        // that accidentally takes over an unreachable external endpoint.
        if !address.eq_ignore_ascii_case(future_rpc::transport::AUTO_ENDPOINT) {
            return Err("The configured Agent is unavailable. Start it first, or use the default local endpoint.".into());
        }
        let executable = find_sidecar()?;
        let mut command = tokio::process::Command::new(&executable);
        command.arg("agent").stdin(Stdio::null()).kill_on_drop(true);
        // Only the host receives terminal signals; it first closes remote
        // access, then stops the child it owns. Never signal an external agent.
        #[cfg(unix)]
        command.process_group(0);
        #[cfg(windows)]
        command.creation_flags(0x00000200); // CREATE_NEW_PROCESS_GROUP
        self.child = Some(
            command
                .spawn()
                .map_err(|e| format!("Start {}: {e}", executable.display()))?,
        );
        eprintln!(
            "Started the bundled Agent. Ctrl+C will also stop this Agent and its active runs."
        );
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                self.check_running()?;
                if reachable(&address).await {
                    return Ok::<_, crate::AppError>(());
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        })
        .await
        .map_err(|_| crate::AppError::from("Agent startup timed out."))?
    }

    pub(super) fn check_running(&mut self) -> Result<(), crate::AppError> {
        if let Some(child) = &mut self.child {
            if let Some(status) = child.try_wait()? {
                return Err(format!(
                    "The owned Agent exited ({status}). Restart headless Desktop."
                )
                .into());
            }
        }
        Ok(())
    }

    pub(super) async fn shutdown(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        // Let the Agent cancel tool subprocess groups and settle active runs
        // before killing it. This branch is never entered for an external Agent.
        if tokio::time::timeout(Duration::from_secs(4), async {
            let sessions = crate::store::active_run_sessions().unwrap_or_default();
            futures::future::join_all(sessions.iter().map(|session| async move {
                if let Err(error) = crate::agent_bridge::abort_session(session).await {
                    eprintln!("Unable to abort an owned Agent session on exit: {error}");
                }
                crate::agent_bridge::wait_for_agent_idle(session).await;
            }))
            .await;
        })
        .await
        .is_err()
        {
            eprintln!("Timed out cancelling active runs; stopping the owned Agent.");
        }
        #[cfg(windows)]
        match tokio::time::timeout(
            Duration::from_secs(5),
            crate::agent_bridge::reset_windows_sandbox(),
        )
        .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => eprintln!("Agent sandbox cleanup failed: {error}"),
            Err(_) => eprintln!("Agent sandbox cleanup timed out."),
        }
        // kill_on_drop is also armed during startup/error cancellation.
        if child.try_wait().ok().flatten().is_none() {
            if let Err(error) = child.start_kill() {
                eprintln!("Unable to stop the owned Agent: {error}");
            }
            match tokio::time::timeout(Duration::from_secs(3), child.wait()).await {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => eprintln!("Unable to reap the owned Agent: {error}"),
                Err(_) => eprintln!("Timed out waiting for the owned Agent to exit."),
            }
        }
    }
}

async fn reachable(address: &str) -> bool {
    future_rpc::transport::connect_channel(
        Some(address),
        Duration::from_secs(2),
        Some(Duration::from_secs(2)),
    )
    .await
    .is_ok()
}

fn find_sidecar() -> Result<PathBuf, crate::AppError> {
    let current = std::env::current_exe()?;
    let parent = current
        .parent()
        .ok_or("Desktop executable has no parent directory")?;
    let name = if cfg!(windows) {
        "future.exe"
    } else {
        "future"
    };
    // Packaged sidecar beside Desktop, then the explicitly installed CLI on
    // PATH. Do not search the current directory or spawn through a shell.
    sidecar_candidates(parent, std::env::var_os("PATH").as_deref(), name)
        .into_iter()
        .find(|path| path.is_absolute() && path.is_file())
        .ok_or_else(|| "No Agent found. Place the matching future CLI beside Desktop, install it on PATH, or start `future agent` separately.".into())
}

fn sidecar_candidates(parent: &Path, path: Option<&std::ffi::OsStr>, name: &str) -> Vec<PathBuf> {
    let mut candidates = vec![parent.join(name)];
    if let Some(path) = path {
        candidates.extend(
            std::env::split_paths(path)
                .filter(|p| p.is_absolute())
                .map(|p| p.join(name)),
        );
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_resolution_prefers_bundle_and_ignores_relative_path_entries() {
        let directory = tempfile::tempdir().unwrap();
        let bin = directory.path().join("installed");
        let path = std::env::join_paths([Path::new("."), bin.as_path()]).unwrap();
        assert_eq!(
            sidecar_candidates(directory.path(), Some(&path), "future"),
            vec![directory.path().join("future"), bin.join("future")]
        );
    }

    #[tokio::test]
    async fn shutdown_without_an_owned_child_is_a_noop() {
        let mut agent = Agent::default();
        agent.check_running().unwrap();
        agent.shutdown().await;
        assert!(agent.child.is_none());
    }
}
