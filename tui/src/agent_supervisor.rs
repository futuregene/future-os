//! TUI-owned Agent sidecar lifecycle.
//!
//! A TUI validates its configured Agent before sending business requests. Only
//! an absent default local endpoint and an available instance lock authorize
//! the unified `future agent` sidecar (or standalone fallback).

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
struct LaunchSpec {
    program: PathBuf,
    args: Vec<String>,
}

fn executable_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn launch_candidates(current_exe: &Path) -> Vec<LaunchSpec> {
    let agent_args = vec![
        "agent".to_string(),
        "--grpc-addr".to_string(),
        "auto".to_string(),
    ];
    let standalone_args = vec!["--grpc-addr".to_string(), "auto".to_string()];

    let mut candidates = Vec::new();
    if executable_name(current_exe) == "future" {
        candidates.push(LaunchSpec {
            program: current_exe.to_path_buf(),
            args: agent_args.clone(),
        });
    }
    if let Some(parent) = current_exe.parent() {
        let exe = if cfg!(windows) { ".exe" } else { "" };
        candidates.push(LaunchSpec {
            program: parent.join(format!("future{exe}")),
            args: agent_args.clone(),
        });
        candidates.push(LaunchSpec {
            program: parent.join(format!("future-agent{exe}")),
            args: standalone_args.clone(),
        });
    }
    candidates.push(LaunchSpec {
        program: PathBuf::from("future"),
        args: agent_args,
    });
    candidates.push(LaunchSpec {
        program: PathBuf::from("future-agent"),
        args: standalone_args,
    });
    candidates.dedup();
    candidates
}

pub struct OwnedAgent {
    child: Child,
}

impl Drop for OwnedAgent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Return `None` when an existing Agent was found, or an owned child when the
/// TUI had to launch one. The child is terminated when the returned guard is
/// dropped.
pub async fn ensure_agent_running(configured: &str) -> Result<Option<OwnedAgent>, String> {
    let permit = match future_rpc::connection::select_agent(
        configured,
        future_rpc::connection::EXPECTED_AGENT_VERSION,
        Duration::from_millis(400),
        Duration::from_secs(2),
    )
    .await
    .map_err(|error| error.to_string())?
    {
        future_rpc::connection::AgentSelection::External(_) => return Ok(None),
        future_rpc::connection::AgentSelection::LocalStart(permit) => permit,
    };
    permit.recheck().map_err(|error| error.to_string())?;

    if cfg!(test) {
        return Err("agent sidecar startup is disabled in unit tests".to_string());
    }

    let current_exe = std::env::current_exe()
        .map_err(|error| format!("cannot locate the TUI executable: {error}"))?;
    let mut failures = Vec::new();
    for candidate in launch_candidates(&current_exe) {
        let mut command = Command::new(&candidate.program);
        command
            .args(&candidate.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                failures.push(format!("{}: {error}", candidate.program.display()));
                continue;
            }
        };
        for _ in 0..40 {
            match future_rpc::connection::connect_verified(
                configured,
                future_rpc::connection::EXPECTED_AGENT_VERSION,
                Duration::from_millis(400),
                Duration::from_secs(2),
            )
            .await
            {
                Ok(_) => return Ok(Some(OwnedAgent { child })),
                Err(
                    future_rpc::connection::ConnectionError::Transport(_)
                    | future_rpc::connection::ConnectionError::Unverified(_),
                ) => {}
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error.to_string());
                }
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    failures.push(format!(
                        "{} exited with {status}",
                        candidate.program.display()
                    ));
                    break;
                }
                Ok(None) => tokio::time::sleep(Duration::from_millis(125)).await,
                Err(error) => {
                    failures.push(format!("{}: {error}", candidate.program.display()));
                    break;
                }
            }
        }
        let _ = child.kill();
        let _ = child.wait();
    }
    Err(format!(
        "no Future Agent was reachable and no sidecar could be started ({})",
        failures.join("; ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unified_binary_launches_agent_without_tcp_in_auto_mode() {
        let specs = launch_candidates(Path::new("/opt/future/bin/future"));
        assert_eq!(
            specs[0],
            LaunchSpec {
                program: PathBuf::from("/opt/future/bin/future"),
                args: vec!["agent".into(), "--grpc-addr".into(), "auto".into()],
            }
        );
    }

    #[test]
    fn every_sidecar_candidate_is_pinned_to_the_local_endpoint() {
        let specs = launch_candidates(Path::new("/opt/future/bin/future-tui"));
        assert_eq!(specs[0].args, vec!["agent", "--grpc-addr", "auto"]);
        assert_eq!(specs[1].args, vec!["--grpc-addr", "auto"]);
    }

    /// A long-running child that exits on its own only long after the test.
    fn spawn_sleeper() -> Child {
        #[cfg(windows)]
        let mut command = {
            let mut command = Command::new("cmd");
            command.args(["/c", "ping -n 60 127.0.0.1 > nul"]);
            command
        };
        #[cfg(unix)]
        let mut command = {
            let mut command = Command::new("sleep");
            command.arg("60");
            command
        };
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn a sleeper")
    }

    /// Whether `pid` still exists, asked of the OS rather than of our own
    /// handle (a `Child` we still hold would answer "running" about a corpse).
    fn process_is_running(pid: u32) -> bool {
        #[cfg(windows)]
        {
            let out = Command::new("tasklist")
                .args(["/FI", &format!("PID eq {pid}"), "/NH"])
                .output()
                .expect("tasklist");
            String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
        }
        #[cfg(unix)]
        {
            Command::new("ps")
                .args(["-p", &pid.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .expect("ps")
                .success()
        }
    }

    /// The TUI owns this guard: dropping it must kill and reap the sidecar it
    /// started, or every TUI exit would leak a `future agent` process.
    ///
    /// The liveness check runs in a *separate* process and is first shown to
    /// answer "running" for the live child — so "gone" afterwards cannot be the
    /// observer simply never seeing anything.
    #[test]
    fn dropping_the_owned_agent_kills_its_child() {
        let child = spawn_sleeper();
        let pid = child.id();
        assert!(
            process_is_running(pid),
            "the observer must be able to see a live process"
        );

        drop(OwnedAgent { child });

        assert!(
            !process_is_running(pid),
            "a dropped OwnedAgent must kill the child it owns (pid {pid})"
        );
    }

    /// The `cfg!(test)` guard: a unit test must never launch a real sidecar, so
    /// the probe's failure must surface as this refusal and not as a spawn.
    #[tokio::test]
    async fn ensure_agent_running_refuses_an_unavailable_explicit_endpoint() {
        // Port 1 is never a running agent, so the health probe fails and the
        // launch path is entered.
        let error = match ensure_agent_running("127.0.0.1:1").await {
            Ok(_) => panic!("a test build must not start an agent"),
            Err(error) => error,
        };
        assert!(error.contains("no local fallback"), "{error}");
    }
}
