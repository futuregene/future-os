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

use std::path::PathBuf;

pub(crate) use future_remote_client::credentials::PeerCreds;
#[cfg(test)]
pub(crate) use future_remote_client::credentials::PeerIdentity;

pub(crate) use future_remote_client::credentials::{PeerBook, PeerLabel, PeerRecord};

fn book_path() -> Result<PathBuf, crate::AppError> {
    Ok(crate::store::future_dir()?.join("remote_peers.json"))
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
#[cfg(test)]
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
#[cfg(test)]
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
#[cfg(test)]
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

/// Compare and write while holding the original per-file lock. A late refresh
/// or handshake may never overwrite a newly paired identity for the same host.
fn edit_if_current(
    expected: &PeerCreds,
    edit: impl FnOnce(&mut PeerBook, usize),
) -> Result<bool, crate::AppError> {
    let path = book_path()?;
    crate::config_io::with_config_lock(&path, || {
        let mut book = load()?;
        let Some(index) = book.peers.iter().position(|record| {
            record.creds.desktop_id == expected.desktop_id
                && record.creds.pair_id == expected.pair_id
                && record.creds.nkey_seed == expected.nkey_seed
                && record.creds.refresh_token == expected.refresh_token
        }) else {
            return Ok(false);
        };
        edit(&mut book, index);
        let value = serde_json::to_value(&book)
            .map_err(|error| crate::AppError::Message(format!("encode peer book: {error}")))?;
        crate::config_io::write_json_atomic(&path, &value, true)?;
        Ok(true)
    })
}
pub(crate) fn update_credentials_if_current(
    expected: &PeerCreds,
    fresh: &future_remote_client::host::Refreshed,
) -> Result<bool, crate::AppError> {
    edit_if_current(expected, |book, index| {
        let creds = &mut book.peers[index].creds;
        creds.user_jwt = fresh.user_jwt.clone();
        creds.nats_url = fresh.nats_url.clone();
        creds.nats_ws_url = fresh.nats_ws_url.clone();
        creds.jwt_expires_at = fresh.jwt_expires_at;
    })
}
pub(crate) fn clear_secret_if_current(expected: &PeerCreds) -> Result<bool, crate::AppError> {
    edit_if_current(expected, |book, index| {
        if let Some(identity) = book.peers[index].creds.secure.as_mut() {
            identity.secret = None;
        }
    })
}
pub(crate) fn remove_if_current(expected: &PeerCreds) -> Result<bool, crate::AppError> {
    edit_if_current(expected, |book, index| {
        book.peers.remove(index);
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
    fn late_grants_and_handshakes_cannot_mutate_a_repaired_host() {
        let _home = home("peer-cas-repaired");
        let original = creds("desktop_a", "pair_old");
        upsert(original.clone()).unwrap();
        let replacement = creds("desktop_a", "pair_new");
        upsert(replacement.clone()).unwrap();
        let fresh = future_remote_client::host::Refreshed {
            user_jwt: "late-token-fixture".into(),
            nats_url: "nats://127.0.0.1:1".into(),
            nats_ws_url: "wss://example.invalid".into(),
            jwt_expires_at: 2_000,
        };
        assert!(!update_credentials_if_current(&original, &fresh).unwrap());
        assert!(!clear_secret_if_current(&original).unwrap());
        assert!(!remove_if_current(&original).unwrap());
        assert_eq!(
            load().unwrap().find("desktop_a").unwrap().creds,
            replacement
        );
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
        std::env::remove_var("HOME");
        // `USERPROFILE` is the Windows spelling and is not set here, so removing
        // it is the whole of its handling: restoring a value that never existed
        // would invent one.
        std::env::remove_var("USERPROFILE");
        let result = load();
        // Restore *unconditionally* before asserting: a panic here would
        // otherwise leave every later test without a home.
        std::env::set_var("HOME", previous);
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
}
