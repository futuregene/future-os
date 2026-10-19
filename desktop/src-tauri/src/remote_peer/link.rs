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

/// The platform endpoints to use for a claimed pairing, derived from the claim
/// URL so a self-hosted platform works without another configuration knob.
///
/// Only reachable for a URL the platform actually offered; the scheme guard
/// below exists so a non-HTTP scheme (a `data:` URL, or a typo like `htps:`) is
/// refused rather than handed to `reqwest`, which would reject it later with a
/// message that names no cause.
fn is_acceptable_claim_url(url: &reqwest::Url, relay: Option<&reqwest::Url>) -> bool {
    match url.scheme() {
        "https" => true,
        // A self-hosted relay on a local network has no certificate either side
        // can verify, so a v3 invitation may declare a plain-HTTP origin — the
        // mobile client makes exactly the same allowance, for the same reason.
        // The claim then has to go to the origin the invitation declared,
        // scheme included, so an `https` declaration is never downgraded; a
        // bare loopback URL stays usable on its own for a local relay.
        "http" => match relay {
            Some(declared) => url.origin() == declared.origin(),
            None => is_loopback_http(url),
        },
        _ => false,
    }
}

/// A plaintext claim to this machine, which needs no relay declaration: the
/// local test relay and the browser surface both live here.
fn is_loopback_http(url: &reqwest::Url) -> bool {
    url.host_str().is_some_and(|host| {
        matches!(
            host.trim_start_matches('[').trim_end_matches(']'),
            "127.0.0.1" | "::1" | "localhost"
        )
    })
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
    //
    // v3 is the same invitation shape minted by a self-hosted relay: the Noise
    // material and the one-time secret are unchanged, only the claim endpoint
    // (which the invitation already carries) is not the Future platform's.
    match field("v").as_deref() {
        Some("2") | Some("3") => {}
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
    // A v3 invitation also declares the relay it belongs to; that origin — and
    // only that origin — is what lets a plaintext claim through. A payload
    // with no declaration keeps the old rule.
    let relay = payload
        .get("relay")
        .and_then(Value::as_str)
        .and_then(|value| reqwest::Url::parse(value).ok());
    if !is_acceptable_claim_url(&claim, relay.as_ref()) {
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
    /// a same-machine self-hosted platform), or to the plain-HTTP relay the
    /// invitation itself declares. Everything else must be HTTPS, so a remote
    /// host cannot make a client hand its credentials to a plaintext endpoint
    /// just by minting a link that says so.
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

    /// A v3 invitation minted by a relay on the local network names that relay
    /// inside the code, and that declaration is the *only* thing that makes its
    /// plaintext claim endpoint usable.
    #[test]
    fn a_v3_invitation_may_name_its_plaintext_relay() {
        let claim = "http://192.168.1.10:8099/client/v1/remote/pair/claim";
        let code_for = |relay: &str| {
            URL_SAFE_NO_PAD.encode(
                json!({ "nonce": "n-1", "claim_url": claim, "relay": relay, "exp": 2_000 })
                    .to_string(),
            )
        };

        let code = code_for("http://192.168.1.10:8099");
        let parsed = parse_invitation(&invitation(&[("v", "3"), ("code", &code)]), 1_000)
            .expect("a LAN relay's invitation is usable");
        assert_eq!(parsed.claim_url, claim);

        // Declaring a different relay does not launder the same endpoint.
        let elsewhere = code_for("http://192.168.1.99:8099");
        assert_eq!(
            parse_invitation(&invitation(&[("v", "3"), ("code", &elsewhere)]), 1_000),
            Err(LinkError::InvalidField)
        );
    }

    /// A non-HTTP scheme is refused as a bad *link* rather than handed to
    /// `reqwest`, which would fail later with a message that names no cause.
    #[test]
    fn a_claim_url_with_a_non_http_scheme_is_refused() {
        let with_claim = |claim: &str| {
            let code =
                URL_SAFE_NO_PAD.encode(json!({ "nonce": "n-1", "claim_url": claim }).to_string());
            parse_invitation(&invitation(&[("code", &code)]), 1_000)
        };
        for refused in [
            "ftp://future-os.cn/client/v1/remote/pair/claim",
            "data:text/plain,hi",
            "//future-os.cn/client/v1/remote/pair/claim",
        ] {
            assert_eq!(
                with_claim(refused),
                Err(LinkError::InvalidField),
                "{refused} is not a claim endpoint"
            );
        }
    }

    /// The scheme decision, asserted directly: the link-level tests above cover
    /// the wiring, and this covers every arm of the decision itself — including
    /// the non-HTTP one, which a link can only reach with a scheme `reqwest`
    /// would otherwise reject later with an unhelpful message.
    #[test]
    fn only_https_or_loopback_http_is_an_acceptable_claim_endpoint() {
        let url = |value: &str| reqwest::Url::parse(value).expect("test url");
        assert!(is_acceptable_claim_url(
            &url("https://future-os.cn/x"),
            None
        ));
        assert!(is_acceptable_claim_url(&url("http://127.0.0.1:1/x"), None));
        assert!(is_acceptable_claim_url(&url("http://localhost/x"), None));
        assert!(is_acceptable_claim_url(&url("http://[::1]:1/x"), None));
        assert!(!is_acceptable_claim_url(&url("http://example.com/x"), None));
        assert!(!is_acceptable_claim_url(&url("ftp://example.com/x"), None));
        assert!(!is_acceptable_claim_url(&url("data:text/plain,hi"), None));
        // A `file:` URL parses with no host at all, so the `http` arm's
        // `host_str()` is `None` there; the `https` guard is what keeps this
        // from being a file read.
        assert!(!is_acceptable_claim_url(&url("file:///etc/passwd"), None));
    }

    /// A relay on a LAN is plain HTTP (neither side can verify a certificate
    /// there), and the invitation is what authenticates it. The claim may go
    /// to that declared origin and nowhere else.
    #[test]
    fn a_declared_plaintext_relay_is_claimed_at_its_own_origin_only() {
        let url = |value: &str| reqwest::Url::parse(value).expect("test url");
        let relay = url("http://192.168.1.10:8099");
        assert!(is_acceptable_claim_url(
            &url("http://192.168.1.10:8099/client/v1/remote/pair/claim"),
            Some(&relay)
        ));
        for refused in [
            // Another host, or another port.
            "http://192.168.1.11:8099/client/v1/remote/pair/claim",
            "http://192.168.1.10:8098/client/v1/remote/pair/claim",
            "http://evil.example/client/v1/remote/pair/claim",
        ] {
            assert!(
                !is_acceptable_claim_url(&url(refused), Some(&relay)),
                "{refused} must not be claimed"
            );
        }
        // Declaring an https relay does not buy a plaintext claim either.
        let secure = url("https://relay.example");
        assert!(!is_acceptable_claim_url(
            &url("http://relay.example/client/v1/remote/pair/claim"),
            Some(&secure)
        ));
    }

    /// The public shape of a link failure: every variant carries the code and
    /// the message the UI shows, asserted directly so a new variant cannot be
    /// added without deciding both.
    #[test]
    fn every_link_failure_carries_the_shared_support_code_and_message() {
        let cases = [
            (LinkError::Malformed, "pairing_link_malformed"),
            (
                LinkError::UnsupportedVersion,
                "pairing_link_unsupported_version",
            ),
            (LinkError::InvalidField, "pairing_link_invalid_field"),
            (LinkError::Expired, "pairing_link_expired"),
        ];
        for (error, message) in cases {
            // Every link failure is `PA002` — the phone's "this code did not
            // work, make a new one" — so a support conversation means the same
            // thing on either platform.
            assert_eq!(error.support_code(), "PA002", "{message}");
            assert_eq!(error.message(), message);
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
