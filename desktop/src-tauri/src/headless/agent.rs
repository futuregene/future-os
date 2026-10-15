//! Headless uses the same scoped in-process owner as GUI. The bundled CLI
//! remains needed only for short maintenance commands such as skill bootstrap.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shutdown_without_an_owned_service_is_a_noop() {
        let _home = crate::auth_store::test_support::HomeGuard::new("headless-stop");
        let _mock = crate::agent_bridge::test_support::mock_agent();
        let mut agent = Agent;
        agent.check_running().unwrap();
        agent.shutdown().await.unwrap();
        assert!(!crate::agent_host::host().owns_agent());
    }
}
