//! Platform transaction adapters; preparation never stops application services.
use super::{journal, process::Completion};
#[cfg(target_os = "windows")]
pub(super) type Prepared = super::windows::PreparedObserver;
#[cfg(target_os = "macos")]
pub(super) type Prepared = super::macos::Prepared;
#[cfg(target_os = "linux")]
pub(super) type Prepared = super::linux::Prepared;

pub(super) fn prepare(
    bytes: &[u8],
    signature: &str,
    operation: &journal::Operation,
) -> Result<Prepared, String> {
    #[cfg(not(target_os = "linux"))]
    let _ = signature;
    #[cfg(target_os = "windows")]
    {
        super::windows::PreparedInstaller::new(bytes)
            .map_err(|e| e.to_string())?
            .prepare(operation)
    }
    #[cfg(target_os = "macos")]
    {
        Prepared::new(bytes, operation)
    }
    #[cfg(target_os = "linux")]
    {
        Prepared::new(bytes, signature, operation)
    }
}
pub(super) fn commit(
    prepared: Prepared,
    operation: &journal::Operation,
) -> Result<Completion, String> {
    #[cfg(target_os = "windows")]
    {
        prepared.commit(operation).map(|()| Completion::success())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = operation;
        prepared.commit()
    }
}
pub(super) fn validate_startup(id: &str) -> Result<(), crate::AppError> {
    #[cfg(target_os = "linux")]
    super::linux::confirm(id).map_err(crate::AppError::from)?;
    #[cfg(not(target_os = "linux"))]
    let _ = id;
    Ok(())
}
pub(super) fn cleanup_confirmed(id: &str) -> Result<(), crate::AppError> {
    #[cfg(target_os = "macos")]
    super::macos::confirm(id).map_err(crate::AppError::from)?;
    #[cfg(target_os = "windows")]
    {
        let backup = future_rpc::home::future_home()
            .join("app/updates")
            .join(id)
            .join("program-backup");
        if backup.exists() {
            std::fs::remove_dir_all(backup)?;
        }
    }
    #[cfg(target_os = "linux")]
    let _ = id;
    Ok(())
}
