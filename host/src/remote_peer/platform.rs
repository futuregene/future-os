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
//! not `nats_ws_url`. That endpoint is **not** on every response the client
//! role receives: the platform's `ClaimPairResponse` carries only
//! `nats_ws_url` (the phone connects over WebSocket, so only that was ever
//! needed), while the TCP URL is only ever returned by `/auth/token`. A claim
//! without it is therefore not a failure — it is one refresh, immediately,
//! which is where the endpoint lives. Returning an error here instead would
//! reject every pairing the real platform can produce.

use super::creds::PeerCreds;
use super::link::Invitation;
use serde::Deserialize;
use serde_json::json;

pub(crate) use future_remote_client::host::{Claimed, Refreshed};

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

/// Take an invitation. One-shot by contract: the platform consumes the nonce,
/// so a failure here must surface (and send the user back to the host for a new
/// QR) rather than be retried — the same rule the phone follows.
pub(crate) async fn claim(
    invitation: &Invitation,
    device_id: &str,
    device_public_key: &str,
    device_name: &str,
) -> Result<Claimed, crate::Error> {
    let response = crate::remote_platform::http_client()?
        .post(&invitation.claim_url)
        .json(&json!({
            "nonce": invitation.nonce,
            "device_id": device_id,
            "device_public_key": device_public_key,
            "device_name": device_name,
        }))
        .send()
        .await
        .map_err(|error| crate::remote_platform::transport_or_message("claim pairing", error))?;
    let body: ClaimResponse =
        crate::remote_platform::parse_response(response, "claim pairing").await?;
    // Platform endpoints are cached on the credential so a later refresh needs
    // no re-derivation; they are also where the revoke URL comes from.
    let nats_url = optional_endpoint(body.nats_url);
    let nats_ws_url = body
        .nats_ws_url
        .filter(|url| !url.trim().is_empty())
        .or_else(|| nats_url.clone())
        .unwrap_or_default();
    let jwt_expires_at = crate::remote_platform::jwt_expiry(&body.user_jwt)?;
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
pub(crate) async fn refresh(creds: &PeerCreds) -> Result<Refreshed, crate::Error> {
    let key_pair = nkeys::KeyPair::from_seed(&creds.nkey_seed)
        .map_err(|error| crate::Error::Message(format!("Invalid stored device NKey: {error}")))?;
    let response = crate::remote_platform::http_client()?
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
            crate::remote_platform::transport_or_message("refresh peer credentials", error)
        })?;
    let body: RefreshResponse =
        crate::remote_platform::parse_response(response, "refresh peer credentials").await?;
    let nats_url = required_endpoint(body.nats_url, "nats_url")?;
    let nats_ws_url = body
        .nats_ws_url
        .filter(|url| !url.trim().is_empty())
        .unwrap_or_else(|| nats_url.clone());
    Ok(Refreshed {
        jwt_expires_at: crate::remote_platform::jwt_expiry(&body.user_jwt)?,
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
pub(crate) async fn revoke(creds: &PeerCreds) -> Result<(), crate::Error> {
    let key_pair = nkeys::KeyPair::from_seed(&creds.nkey_seed)
        .map_err(|error| crate::Error::Message(format!("Invalid stored device NKey: {error}")))?;
    let response = crate::remote_platform::http_client()?
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
            crate::remote_platform::transport_or_message("revoke peer pairing", error)
        })?;
    if response.status().is_success() || matches!(response.status().as_u16(), 401 | 404) {
        return Ok(());
    }
    let status = response.status();
    Err(crate::Error::Message(format!(
        "Failed to revoke peer pairing (HTTP {})",
        status.as_u16()
    )))
}

/// The client role needs a TCP endpoint. Absent is an error, not a cue to fall
/// back: silently using the WebSocket URL would connect over a transport whose
/// TLS settings this client does not verify the way it does for TCP.
///
/// Used by `refresh`, which is the endpoint that *does* carry `nats_url`. The
/// claim response is read with [`optional_endpoint`] instead.
fn required_endpoint(value: Option<String>, field: &str) -> Result<String, crate::Error> {
    optional_endpoint(value).ok_or_else(|| {
        crate::Error::Message(format!(
            "Remote server did not return a usable {field} for this client"
        ))
    })
}

