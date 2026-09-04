//! TUI-owned Agent sidecar lifecycle.
//!
//! A TUI first probes every configured transport. If no Agent is reachable it
//! launches the unified `future agent` sidecar (or the standalone
//! `future-agent` fallback) and owns that child until the TUI exits.

use future_rpc::proto::future_agent_client::FutureAgentClient;
use future_rpc::proto::RpcCommand;
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

fn launch_candidates(current_exe: &Path, configured: &str) -> Vec<LaunchSpec> {
    let mut agent_args = vec!["agent".to_string()];
    let mut standalone_args = Vec::new();
    if !configured.eq_ignore_ascii_case(future_rpc::transport::AUTO_ENDPOINT) {
        let addr = configured
            .trim_start_matches("http://")
            .trim_start_matches("https://")
            .to_string();
        agent_args.extend(["--grpc-addr".to_string(), addr.clone()]);
        standalone_args.extend(["--grpc-addr".to_string(), addr]);
    }

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

async fn agent_healthy(configured: &str) -> bool {
    let Ok(connected) = future_rpc::transport::connect_channel(
        Some(configured),
        Duration::from_millis(400),
        Duration::from_secs(2),
    )
    .await
    else {
        return false;
    };
    let mut client = FutureAgentClient::new(connected.channel);
    client
        .execute_command(RpcCommand {
            id: uuid::Uuid::new_v4().to_string(),
            r#type: "list_streaming_sessions".to_string(),
            ..Default::default()
        })
        .await
        .map(|response| response.into_inner().success)
        .unwrap_or(false)
}

/// Return `None` when an existing Agent was found, or an owned child when the
/// TUI had to launch one. The child is terminated when the returned guard is
/// dropped.
pub async fn ensure_agent_running(configured: &str) -> Result<Option<OwnedAgent>, String> {
    if agent_healthy(configured).await {
        return Ok(None);
    }

    if cfg!(test) {
        return Err("agent sidecar startup is disabled in unit tests".to_string());
    }

    let current_exe = std::env::current_exe()
        .map_err(|error| format!("cannot locate the TUI executable: {error}"))?;
    let mut failures = Vec::new();
    for candidate in launch_candidates(&current_exe, configured) {
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
            if agent_healthy(configured).await {
                return Ok(Some(OwnedAgent { child }));
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
        let specs = launch_candidates(Path::new("/opt/future/bin/future"), "auto");
        assert_eq!(
            specs[0],
            LaunchSpec {
                program: PathBuf::from("/opt/future/bin/future"),
                args: vec!["agent".into()],
            }
        );
    }

    #[test]
    fn explicit_tcp_is_forwarded_only_when_requested() {
        let specs = launch_candidates(
            Path::new("/opt/future/bin/future-tui"),
            "http://127.0.0.1:55001",
        );
        assert_eq!(
            specs[0].args,
            vec!["agent", "--grpc-addr", "127.0.0.1:55001"]
        );
        assert_eq!(specs[1].args, vec!["--grpc-addr", "127.0.0.1:55001"]);
    }
}
