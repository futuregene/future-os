//! Stable identity of this Desktop installation.
//!
//! `device_id` is intentionally broader than remote pairing: Agent session
//! provenance, remote control, and future device-scoped features all refer to
//! the same installation identity. It survives app-data reset and pairing
//! revocation. A future `client_id` should identify one process/connection and
//! must not replace this durable value.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

static DEVICE_IDS: LazyLock<Mutex<HashMap<PathBuf, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Return the durable Desktop installation id, creating it exactly once.
///
/// Before this component existed the id lived only inside
/// `remote_pairing.json`. Seed from that legacy location when present so an
/// upgraded, already-paired installation keeps its platform identity.
pub fn device_id() -> Result<String, crate::AppError> {
    let identity_root = PathBuf::from(crate::home_dir().ok_or_else(|| {
        crate::AppError::Message("HOME/USERPROFILE environment variable is not set.".to_string())
    })?)
    .join(".future");
    if let Some(device_id) = DEVICE_IDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&identity_root)
        .cloned()
    {
        return Ok(device_id);
    }
    let legacy_id = legacy_pairing_device_id();
    let candidate = legacy_id.unwrap_or_else(new_device_id);
    let device_id = crate::store::get_or_create_device_id(&candidate)?;
    DEVICE_IDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(identity_root, device_id.clone());
    Ok(device_id)
}

/// Best-effort identity for non-critical lifecycle metadata.
///
/// Creating or forking an Agent session must not fail merely because the local
/// identity store is momentarily locked. An empty creator id activates the
/// conservative legacy `createdBy == desktop` filter; later reconciliation
/// converges through the unique agent-session binding.
pub fn device_id_or_empty() -> String {
    device_id().unwrap_or_default()
}

fn legacy_pairing_device_id() -> Option<String> {
    let home = crate::home_dir()?;
    let path = PathBuf::from(home)
        .join(".future")
        .join("remote_pairing.json");
    crate::config_io::read_json_object(&path)
        .ok()?
        .get("desktopId")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

fn new_device_id() -> String {
    let key = nkeys::KeyPair::new_user().public_key();
    format!("desktop_{}", &key[1..17])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_id_is_stable_and_keeps_the_desktop_namespace() {
        let _home = crate::store::test_schema_home("device_identity_stable");
        let first = device_id().expect("first id");
        let second = device_id().expect("second id");
        assert_eq!(first, second);
        assert!(first.starts_with("desktop_"));
    }

    #[test]
    fn legacy_pairing_id_seeds_the_common_identity() {
        let _home = crate::store::test_schema_home("device_identity_legacy");
        let home_path = std::env::var("HOME").expect("test HOME");
        let path = std::path::Path::new(&home_path)
            .join(".future")
            .join("remote_pairing.json");
        crate::config_io::write_json_atomic(
            &path,
            &serde_json::json!({"desktopId": "desktop_legacy"}),
            true,
        )
        .expect("legacy creds");

        assert_eq!(device_id().expect("migrated id"), "desktop_legacy");
    }
}
