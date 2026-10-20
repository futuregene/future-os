//! Desktop maintenance discovery adapter; shared contract and cache live in Host.
use std::path::PathBuf;

pub(crate) fn resolve() -> Result<PathBuf, crate::AppError> {
    #[cfg(test)]
    future_host::test_support::configure_entry_fixture();
    future_host::cli_tool::resolve()
}

pub(crate) fn resolve_for(capability: &str) -> Result<PathBuf, crate::AppError> {
    #[cfg(test)]
    future_host::test_support::configure_entry_fixture();
    future_host::cli_tool::resolve_for(capability)
}

#[cfg(feature = "gui")]
pub(crate) fn discover(
    capability: Option<&str>,
) -> Result<future_host::cli_tool::Descriptor, crate::AppError> {
    #[cfg(test)]
    future_host::test_support::configure_entry_fixture();
    future_host::cli_tool::discover(capability)
}

pub(crate) fn prepare_path() -> Result<(), crate::AppError> {
    #[cfg(test)]
    future_host::test_support::configure_entry_fixture();
    future_host::cli_tool::prepare_path()
}

#[cfg(all(test, feature = "gui"))]
pub(crate) use future_host::cli_tool::EnvGuard;