/// A trimmed, non-empty endpoint, or `None`.
fn optional_endpoint(value: Option<String>) -> Option<String> {
    value
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::test_support::{jwt, now_secs, FakeNats, HomeGuard, MockPlatform};

    fn creds(token_url: &str, nkey_seed: &str) -> super::super::creds::PeerCreds {
        super::super::creds::PeerCreds {
            pair_id: "pair_1".into(),
            desktop_id: "desktop_1".into(),
            device_id: "dev_1".into(),
            nkey_seed: nkey_seed.into(),
            user_jwt: jwt(now_secs() + 3_600),
            refresh_token: "refresh".into(),
            nats_url: "nats://127.0.0.1:4222".into(),
            nats_ws_url: "wss://example.invalid".into(),
            jwt_expires_at: now_secs() + 3_600,
            token_url: token_url.into(),
            secure: None,
        }
    }

    /// A real NKey seed, so the signing path is exercised rather than short-
    /// circuited by an invalid one.
    fn valid_seed() -> String {
        nkeys::KeyPair::new_user().seed().expect("seed")
    }

    #[test]
    fn a_missing_tcp_endpoint_is_an_error_not_a_websocket_fallback() {
        assert!(required_endpoint(None, "nats_url").is_err());
        assert!(required_endpoint(Some("  ".into()), "nats_url").is_err());
        assert_eq!(
            required_endpoint(Some(" nats://host:4222 ".into()), "nats_url").unwrap(),
            "nats://host:4222"
        );
    }

    /// A stored seed that cannot be read is a *local* credential fault, and must
    /// be reported as one rather than sent to the platform as an empty key.
    #[tokio::test]
    async fn an_unreadable_seed_is_reported_before_any_request() {
        let _home = HomeGuard::new("peer-platform-bad-seed");
        let broken = creds(
            "http://127.0.0.1:1/client/v1/remote/auth/token",
            "not-a-seed",
        );
        let error = refresh(&broken)
            .await
            .expect_err("an invalid seed cannot sign");
        assert!(
            error.to_string().contains("Invalid stored device NKey"),
            "{error}"
        );
        let error = revoke(&broken)
            .await
            .expect_err("an invalid seed cannot sign");
        assert!(
            error.to_string().contains("Invalid stored device NKey"),
            "{error}"
        );
    }

    /// A platform that cannot be reached is a *transport* failure, so the UI can
    /// say "check your network" rather than showing a server error.
    #[tokio::test]
    async fn an_unreachable_platform_is_reported_as_a_transport_failure() {
        let _home = HomeGuard::new("peer-platform-unreachable");
        // Port 1 is reserved and never listening, so the send fails outright.
        let unreachable = creds(
            "http://127.0.0.1:1/client/v1/remote/auth/token",
            &valid_seed(),
        );
        assert!(matches!(
            refresh(&unreachable).await,
            Err(crate::Error::RemoteTransport(_))
        ));
        assert!(matches!(
            revoke(&unreachable).await,
            Err(crate::Error::RemoteTransport(_))
        ));
    }

    /// Claiming an invitation whose endpoint is gone is the same class of
    /// failure: no HTTP response was possible, so it is transport, not server.
    #[tokio::test]
    async fn an_unreachable_claim_endpoint_is_a_transport_failure() {
        let _home = HomeGuard::new("peer-platform-claim-unreachable");
        let invitation = super::super::link::Invitation {
            code: "code".into(),
            desktop_id: "desktop_1".into(),
            desktop_key: "UKEY".into(),
            secure_key: "key".into(),
            secret: "secret".into(),
            expires_at: None,
            claim_url: "http://127.0.0.1:1/client/v1/remote/pair/claim".into(),
            nonce: "nonce".into(),
        };
        let error = claim(&invitation, "dev_1", "UDEV", "Test")
            .await
            .expect_err("an unreachable claim endpoint cannot succeed");
        assert!(matches!(error, crate::Error::RemoteTransport(_)), "{error}");
    }

    /// A refresh the platform refuses is a *server* answer, not a transport
    /// failure — the distinction is what tells the UI to stop retrying.
    #[tokio::test]
    async fn a_refused_refresh_reports_the_platforms_own_error() {
        let _home = HomeGuard::new("peer-platform-refused");
        let platform = MockPlatform::start().await;
        platform.respond_refresh_revoked();
        let creds = creds(
            &format!("{}/client/v1/remote/auth/token", platform.url()),
            &valid_seed(),
        );
        let error = refresh(&creds).await.expect_err("a refused refresh fails");
        assert!(
            !matches!(error, crate::Error::RemoteTransport(_)),
            "{error}"
        );
    }

    /// A response with no TCP endpoint is an error rather than a silent
    /// downgrade to the WebSocket one, whose TLS this client does not verify the
    /// way it does for TCP. `refresh` is the endpoint that *does* carry
    /// `nats_url`, so its rule is the strict one.
    #[tokio::test]
    async fn a_refresh_without_a_tcp_endpoint_is_an_error() {
        let _home = HomeGuard::new("peer-platform-no-endpoint");
        let platform = MockPlatform::start().await;
        platform.push(
            "/client/v1/remote/auth/token",
            200,
            serde_json::json!({
                "user_jwt": jwt(now_secs() + 3_600),
                "nats_ws_url": "wss://example.invalid",
            }),
        );
        let creds = creds(
            &format!("{}/client/v1/remote/auth/token", platform.url()),
            &valid_seed(),
        );
        let error = refresh(&creds).await.expect_err("no nats_url is unusable");
        assert!(error.to_string().contains("nats_url"), "{error}");
    }

    /// The redaction is the whole point of the manual `Debug`: this value ends up
    /// in `expect`/`assert` output, and a bearer token in a panic message is a
    /// token in a bug report.
    #[test]
    fn debug_redacts_the_claim_secrets() {
        let claimed = Claimed {
            pair_id: "pair_1".into(),
            user_jwt: "jwt-secret".into(),
            refresh_token: "refresh-secret".into(),
            nats_url: Some("nats://host:4222".into()),
            nats_ws_url: "wss://host:4222".into(),
            jwt_expires_at: 42,
        };
        let rendered = format!("{claimed:?}");
        assert!(!rendered.contains("jwt-secret"), "{rendered}");
        assert!(!rendered.contains("refresh-secret"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
        // The non-secret fields stay, so a failure is still diagnosable.
        assert!(rendered.contains("pair_1"), "{rendered}");
        assert!(rendered.contains("nats://host:4222"), "{rendered}");
    }

    /// The claim endpoint is the one that matters most for this rule, and it is
    /// the one where the rule is *inverted*: the platform's `ClaimPairResponse`
    /// has no `nats_url` at all (see the server's `routes/remote.rs`), so a
    /// claim without it is the normal, successful case. Failing here rejects
    /// every pairing the real platform can produce — the client then never
    /// connects, however valid the invitation was.
    #[tokio::test]
    async fn a_claim_without_a_tcp_endpoint_is_not_an_error() {
        let _home = HomeGuard::new("peer-platform-claim-no-endpoint");
        let platform = MockPlatform::start().await;
        platform.push(
            "/client/v1/remote/pair/claim",
            200,
            serde_json::json!({
                "pair_id": "pair_1",
                "user_jwt": jwt(now_secs() + 3_600),
                "refresh_token": "refresh",
                "nats_ws_url": "wss://example.invalid",
            }),
        );
        let invitation = super::super::link::Invitation {
            code: "code".into(),
            desktop_id: "desktop_1".into(),
            desktop_key: "UKEY".into(),
            secure_key: "key".into(),
            secret: "secret".into(),
            expires_at: None,
            claim_url: format!("{}/client/v1/remote/pair/claim", platform.url()),
            nonce: "nonce".into(),
        };
        let claimed = claim(&invitation, "dev_1", "UDEV", "Test")
            .await
            .expect("a claim without nats_url is the normal case");
        assert!(claimed.nats_url.is_none(), "{claimed:?}");
        // The WebSocket URL is still recorded, so the field is populated rather
        // than lost: it is the only endpoint this response carries.
        assert_eq!(claimed.nats_ws_url, "wss://example.invalid");
    }

    /// A claim with no endpoint at all still yields the WebSocket field as an
    /// empty string rather than panicking on the missing TCP one.
    #[tokio::test]
    async fn a_claim_with_no_endpoints_at_all_still_yields_its_grant() {
        let _home = HomeGuard::new("peer-platform-claim-bare");
        let platform = MockPlatform::start().await;
        platform.push(
            "/client/v1/remote/pair/claim",
            200,
            serde_json::json!({
                "pair_id": "pair_1",
                "user_jwt": jwt(now_secs() + 3_600),
                "refresh_token": "refresh",
            }),
        );
        let invitation = super::super::link::Invitation {
            code: "code".into(),
            desktop_id: "desktop_1".into(),
            desktop_key: "UKEY".into(),
            secure_key: "key".into(),
            secret: "secret".into(),
            expires_at: None,
            claim_url: format!("{}/client/v1/remote/pair/claim", platform.url()),
            nonce: "nonce".into(),
        };
        let claimed = claim(&invitation, "dev_1", "UDEV", "Test")
            .await
            .expect("the grant itself is what makes a claim usable");
        assert!(claimed.nats_url.is_none(), "{claimed:?}");
        assert_eq!(claimed.nats_ws_url, "");
        assert_eq!(claimed.pair_id, "pair_1");
    }

    /// A claim the platform answers with an error body must surface that body's
    /// machine code, not a generic message.
    #[tokio::test]
    async fn a_refused_claim_reports_the_platforms_error_code() {
        let _home = HomeGuard::new("peer-platform-claim-refused");
        let platform = MockPlatform::start().await;
        platform.push(
            "/client/v1/remote/pair/claim",
            400,
            serde_json::json!({ "error": "invitation_consumed", "message": "already used" }),
        );
        let invitation = super::super::link::Invitation {
            code: "code".into(),
            desktop_id: "desktop_1".into(),
            desktop_key: "UKEY".into(),
            secure_key: "key".into(),
            secret: "secret".into(),
            expires_at: None,
            claim_url: format!("{}/client/v1/remote/pair/claim", platform.url()),
            nonce: "nonce".into(),
        };
        let error = claim(&invitation, "dev_1", "UDEV", "Test")
            .await
            .expect_err("a used invitation cannot be claimed");
        // Both ids must survive: the machine code is what a caller branches on,
        // and the human message is what the user reads. Asserted on the rendered
        // error so a non-`Remote` variant fails the assertion rather than
        // needing an arm that a passing test can never execute.
        let rendered = format!("{error:?}");
        assert!(rendered.contains("invitation_consumed"), "{rendered}");
        assert!(rendered.contains("already used"), "{rendered}");
    }

    /// A `nats_ws_url` the platform omits falls back to the TCP one rather than
    /// to an empty string: the field is stored for a UI that may show it.
    #[tokio::test]
    async fn a_missing_websocket_endpoint_falls_back_to_the_tcp_one() {
        let _home = HomeGuard::new("peer-platform-ws-fallback");
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        platform.push(
            "/client/v1/remote/auth/token",
            200,
            serde_json::json!({
                "user_jwt": jwt(now_secs() + 3_600),
                "nats_url": nats.url(),
            }),
        );
        let creds = creds(
            &format!("{}/client/v1/remote/auth/token", platform.url()),
            &valid_seed(),
        );
        let fresh = refresh(&creds).await.expect("refresh");
        assert_eq!(fresh.nats_url, nats.url());
        assert_eq!(fresh.nats_ws_url, nats.url());
    }

    /// A blank `nats_ws_url` is the same as an absent one.
    #[tokio::test]
    async fn a_blank_websocket_endpoint_falls_back_to_the_tcp_one() {
        let _home = HomeGuard::new("peer-platform-ws-blank");
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        platform.push(
            "/client/v1/remote/auth/token",
            200,
            serde_json::json!({
                "user_jwt": jwt(now_secs() + 3_600),
                "nats_url": nats.url(),
                "nats_ws_url": "   ",
            }),
        );
        let creds = creds(
            &format!("{}/client/v1/remote/auth/token", platform.url()),
            &valid_seed(),
        );
        let fresh = refresh(&creds).await.expect("refresh");
        assert_eq!(fresh.nats_ws_url, nats.url());
    }
}
