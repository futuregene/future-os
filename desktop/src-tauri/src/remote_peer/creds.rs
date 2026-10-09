//! Client-side credential book: what this desktop knows about the *remote*
//! desktops it has paired with.
//!
//! Deliberately a separate file from `remote_pairing.json` (the host-side
//! credential for phones pairing *into* this machine). The two roles can be
//! active at once, and a bug in one must never rewrite the other.
//!
//! Threat model, unchanged from the host side: the NKey seed and refresh token
//! are bearer credentials, so the file is owner-only (`0600`) and written
//! atomically (temp file + rename) under a per-path lock. The file is a *book*
//! because a desktop may be paired with several remote desktops at once; each
//! entry is independent, and losing/corrupting one must not take the others
//! with it.
//!
//! Sort order is the book's own (insertion order); the session list's ordering
//! is a UI concern and comes from `lastMessageAt`, never from this file.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The Noise identity this desktop uses with one remote host.
///
/// `peer_public_key` is the host's Noise static key. It is known from the
/// invitation itself (the QR carries it), not learned from the handshake — so
/// it is pinned at claim time and *verified* during every handshake, including
/// the first one. A key offered by the broker or the platform is refused
/// rather than adopted: there is no trust-on-first-use here.
///
/// `secret` is the invitation PSK. It exists only until the first handshake
/// succeeds; the host deletes its own copy at the same moment, so keeping ours
/// would leave a verbatim bearer credential on disk for no benefit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PeerIdentity {
    pub private_key: String,
    pub public_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_public_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
}

/// Everything needed to reach one remote host again, minus the label the user
/// chose (that lives on the [`PeerRecord`], so it survives a credential
/// refresh).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PeerCreds {
    pub pair_id: String,
    /// The *host* desktop's identity — what the UI names and de-duplicates by.
    pub desktop_id: String,
    /// This installation's pairing device id (shared with the host role; the
    /// platform binds the pair to it).
    pub device_id: String,
    pub nkey_seed: String,
    pub user_jwt: String,
    pub refresh_token: String,
    pub nats_url: String,
    pub nats_ws_url: String,
    /// Unix seconds. Drives the same pre-expiry refresh the host performs.
    pub jwt_expires_at: i64,
    /// `…/client/v1/remote/auth/token` for the platform that minted the pair.
    pub token_url: String,
    /// Absent only for a v1-era import; a v2 pairing always has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secure: Option<PeerIdentity>,
}

impl PeerCreds {
    /// Expired (or unreadable) JWTs are refreshed before a connect attempt.
    /// Mirrors the mobile client's 60 s margin: a token that expires *during*
    /// the handshake fails it in a way that looks like a protocol fault.
    pub(crate) fn needs_refresh(&self, now: i64) -> bool {
        self.jwt_expires_at > 0 && self.jwt_expires_at - 60 <= now
    }

    pub(crate) fn revoke_url(&self) -> String {
        self.token_url.replace("/auth/token", "/pair/revoke")
    }
}

/// One paired host: its credentials plus the purely-local presentation the user
/// picked. `name`/`icon` never reach the platform or the host desktop.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PeerLabel {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// A short glyph from the fixed picker (`desktopIcons.ts`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PeerRecord {
    #[serde(flatten)]
    pub creds: PeerCreds,
    #[serde(flatten)]
    pub label: PeerLabel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PeerBook {
    #[serde(default)]
    pub peers: Vec<PeerRecord>,
}

impl PeerBook {
    pub(crate) fn find(&self, desktop_id: &str) -> Option<&PeerRecord> {
        self.peers
            .iter()
            .find(|peer| peer.creds.desktop_id == desktop_id)
    }
}

fn book_path() -> Result<PathBuf, crate::AppError> {
    let home = crate::home_dir().ok_or_else(|| {
        crate::AppError::Message("HOME/USERPROFILE environment variable is not set.".to_string())
    })?;
    Ok(PathBuf::from(home)
        .join(".future")
        .join("remote_peers.json"))
}

/// Read the book. A missing file is an empty book; a corrupt one is an error —
/// returning "no pairings" for unreadable credentials would silently drop the
/// user's pairings on the next write.
pub(crate) fn load() -> Result<PeerBook, crate::AppError> {
    let path = book_path()?;
    if !path.try_exists()? {
        return Ok(PeerBook::default());
    }
    let value = crate::config_io::read_json_object(&path)?;
    Ok(serde_json::from_value(value)?)
}

