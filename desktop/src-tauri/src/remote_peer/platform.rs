//! Platform control plane for the client role: claim an invitation, renew the
//! NATS user JWT, and revoke the pair.
//!
//! Same endpoints as the phone and the host (`/client/v1/remote/…`); the only
//! differences are the ones the server actually keys on:
//!
//! * `role` on the token request is `client` (the host sends `bridge`). The
//!   platform signs role-scoped NATS subjects, so sending the wrong one gets a
//!   JWT that cannot subscribe to its own command subject.
//! * `device_name` is what the host desktop shows for this pairing. A phone
//!   sends its model; a desktop sends its hostname, so the host's UI can say
//!   *which* machine took the slot instead of showing an opaque id.
//!
//! The desktop connects over TCP with verified TLS, so it needs `nats_url`,
//! not `nats_ws_url`. The platform returns both to every client, but a
//! response without `nats_url` is an error rather than a silent downgrade to
//! the WebSocket endpoint.

use super::creds::PeerCreds;
use super::link::Invitation;
use serde::Deserialize;
use serde_json::json;

/// What a successful claim produced, before it is split into this installation's
/// identity (generated locally, never sent) and the server's grant.
pub(crate) struct Claimed {
    pub pair_id: String,
    pub user_jwt: String,
    pub refresh_token: String,
    pub nats_url: String,
    pub nats_ws_url: String,
    pub jwt_expires_at: i64,
}

#[derive(Debug, Deserialize)]
struct ClaimResponse {
    pair_id: String,
    user_jwt: String,
    refresh_token: String,
    nats_url: Option<String>,
    nats_ws_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RefreshResponse {
    user_jwt: String,
    nats_url: Option<String>,
    nats_ws_url: Option<String>,
}

/// A refreshed grant, ready to be written back onto the stored credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Refreshed {
    pub user_jwt: String,
    pub nats_url: String,
    pub nats_ws_url: String,
    pub jwt_expires_at: i64,
}

/// Take an invitation. One-shot by contract: the platform consumes the nonce,
/// so a failure here must surface (and send the user back to the host for a new
/// QR) rather than be retried — the same rule the phone follows.
pub(crate) async fn claim(
    invitation: &Invitation,
    device_id: &str,
    device_public_key: &str,
    device_name: &str,
) -> Result<Claimed, crate::AppError> {
    let response = crate::remote_host::pairing::http_client()?
        .post(&invitation.claim_url)
        .json(&json!({
            "nonce": invitation.nonce,
            "device_id": device_id,
            "device_public_key": device_public_key,
            "device_name": device_name,
        }))
        .send()
        .await
        .map_err(|error| {
            crate::remote_host::pairing::transport_or_message("claim pairing", error)
        })?;
    let body: ClaimResponse =
        crate::remote_host::pairing::parse_response(response, "claim pairing").await?;
    // Platform endpoints are cached on the credential so a later refresh needs
    // no re-derivation; they are also where the revoke URL comes from.
    let nats_url = required_endpoint(body.nats_url, "nats_url")?;
    let nats_ws_url = body
        .nats_ws_url
        .filter(|url| !url.trim().is_empty())
        .unwrap_or_else(|| nats_url.clone());
    let jwt_expires_at = crate::remote_host::pairing::jwt_expiry(&body.user_jwt)?;
    Ok(Claimed {
        pair_id: body.pair_id,
        user_jwt: body.user_jwt,
        refresh_token: body.refresh_token,
        nats_url,
        nats_ws_url,
        jwt_expires_at,
    })
}

/// Renew the short-lived NATS JWT. The refresh token is the long-lived secret;
/// the device NKey proves possession, so a stolen refresh token alone cannot be
/// redeemed without the seed.
pub(crate) async fn refresh(creds: &PeerCreds) -> Result<Refreshed, crate::AppError> {
    let key_pair = nkeys::KeyPair::from_seed(&creds.nkey_seed).map_err(|error| {
        crate::AppError::Message(format!("Invalid stored device NKey: {error}"))
    })?;
    let response = crate::remote_host::pairing::http_client()?
        .post(&creds.token_url)
        .json(&json!({
            "pair_id": creds.pair_id,
            "device_id": creds.device_id,
            "public_key": key_pair.public_key(),
            "role": "client",
            "refresh_token": creds.refresh_token,
        }))
        .send()
        .await
        .map_err(|error| {
            crate::remote_host::pairing::transport_or_message("refresh peer credentials", error)
        })?;
    let body: RefreshResponse =
        crate::remote_host::pairing::parse_response(response, "refresh peer credentials").await?;
    let nats_url = required_endpoint(body.nats_url, "nats_url")?;
    let nats_ws_url = body
        .nats_ws_url
        .filter(|url| !url.trim().is_empty())
        .unwrap_or_else(|| nats_url.clone());
    Ok(Refreshed {
        jwt_expires_at: crate::remote_host::pairing::jwt_expiry(&body.user_jwt)?,
        user_jwt: body.user_jwt,
        nats_url,
        nats_ws_url,
    })
}

/// Server-side revocation, best-effort.
///
/// Local removal must never wait on the network: a user who unpairs while
/// offline still stops connecting now, and the platform cleans up when it can.
/// `401`/`404` are terminal ("already gone"), so a queued retry drains instead
/// of looping — the same contract the phone implements.
pub(crate) async fn revoke(creds: &PeerCreds) -> Result<(), crate::AppError> {
    let key_pair = nkeys::KeyPair::from_seed(&creds.nkey_seed).map_err(|error| {
        crate::AppError::Message(format!("Invalid stored device NKey: {error}"))
    })?;
    let response = crate::remote_host::pairing::http_client()?
        .post(creds.revoke_url())
        .json(&json!({
            "pair_id": creds.pair_id,
            "device_id": creds.device_id,
            "public_key": key_pair.public_key(),
            "refresh_token": creds.refresh_token,
        }))
        .send()
        .await
        .map_err(|error| {
            crate::remote_host::pairing::transport_or_message("revoke peer pairing", error)
        })?;
    if response.status().is_success() || matches!(response.status().as_u16(), 401 | 404) {
        return Ok(());
    }
    let status = response.status();
    Err(crate::AppError::Message(format!(
        "Failed to revoke peer pairing (HTTP {})",
        status.as_u16()
    )))
}

/// The client role needs a TCP endpoint. Absent is an error, not a cue to fall
/// back: silently using the WebSocket URL would connect over a transport whose
/// TLS settings this client does not verify the way it does for TCP.
fn required_endpoint(value: Option<String>, field: &str) -> Result<String, crate::AppError> {
    value
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
        .ok_or_else(|| {
            crate::AppError::Message(format!(
                "Remote server did not return a usable {field} for this client"
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_tcp_endpoint_is_an_error_not_a_websocket_fallback() {
        assert!(required_endpoint(None, "nats_url").is_err());
        assert!(required_endpoint(Some("  ".into()), "nats_url").is_err());
        assert_eq!(
            required_endpoint(Some(" nats://host:4222 ".into()), "nats_url").unwrap(),
            "nats://host:4222"
        );
    }
}
