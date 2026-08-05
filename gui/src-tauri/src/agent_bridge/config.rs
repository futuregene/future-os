//! RPC-first provider/auth config writes (audit item 2).
//!
//! Instead of editing `~/.future/agent/auth.json` / `models.json` directly and
//! then patching the agent's in-memory state with `reload_auth`, the GUI asks
//! the agent to write its own config via `set_auth` / `upsert_provider` /
//! `delete_provider`. The agent applies the change and refreshes its registry
//! and live-session credentials internally, so no follow-up round-trip is
//! needed on this path.
//!
//! Fallback keeps version skew and offline edits working: an unreachable
//! agent, or one that predates these commands (it answers `success = false`
//! with "unknown command"), sends the caller back to the GUI's own file
//! writers plus the best-effort `reload_auth` follow-up — exactly the legacy
//! behavior. An explicit rejection from an agent that DOES know the command
//! (validation failure, e.g. a duplicate provider id) is surfaced as an error
//! instead, never silently re-applied locally.

use crate::agent_proto::{AuthUpdate, RpcCommand};
use crate::AppError;

use super::client::{base_command, connect_agent};

/// Send a config-write command to the agent.
///
/// Returns `Ok(true)` when the agent applied the change (its live state is
/// already refreshed), `Ok(false)` when the caller must fall back to the local
/// file writers (agent unreachable or too old), and `Err` when the agent
/// explicitly rejected the change.
async fn send_config_write(command: RpcCommand, context: &str) -> Result<bool, AppError> {
    let mut client = match connect_agent().await {
        Ok(client) => client,
        Err(error) => {
            eprintln!(
                "FutureOS: agent unavailable for {context} ({error}); falling back to the local config write"
            );
            return Ok(false);
        }
    };
    let response = match client.execute_command(command).await {
        Ok(response) => response.into_inner(),
        Err(status) => {
            eprintln!(
                "FutureOS: {context} failed at transport level ({status}); falling back to the local config write"
            );
            return Ok(false);
        }
    };
    if response.success {
        return Ok(true);
    }
    // A pre-item-2 agent answers success=false "unknown command: <type>".
    // Treat it like an unavailable agent and let the caller fall back.
    if response.error.contains("unknown command") {
        eprintln!(
            "FutureOS: agent does not support {context}; falling back to the local config write"
        );
        return Ok(false);
    }
    let message = if response.error.is_empty() {
        format!("Future Agent rejected {context}.")
    } else {
        response.error
    };
    Err(message.into())
}

fn set_auth_command(update: AuthUpdate) -> RpcCommand {
    RpcCommand {
        auth_update: Some(update),
        ..base_command("set_auth", String::new())
    }
}

/// Set a provider's API key (`auth.json` entry, preserving other fields).
pub(crate) async fn set_provider_key(id: &str, key: &str) -> Result<bool, AppError> {
    send_config_write(
        set_auth_command(AuthUpdate {
            provider: id.to_string(),
            key: key.to_string(),
            ..Default::default()
        }),
        "set_auth",
    )
    .await
}

/// Remove a provider's API key, keeping the rest of its entry.
pub(crate) async fn clear_provider_key(id: &str) -> Result<bool, AppError> {
    send_config_write(
        set_auth_command(AuthUpdate {
            provider: id.to_string(),
            clear_key: true,
            ..Default::default()
        }),
        "set_auth",
    )
    .await
}

/// FutureGene login: store the device-flow key and pin `base_url` (mirrors the
/// CLI's `saveAuth` shape, applied by the agent).
pub(crate) async fn future_login(key: &str, base_url: &str) -> Result<bool, AppError> {
    send_config_write(
        set_auth_command(AuthUpdate {
            provider: crate::auth_store::FUTURE_PROVIDER_ID.to_string(),
            key: key.to_string(),
            base_url: base_url.to_string(),
            ..Default::default()
        }),
        "set_auth",
    )
    .await
}

/// FutureGene logout: drop the key, keep `base_url`.
pub(crate) async fn future_logout() -> Result<bool, AppError> {
    send_config_write(
        set_auth_command(AuthUpdate {
            provider: crate::auth_store::FUTURE_PROVIDER_ID.to_string(),
            clear_key: true,
            ..Default::default()
        }),
        "set_auth",
    )
    .await
}

/// Set (or clear, when `base_url` is empty) a built-in provider's Base URL
/// override in `models.json`.
pub(crate) async fn set_builtin_provider_base_url(
    id: &str,
    base_url: &str,
) -> Result<bool, AppError> {
    send_config_write(
        RpcCommand {
            provider_config: Some(crate::agent_proto::ProviderUpsert {
                id: id.to_string(),
                base_url: base_url.to_string(),
                clear_base_url: base_url.is_empty(),
                ..Default::default()
            }),
            ..base_command("upsert_provider", String::new())
        },
        "upsert_provider",
    )
    .await
}

/// Create/update a `models.json` provider entry (plus optional auth key).
pub(crate) async fn upsert_provider(
    config: crate::agent_proto::ProviderUpsert,
) -> Result<bool, AppError> {
    send_config_write(
        RpcCommand {
            provider_config: Some(config),
            ..base_command("upsert_provider", String::new())
        },
        "upsert_provider",
    )
    .await
}

/// Remove a provider's `models.json` entry AND its `auth.json` entry.
pub(crate) async fn delete_provider(id: &str) -> Result<bool, AppError> {
    send_config_write(
        RpcCommand {
            provider_config: Some(crate::agent_proto::ProviderUpsert {
                id: id.to_string(),
                ..Default::default()
            }),
            ..base_command("delete_provider", String::new())
        },
        "delete_provider",
    )
    .await
}