/// Insert or replace one pairing, keeping the user's label for that host.
///
/// Re-pairing the same host replaces its credentials *and* pins the new peer
/// key: a re-pair is an explicit new trust decision, not a merge.
pub(crate) fn upsert(creds: PeerCreds) -> Result<(), crate::AppError> {
    let path = book_path()?;
    crate::config_io::with_config_lock(&path, || {
        let mut book = load()?;
        let mut record = PeerRecord {
            creds,
            label: PeerLabel::default(),
        };
        if let Some(previous) = book.find(&record.creds.desktop_id) {
            record.label = previous.label.clone();
        }
        book.peers
            .retain(|peer| peer.creds.desktop_id != record.creds.desktop_id);
        book.peers.push(record);
        let value = serde_json::to_value(&book)
            .map_err(|error| crate::AppError::Message(format!("encode peer book: {error}")))?;
        crate::config_io::write_json_atomic(&path, &value, true)
    })
}

/// Store the user's chosen name/icon for one host. Empty strings clear a field.
pub(crate) fn set_label(
    desktop_id: &str,
    name: Option<&str>,
    icon: Option<&str>,
) -> Result<(), crate::AppError> {
    let path = book_path()?;
    crate::config_io::with_config_lock(&path, || {
        let mut book = load()?;
        let Some(record) = book
            .peers
            .iter_mut()
            .find(|peer| peer.creds.desktop_id == desktop_id)
        else {
            return Ok(());
        };
        let clean = |value: Option<&str>| {
            value
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        if name.is_some() {
            record.label.name = clean(name);
        }
        if icon.is_some() {
            record.label.icon = clean(icon);
        }
        let value = serde_json::to_value(&book)
            .map_err(|error| crate::AppError::Message(format!("encode peer book: {error}")))?;
        crate::config_io::write_json_atomic(&path, &value, true)
    })
}

/// Forget one pairing locally. Server-side revocation is the caller's job and
/// is best-effort (see [`super::platform::revoke`]).
pub(crate) fn remove(desktop_id: &str) -> Result<(), crate::AppError> {
    let path = book_path()?;
    crate::config_io::with_config_lock(&path, || {
        let mut book = load()?;
        book.peers
            .retain(|peer| peer.creds.desktop_id != desktop_id);
        let value = serde_json::to_value(&book)
            .map_err(|error| crate::AppError::Message(format!("encode peer book: {error}")))?;
        crate::config_io::write_json_atomic(&path, &value, true)
    })
}

/// Persist a refreshed JWT/endpoint set for one pairing, leaving the label and
/// the pinned identity alone.
pub(crate) fn update_credentials(
    desktop_id: &str,
    user_jwt: String,
    nats_url: String,
    nats_ws_url: String,
    jwt_expires_at: i64,
) -> Result<(), crate::AppError> {
    let path = book_path()?;
    crate::config_io::with_config_lock(&path, || {
        let mut book = load()?;
        let Some(record) = book
            .peers
            .iter_mut()
            .find(|peer| peer.creds.desktop_id == desktop_id)
        else {
            return Ok(());
        };
        record.creds.user_jwt = user_jwt;
        record.creds.nats_url = nats_url;
        record.creds.nats_ws_url = nats_ws_url;
        record.creds.jwt_expires_at = jwt_expires_at;
        let value = serde_json::to_value(&book)
            .map_err(|error| crate::AppError::Message(format!("encode peer book: {error}")))?;
        crate::config_io::write_json_atomic(&path, &value, true)
    })
}

/// Forget the invitation PSK once the first handshake has succeeded.
///
/// Called only after the host has both proved possession of the PSK and bound
/// this identity. From here the pair uses `Noise_IK` with the pinned peer key,
/// which is what makes the (now useless) invitation a dead bearer token.
pub(crate) fn clear_secret(desktop_id: &str) -> Result<(), crate::AppError> {
    let path = book_path()?;
    crate::config_io::with_config_lock(&path, || {
        let mut book = load()?;
        let Some(record) = book
            .peers
            .iter_mut()
            .find(|peer| peer.creds.desktop_id == desktop_id)
        else {
            return Ok(());
        };
        if let Some(identity) = record.creds.secure.as_mut() {
            identity.secret = None;
        }
        let value = serde_json::to_value(&book)
            .map_err(|error| crate::AppError::Message(format!("encode peer book: {error}")))?;
        crate::config_io::write_json_atomic(&path, &value, true)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds(desktop_id: &str, pair_id: &str) -> PeerCreds {
        PeerCreds {
            pair_id: pair_id.into(),
            desktop_id: desktop_id.into(),
            device_id: "dev_1".into(),
            nkey_seed: "SUAISECRET".into(),
            user_jwt: "jwt".into(),
            refresh_token: "refresh".into(),
            nats_url: "nats://127.0.0.1:4222".into(),
            nats_ws_url: "wss://example.invalid".into(),
            jwt_expires_at: 1_000,
            token_url: "https://future-os.cn/client/v1/remote/auth/token".into(),
            secure: Some(PeerIdentity {
                private_key: "priv".into(),
                public_key: "pub".into(),
                peer_public_key: Some("PEER".into()),
                secret: None,
            }),
        }
    }

    fn home(name: &str) -> crate::remote::test_support::HomeGuard {
        crate::remote::test_support::HomeGuard::new(name)
    }

    #[test]
    fn a_missing_book_reads_as_empty_and_round_trips_entries() {
        let _home = home("peer-book-roundtrip");
        assert!(load().expect("empty book").peers.is_empty());

        upsert(creds("desktop_a", "pair_a")).expect("insert a");
        upsert(creds("desktop_b", "pair_b")).expect("insert b");
        let book = load().expect("book");
        assert_eq!(book.peers.len(), 2);
        assert_eq!(book.find("desktop_a").unwrap().creds.pair_id, "pair_a");
    }

    /// The whole point of a book: several remotes coexist, and re-pairing one
    /// must not disturb the others.
    #[test]
    fn re_pairing_one_host_replaces_only_that_host() {
        let _home = home("peer-book-replace");
        upsert(creds("desktop_a", "pair_a")).expect("insert a");
        upsert(creds("desktop_b", "pair_b")).expect("insert b");

        let mut again = creds("desktop_a", "pair_a2");
        again.refresh_token = "refresh2".into();
        upsert(again).expect("re-pair a");

        let book = load().expect("book");
        assert_eq!(book.peers.len(), 2);
        assert_eq!(book.find("desktop_a").unwrap().creds.pair_id, "pair_a2");
        assert_eq!(book.find("desktop_b").unwrap().creds.pair_id, "pair_b");
    }

    /// A refresh rotates the credential fields but must not drop the name or
    /// icon the user chose — that is exactly why the label is a separate struct.
    #[test]
    fn a_credential_refresh_keeps_the_users_label() {
        let _home = home("peer-book-label-survives");
        upsert(creds("desktop_a", "pair_a")).expect("insert");
        set_label("desktop_a", Some("Studio iMac"), Some("🖥")).expect("label");

        update_credentials(
            "desktop_a",
            "jwt2".into(),
            "nats://127.0.0.1:4223".into(),
            "wss://example.invalid".into(),
            2_000,
        )
        .expect("refresh");

        let record = load().expect("book").find("desktop_a").cloned().unwrap();
        assert_eq!(record.creds.user_jwt, "jwt2");
        assert_eq!(record.creds.nats_url, "nats://127.0.0.1:4223");
        assert_eq!(record.label.name.as_deref(), Some("Studio iMac"));
        assert_eq!(record.label.icon.as_deref(), Some("🖥"));
    }

    #[test]
    fn labels_are_clearable_and_never_invented_for_an_unknown_host() {
        let _home = home("peer-book-label-clear");
        upsert(creds("desktop_a", "pair_a")).expect("insert");
        set_label("desktop_a", Some("Named"), Some("🖥")).expect("label");
        set_label("desktop_a", Some("   "), Some("")).expect("clear");

        let record = load().expect("book").find("desktop_a").cloned().unwrap();
        assert_eq!(record.label.name, None);
        assert_eq!(record.label.icon, None);

        assert!(set_label("desktop_missing", Some("x"), Some("y")).is_ok());
        assert_eq!(load().expect("book").peers.len(), 1);
    }

    /// The invitation PSK is a bearer credential for one pairing. It is stored
    /// only so an interrupted first pairing can be retried, and removed the
    /// moment the host has bound this identity. The pinned peer key stays — it
    /// is what authenticates every later `Noise_IK` handshake and it comes from
    /// the invitation, not from the handshake that just succeeded.
    #[test]
    fn the_invitation_secret_is_stored_then_cleared_while_the_peer_key_stays() {
        let _home = home("peer-book-secret");
        let mut fresh = creds("desktop_a", "pair_a");
        if let Some(identity) = fresh.secure.as_mut() {
            identity.secret = Some("PSK".into());
        }
        upsert(fresh).expect("insert");
        assert_eq!(
            identity_of("desktop_a").secret.as_deref(),
            Some("PSK"),
            "a first pairing keeps the PSK so a lost response can be retried"
        );

        clear_secret("desktop_a").expect("clear");
        let identity = identity_of("desktop_a");
        assert_eq!(identity.secret, None);
        assert_eq!(
            identity.peer_public_key.as_deref(),
            Some("PEER"),
            "clearing the PSK must not unpin the host identity"
        );
        // Idempotent, and a no-op for an unknown host.
        clear_secret("desktop_a").expect("clear again");
        clear_secret("desktop_missing").expect("unknown host");
    }

    fn identity_of(desktop_id: &str) -> PeerIdentity {
        load()
            .expect("book")
            .find(desktop_id)
            .cloned()
            .expect("peer")
            .creds
            .secure
            .expect("noise identity")
    }

    #[test]
    fn removing_one_host_leaves_the_rest_and_a_reused_desktop_id_starts_unpinned() {
        let _home = home("peer-book-remove");
        upsert(creds("desktop_a", "pair_a")).expect("insert a");
        upsert(creds("desktop_b", "pair_b")).expect("insert b");
        remove("desktop_a").expect("remove a");

        let book = load().expect("book");
        assert_eq!(book.peers.len(), 1);
        assert!(book.find("desktop_b").is_some());

        // Re-pairing after an unpair is a *new* trust decision: whatever the
        // previous pairing learned about that host is gone with it.
        let mut re_paired = creds("desktop_a", "pair_a3");
        if let Some(identity) = re_paired.secure.as_mut() {
            identity.peer_public_key = None;
            identity.secret = Some("NEWPSK".into());
        }
        upsert(re_paired).expect("re-pair a");
        let identity = identity_of("desktop_a");
        assert_eq!(identity.peer_public_key, None);
        assert_eq!(identity.secret.as_deref(), Some("NEWPSK"));
    }

    #[test]
    fn an_unreadable_book_is_an_error_not_an_empty_one() {
        let _home = home("peer-book-corrupt");
        let path = book_path().expect("path");
        std::fs::create_dir_all(path.parent().unwrap()).expect("home dir");
        std::fs::write(&path, "{ not json").expect("write junk");
        assert!(load().is_err());
        // An unreadable book stays an error for every reader; there is no
        // "assume empty" accessor, because that is how a pairing gets dropped.
        // A write after a corrupt read must not silently drop the pairing.
        assert!(persist_after_corrupt(&path).is_err());
    }

    /// With no home there is nowhere to keep the book, and that is a local
    /// configuration fault rather than "this machine has no pairings".
    #[test]
    fn an_absent_home_is_an_error_not_an_empty_book() {
        let _home = home("peer-book-no-home");
        // `HOME` is process-global, so this runs under the same lock every other
        // home-scoped test holds.
        let previous = std::env::var("HOME").expect("the harness always has a HOME");
        let previous_profile = std::env::var("USERPROFILE").ok();
        std::env::remove_var("HOME");
        std::env::remove_var("USERPROFILE");
        let result = load();
        // Restore *unconditionally* before asserting: a panic here would
        // otherwise leave every later test without a home.
        std::env::set_var("HOME", previous);
        if let Some(profile) = previous_profile {
            std::env::set_var("USERPROFILE", profile);
        }
        let error = result.expect_err("an absent home cannot be resolved");
        assert!(error.to_string().contains("HOME/USERPROFILE"), "{error}");
    }

    /// A refresh can arrive for a host the user has already removed (a slow
    /// rotation racing an unpair, say). Writing it back would resurrect a
    /// partial entry; the call is a no-op instead.
    #[test]
    fn a_credential_refresh_for_an_unknown_host_is_a_no_op() {
        let _home = home("peer-book-refresh-unknown");
        update_credentials(
            "desktop_nobody",
            "jwt".into(),
            "nats://127.0.0.1:4222".into(),
            "wss://example.invalid".into(),
            1_000,
        )
        .expect("no-op");
        assert!(load().expect("book").peers.is_empty());
        clear_secret("desktop_nobody").expect("no-op");
    }

    fn persist_after_corrupt(_path: &std::path::Path) -> Result<(), crate::AppError> {
        set_label("desktop_a", Some("x"), None)
    }

    /// Refreshing is a clock decision, with a margin: a JWT that expires inside
    /// the margin is refreshed *before* the connect that would otherwise fail.
    #[test]
    fn refresh_is_required_inside_the_sixty_second_margin() {
        let mut creds = creds("desktop_a", "pair_a");
        creds.jwt_expires_at = 1_000;
        assert!(!creds.needs_refresh(900));
        assert!(creds.needs_refresh(940));
        assert!(creds.needs_refresh(1_000));
        // An unreadable expiry is not "expired": it is a broken credential the
        // handshake will reject, and looping a refresh on it would hammer the
        // token endpoint.
        creds.jwt_expires_at = 0;
        assert!(!creds.needs_refresh(9_999));
    }

    /// The revoke endpoint is derived, not stored, so a platform change (or a
    /// self-hosted one) needs no extra field — matching the host and mobile.
    #[test]
    fn the_revoke_url_is_derived_from_the_token_url() {
        assert_eq!(
            creds("desktop_a", "pair_a").revoke_url(),
            "https://future-os.cn/client/v1/remote/pair/revoke"
        );
    }
}
