//! Client-side reading of a `futureos://remote/pair` invitation.
//!
//! The invitation is minted by the *host* desktop (`remote_host::pairing`) and
//! carried to this desktop by the user — scanned as a QR by a phone, pasted as
//! text here. It is a short-lived bearer credential: it contains the one-time
//! platform claim code plus the host's Noise static key and the invitation PSK.
//! Nothing in this module talks to the network, and nothing logs the invite.
//!
//! Format (v2), all in the query string:
//!   `v=2&code=…&desktopId=…&desktopKey=…&secureKey=…&secret=…`
//!
//! Only `v=2` is accepted. A v1 link carries no Noise material and cannot
//! authenticate anything, so it is rejected rather than silently downgraded —
//! the same rule the mobile client applies.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::Value;

/// A parsed, structurally valid v2 invitation. Field-level validity (lengths,
/// key shapes) is checked here; *authority* is not — the platform claim and the
/// Noise handshake are what actually decide whether it is still good.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Invitation {
    /// One-time platform claim code (base64url of a JSON payload).
    pub code: String,
    /// Host desktop identity, `desktop_…`.
    pub desktop_id: String,
    /// Host NATS user public key (`U…`) — the *bridge* key, not the Noise key.
    pub desktop_key: String,
    /// Host Noise X25519 static public key (base64url, 32 bytes).
    pub secure_key: String,
    /// Invitation pre-shared key (base64url, 32 bytes).
    pub secret: String,
    /// `exp` from the decoded claim code, in unix seconds, when present.
    pub expires_at: Option<i64>,
    /// Platform endpoint the claim must be sent to, read from the code. Kept
    /// here so nothing downstream re-decodes the code (and re-invents its
    /// validation) just to find it.
    pub claim_url: String,
    /// One-time claim nonce, likewise from the code.
    pub nonce: String,
}

/// Why an invitation could not be used. User-facing text is the UI's job; the
/// codes mirror the mobile client's `PA*` support codes so both platforms can
/// explain the same failure the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LinkError {
    /// Not a `futureos://remote/pair` URL at all.
    Malformed,
    /// A well-formed invitation, but not `v=2` (an old link + PSK-less pair).
    UnsupportedVersion,
    /// A required field is missing or does not have the shape the protocol needs.
    InvalidField,
    /// The claim code's own `exp` has passed.
    Expired,
}

impl LinkError {
    /// Stable support code (`PA*`), matching the mobile client's table.
    pub(crate) fn support_code(self) -> &'static str {
        match self {
            Self::UnsupportedVersion | Self::InvalidField => "PA002",
            Self::Expired => "PA002",
            Self::Malformed => "PA002",
        }
    }
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Malformed => "pairing_link_malformed",
            Self::UnsupportedVersion => "pairing_link_unsupported_version",
            Self::InvalidField => "pairing_link_invalid_field",
            Self::Expired => "pairing_link_expired",
        }
    }
}

/// The claim endpoint is where credentials are handed over, so it must be
/// HTTPS — with one deliberate exception: a loopback address. That is how the
/// repo's own local platform harness and a developer's same-machine platform
/// are reached, and a loopback hop cannot be intercepted off-host. Without it
/// the client could not be tested against the same harness the rest of the
/// remote stack uses, and the rule would be verified only in production.
fn is_acceptable_claim_url(url: &reqwest::Url) -> bool {
    match url.scheme() {
        "https" => true,
        "http" => url.host_str().is_some_and(|host| {
            matches!(
                host.trim_start_matches('[').trim_end_matches(']'),
                "127.0.0.1" | "::1" | "localhost"
            )
        }),
        _ => false,
    }
}

/// A pairing code is a JSON object in base64url. `exp` is unix seconds.
fn decode_code(code: &str) -> Option<Value> {
    let bytes = URL_SAFE_NO_PAD.decode(code).ok()?;
    serde_json::from_slice::<Value>(&bytes).ok()
}

/// Structural check for a base64url-encoded 32-byte key. Both the Noise key and
/// the PSK are raw 25519-sized values; a short or padded one would fail later,
/// inside the handshake, where the error is harder to attribute.
fn is_key32(value: &str) -> bool {
    URL_SAFE_NO_PAD
        .decode(value)
        .is_ok_and(|bytes| bytes.len() == 32 && URL_SAFE_NO_PAD.encode(&bytes) == value)
}

