//! Headless uses the same scoped in-process owner as GUI. The bundled CLI
//! remains needed only for short maintenance commands such as skill bootstrap.
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;

#[derive(Default)]
pub(super) struct Agent;
impl Agent {
    pub(super) async fn ensure_running(&mut self) -> Result<(), crate::AppError> {
        let address = crate::agent_bridge::raw_agent_addr();
        tokio::task::spawn_blocking(move || crate::agent_host::host().start(address))
            .await
            .map_err(|error| error.to_string())??;
        Ok(())
    }
    pub(super) fn check_running(&mut self) -> Result<(), crate::AppError> {
        match crate::agent_host::host().phase() {
            crate::agent_host::Phase::Failed(error) => Err(error.into()),
            _ => Ok(()),
        }
    }
    pub(super) async fn shutdown(&mut self) -> Result<(), crate::AppError> {
        tokio::task::spawn_blocking(|| crate::desktop_shutdown::stop("headless_exit"))
            .await
            .map_err(|error| error.to_string())?
    }
}

/// The bundled `future` CLI: beside the running executable, then the explicitly
/// installed CLI on PATH. Also spawns `future init` for the built-in skills, so
/// the message names the binary rather than only the Agent.
pub(super) fn find_sidecar() -> Result<PathBuf, crate::AppError> {
    crate::cli_tool::resolve()
}

#[cfg(test)]
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
        let _home = crate::auth_store::test_support::HomeGuard::new("headless-stop");
        let _mock = crate::agent_bridge::test_support::mock_agent();
        let mut agent = Agent;
        agent.check_running().unwrap();
        agent.shutdown().await.unwrap();
        assert!(!crate::agent_host::host().owns_agent());
    }
}