/// `now` is passed in so the expiry rule is testable without a clock seam.
pub(crate) fn parse_invitation(value: &str, now: i64) -> Result<Invitation, LinkError> {
    let url = reqwest::Url::parse(value.trim()).map_err(|_| LinkError::Malformed)?;
    if url.scheme() != "futureos" || url.host_str() != Some("remote") || url.path() != "/pair" {
        return Err(LinkError::Malformed);
    }
    let field = |name: &str| -> Option<String> {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
            .filter(|value| !value.is_empty())
    };
    // Reject v1 explicitly instead of treating it as a missing field: the link
    // is well-formed and the user would otherwise see a generic error.
    match field("v").as_deref() {
        Some("2") => {}
        _ => return Err(LinkError::UnsupportedVersion),
    }
    let code = field("code").ok_or(LinkError::InvalidField)?;
    let desktop_id = field("desktopId").ok_or(LinkError::InvalidField)?;
    let desktop_key = field("desktopKey").ok_or(LinkError::InvalidField)?;
    let secure_key = field("secureKey").ok_or(LinkError::InvalidField)?;
    let secret = field("secret").ok_or(LinkError::InvalidField)?;
    if !desktop_id.starts_with("desktop_") || !is_key32(&secure_key) || !is_key32(&secret) {
        return Err(LinkError::InvalidField);
    }
    let payload = decode_code(&code).ok_or(LinkError::InvalidField)?;
    let nonce = payload
        .get("nonce")
        .and_then(Value::as_str)
        .filter(|nonce| !nonce.is_empty())
        .ok_or(LinkError::InvalidField)?
        .to_string();
    // The claim URL is the platform endpoint the code was minted for. Trusting
    // it is what lets a desktop pair against a self-hosted platform, so it must
    // be present and HTTPS — the same check the mobile client makes before it
    // sends anything to it. A `claim_url` that is missing or downgraded means
    // the link is not usable, not that we should guess a default.
    let claim_url = payload
        .get("claim_url")
        .and_then(Value::as_str)
        .ok_or(LinkError::InvalidField)?;
    let claim = reqwest::Url::parse(claim_url).map_err(|_| LinkError::InvalidField)?;
    if !is_acceptable_claim_url(&claim) {
        return Err(LinkError::InvalidField);
    }
    let expires_at = payload.get("exp").and_then(Value::as_i64);
    if expires_at.is_some_and(|exp| exp <= now) {
        return Err(LinkError::Expired);
    }
    Ok(Invitation {
        code,
        desktop_id,
        desktop_key,
        secure_key,
        secret,
        expires_at,
        claim_url: claim_url.to_string(),
        nonce,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn invitation(overrides: &[(&str, &str)]) -> String {
        let code = URL_SAFE_NO_PAD.encode(
            json!({ "nonce": "n-1", "claim_url": "https://future-os.cn/client/v1/remote/pair/claim", "exp": 2_000 })
                .to_string(),
        );
        let key = URL_SAFE_NO_PAD.encode([7u8; 32]);
        let mut fields = vec![
            ("v", "2".to_string()),
            ("code", code),
            ("desktopId", "desktop_host".to_string()),
            ("desktopKey", "UDESKTOP".to_string()),
            ("secureKey", key.clone()),
            ("secret", key),
        ];
        for (name, value) in overrides {
            fields.retain(|(key, _)| key != name);
            fields.push((name, (*value).to_string()));
        }
        let mut url = reqwest::Url::parse("futureos://remote/pair").unwrap();
        for (name, value) in fields {
            url.query_pairs_mut().append_pair(name, &value);
        }
        url.into()
    }

    #[test]
    fn parses_a_well_formed_v2_invitation() {
        let parsed = parse_invitation(&invitation(&[]), 1_000).expect("valid link");
        assert_eq!(parsed.desktop_id, "desktop_host");
        assert_eq!(parsed.expires_at, Some(2_000));
        assert_eq!(parsed.nonce, "n-1");
        assert_eq!(
            parsed.claim_url,
            "https://future-os.cn/client/v1/remote/pair/claim"
        );
    }

    /// A v1 link is a *different protocol*, not a broken v2 one. Reporting it as
    /// "unsupported version" is what lets the UI tell the user to re-pair
    /// instead of asking them to check their copy/paste.
    #[test]
    fn rejects_a_v1_invitation_as_unsupported_rather_than_malformed() {
        let v1 = invitation(&[("v", "1")]);
        assert_eq!(
            parse_invitation(&v1, 1_000),
            Err(LinkError::UnsupportedVersion)
        );
        let missing = invitation(&[("v", "")]);
        assert_eq!(
            parse_invitation(&missing, 1_000),
            Err(LinkError::UnsupportedVersion)
        );
    }

    #[test]
    fn rejects_other_urls_and_extra_fragment_forms() {
        assert_eq!(
            parse_invitation("https://future-os.cn/", 1_000),
            Err(LinkError::Malformed)
        );
        assert_eq!(
            parse_invitation("futureos://remote/elsewhere?v=2", 1_000),
            Err(LinkError::Malformed)
        );
        assert_eq!(
            parse_invitation("not a url", 1_000),
            Err(LinkError::Malformed)
        );
    }

    /// Passing the shape checks early keeps a mistyped link from reaching the
    /// handshake, where the same defect surfaces as an opaque protocol failure
    /// *after* the user has been told the connection is being established.
    #[test]
    fn rejects_structurally_invalid_fields() {
        assert_eq!(
            parse_invitation(&invitation(&[("desktopId", "host")]), 1_000),
            Err(LinkError::InvalidField)
        );
        assert_eq!(
            parse_invitation(&invitation(&[("secureKey", "short")]), 1_000),
            Err(LinkError::InvalidField)
        );
        // A padded / non-canonical encoding of the right length is still wrong.
        let padded = format!("{}=", URL_SAFE_NO_PAD.encode([7u8; 32]));
        assert_eq!(
            parse_invitation(&invitation(&[("secret", &padded)]), 1_000),
            Err(LinkError::InvalidField)
        );
        assert_eq!(
            parse_invitation(&invitation(&[("secret", "")]), 1_000),
            Err(LinkError::InvalidField)
        );
    }

    /// The claim URL is where credentials are handed over, so a link that makes
    /// us send them in the clear is refused — as a bad link, not by falling
    /// back to the built-in platform.
    #[test]
    fn refuses_a_claim_url_that_is_missing_or_not_https() {
        for bad in [
            json!({ "nonce": "n-1", "exp": 2_000 }),
            json!({ "nonce": "n-1", "claim_url": "http://future-os.cn/client/v1/remote/pair/claim" }),
            json!({ "nonce": "n-1", "claim_url": "not a url" }),
            // A blank nonce is not a nonce; the claim would 400 on it.
            json!({ "nonce": "", "claim_url": "https://future-os.cn/client/v1/remote/pair/claim" }),
        ] {
            let code = URL_SAFE_NO_PAD.encode(bad.to_string());
            assert_eq!(
                parse_invitation(&invitation(&[("code", &code)]), 1_000),
                Err(LinkError::InvalidField),
                "{bad}"
            );
        }
    }

    /// Plaintext is allowed *only* to loopback (the local platform harness, and
    /// a same-machine self-hosted platform). Everything else must be HTTPS, so
    /// a remote host cannot make a client hand its credentials to a plaintext
    /// endpoint just by minting a link that says so.
    #[test]
    fn plaintext_claims_are_allowed_only_to_loopback() {
        let with_claim = |claim: &str| {
            let code =
                URL_SAFE_NO_PAD.encode(json!({ "nonce": "n-1", "claim_url": claim }).to_string());
            parse_invitation(&invitation(&[("code", &code)]), 1_000)
        };
        for allowed in [
            "http://127.0.0.1:8022/client/v1/remote/pair/claim",
            "http://localhost:8022/client/v1/remote/pair/claim",
            "http://[::1]:8022/client/v1/remote/pair/claim",
            "https://future-os.cn/client/v1/remote/pair/claim",
        ] {
            assert!(with_claim(allowed).is_ok(), "{allowed} should be usable");
        }
        for refused in [
            "http://future-os.cn/client/v1/remote/pair/claim",
            "http://192.168.1.10:8022/client/v1/remote/pair/claim",
            "http://127.0.0.1.example.com/client/v1/remote/pair/claim",
        ] {
            assert_eq!(
                with_claim(refused),
                Err(LinkError::InvalidField),
                "{refused} must not receive credentials in the clear"
            );
        }
    }

    #[test]
    fn rejects_an_expired_code_but_accepts_one_without_an_expiry() {
        let expired = invitation(&[]);
        assert_eq!(
            parse_invitation(&expired, 2_000),
            Err(LinkError::Expired),
            "exp is inclusive-upper: exp == now is already gone"
        );
        assert!(parse_invitation(&expired, 1_999).is_ok());

        let no_exp = URL_SAFE_NO_PAD.encode(
            json!({ "nonce": "n", "claim_url": "https://future-os.cn/client/v1/remote/pair/claim" })
                .to_string(),
        );
        assert!(parse_invitation(&invitation(&[("code", &no_exp)]), 9_999).is_ok());
    }
}
