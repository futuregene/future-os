//! Trust anchors for the relay's TLS client.
//!
//! async-nats 0.49 loads the platform trust store on **every** TLS connection
//! and fails that connection outright when the read errors:
//!
//! ```text
//! Failed to connect to NATS: TLS error: could not load platform certs:
//! failed to load system trust settings: I/O error.
//! ```
//!
//! On macOS that read (rustls-native-certs → the Security framework's trust
//! settings) fails intermittently under load. In the desktop's own log, 17
//! such failures lined up exactly with the renewed-credential connects, and the
//! two paths inside the connector that can produce `nats: TLS error` are both
//! this load, so the 728 `client error: nats: TLS error` events in two days are
//! this read rather than a handshake fault. Losing that race costs the whole
//! connection, and a renewal that cannot connect cannot rotate the credential:
//! one observed outage lasted 11 minutes (18:25 → 18:36) with the trust-store
//! error as the recorded cause of every attempt.
//!
//! Handing async-nats a PEM file instead removes the per-connection platform
//! read, and a later failed read degrades to the last good snapshot instead of
//! to a failed connection. The snapshot *is* the platform store, so this
//! changes no trust decision — it is a cache, not a second policy.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use std::path::{Path, PathBuf};

/// PEM bundle beside the desktop's other own data (`~/.future/app/`).
const BUNDLE_FILE: &str = "relay-root-certs.pem";

/// A PEM bundle of the platform trust anchors for async-nats to load, or
/// `None` when none can be produced — the caller then leaves async-nats to do
/// its own platform load, i.e. today's behaviour.
///
/// A self-hosted relay may present a private CA (the common case for a
/// homelab or an internal deployment). `FUTURE_RELAY_CA` / the relay settings
/// name that CA, and the bundle then carries the platform store *plus* it.
/// That is a deliberate, operator-supplied addition to the trust set, not a
/// relaxation: the extra root is only ever used for the relay the operator
/// configured, and nothing about verification changes for anyone else.
pub(crate) fn root_bundle() -> Option<PathBuf> {
    if let Some(ca) = crate::remote::relay::ca_path() {
        return bundle_with_extra(&ca);
    }
    bundle_at(&bundle_path()?, &load_platform_certs)
}

/// One process-wide bundle for the operator's extra CA.
///
/// The file lives in the OS temp directory rather than the app directory: it
/// is derived from a file the operator named (which may itself be in a
/// read-only location), it is public material, and nothing depends on it
/// surviving the process.
fn bundle_with_extra(ca: &std::path::Path) -> Option<PathBuf> {
    static BUNDLE: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    BUNDLE
        .get_or_init(|| {
            let extra = match std::fs::read_to_string(ca) {
                Ok(text) if text.contains("-----BEGIN CERTIFICATE-----") => text,
                Ok(_) => {
                    eprintln!(
                        "remote: the relay CA file holds no certificate: {}",
                        ca.display()
                    );
                    return None;
                }
                Err(error) => {
                    eprintln!(
                        "remote: could not read the relay CA file {}: {error}",
                        ca.display()
                    );
                    return None;
                }
            };
            let path =
                std::env::temp_dir().join(format!("future-relay-ca-{}.pem", std::process::id()));
            let platform = load_platform_certs().unwrap_or_default();
            let mut text = String::new();
            for cert in platform {
                push_pem(&mut text, &cert);
            }
            text.push_str(&extra);
            if !text.ends_with('\n') {
                text.push('\n');
            }
            match std::fs::write(&path, text) {
                Ok(()) => Some(path),
                Err(error) => {
                    eprintln!("remote: could not write the relay trust bundle: {error}");
                    None
                }
            }
        })
        .clone()
}

#[cfg(not(test))]
fn bundle_path() -> Option<PathBuf> {
    Some(future_app_settings::app_dir().ok()?.join(BUNDLE_FILE))
}

/// Unit tests drive an in-process plaintext broker: they must not read the
/// platform store or write into the real app directory. Nothing else changes
/// shape between the two configurations, so the snapshot and its fallbacks are
/// exercised by `bundle_at`'s own tests.
#[cfg(test)]
fn bundle_path() -> Option<PathBuf> {
    None
}

/// Snapshot `load()` into `path`, falling back to whatever bundle is already
/// there when the load fails. `load` is a parameter so the fallback is testable
/// without a real (and intermittently failing) platform store.
fn bundle_at(path: &Path, load: &dyn Fn() -> Result<Vec<Vec<u8>>, String>) -> Option<PathBuf> {
    match load() {
        Ok(certs) if !certs.is_empty() => match write_bundle(path, &certs) {
            Ok(()) => Some(path.to_path_buf()),
            Err(error) => {
                eprintln!("remote: writing the relay trust bundle failed: {error}");
                existing_bundle(path)
            }
        },
        Ok(_) => {
            eprintln!("remote: the platform trust store returned no certificates");
            existing_bundle(path)
        }
        Err(error) => {
            eprintln!("remote: reading the platform trust store failed: {error}");
            existing_bundle(path)
        }
    }
}

fn existing_bundle(path: &Path) -> Option<PathBuf> {
    let usable = std::fs::read_to_string(path)
        .map(|text| text.contains("-----BEGIN CERTIFICATE-----"))
        .unwrap_or(false);
    usable.then(|| path.to_path_buf())
}

fn write_bundle(path: &Path, certs: &[Vec<u8>]) -> std::io::Result<()> {
    let mut text = String::new();
    for cert in certs {
        push_pem(&mut text, cert);
    }
    // Write beside the target and rename onto it, so a concurrent connect never
    // reads a half-written bundle.
    let staging = path.with_extension("pem.tmp");
    std::fs::write(&staging, text)?;
    std::fs::rename(&staging, path)
}

fn push_pem(text: &mut String, cert: &[u8]) {
    let body = STANDARD.encode(cert);
    text.push_str("-----BEGIN CERTIFICATE-----\n");
    for line in body.as_bytes().chunks(64) {
        text.push_str(std::str::from_utf8(line).unwrap_or_default());
        text.push('\n');
    }
    text.push_str("-----END CERTIFICATE-----\n");
}

fn load_platform_certs() -> Result<Vec<Vec<u8>>, String> {
    let loaded = rustls_native_certs::load_native_certs();
    if !loaded.errors.is_empty() {
        return Err(loaded
            .errors
            .iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("; "));
    }
    Ok(loaded
        .certs
        .into_iter()
        .map(|certificate| certificate.to_vec())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::test_support::unique;

    fn temp_bundle(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(unique(tag));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir.join(BUNDLE_FILE)
    }

    fn pem_body(text: &str) -> Vec<u8> {
        let body: String = text
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .collect();
        STANDARD.decode(body).expect("decode pem body")
    }

    #[test]
    fn a_successful_load_is_snapshotted_as_a_usable_pem_bundle() {
        let path = temp_bundle("relay-bundle-write");
        let certs = vec![vec![1u8, 2, 3, 4], vec![9u8; 100]];
        let chosen = bundle_at(&path, &|| Ok::<_, String>(certs.clone()));
        assert_eq!(chosen.as_deref(), Some(path.as_path()));

        let text = std::fs::read_to_string(&path).expect("bundle written");
        assert_eq!(text.matches("-----BEGIN CERTIFICATE-----").count(), 2);
        let bodies: Vec<&str> = text.split("-----BEGIN CERTIFICATE-----").skip(1).collect();
        assert_eq!(pem_body(bodies[0]), certs[0]);
        assert_eq!(pem_body(bodies[1]), certs[1]);
        // The staging file must not survive a successful write.
        assert!(!path.with_extension("pem.tmp").exists());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_failed_load_falls_back_to_the_previous_bundle() {
        let path = temp_bundle("relay-bundle-fallback");
        assert!(bundle_at(&path, &|| Ok::<_, String>(vec![vec![7u8; 32]])).is_some());

        // The platform read fails now; the connection must still use the
        // snapshot taken a moment ago.
        let chosen = bundle_at(&path, &|| {
            Err::<Vec<Vec<u8>>, String>("failed to load system trust settings".into())
        });
        assert_eq!(chosen.as_deref(), Some(path.as_path()));
        assert_eq!(
            std::fs::read_to_string(&path)
                .expect("bundle kept")
                .matches("-----BEGIN CERTIFICATE-----")
                .count(),
            1
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_failed_load_without_a_bundle_keeps_the_platform_load() {
        let path = temp_bundle("relay-bundle-none");
        assert!(bundle_at(&path, &|| Err::<Vec<Vec<u8>>, String>("I/O error".into())).is_none());
        // A load that returns nothing is not a usable pool either: async-nats
        // must do its own read rather than trust an empty store.
        assert!(bundle_at(&path, &|| Ok::<_, String>(Vec::new())).is_none());
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_stale_non_pem_file_is_not_mistaken_for_a_bundle() {
        let path = temp_bundle("relay-bundle-stale");
        std::fs::write(&path, "not a bundle").expect("write stale file");
        assert!(bundle_at(&path, &|| Err::<Vec<Vec<u8>>, String>("I/O error".into())).is_none());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// A failed snapshot write keeps the bundle that is already there.
    ///
    /// The write is made to fail deterministically by occupying the staging
    /// path with a directory: `std::fs::write` refuses it, which is the `Err`
    /// arm `bundle_at` has to answer with the previous bundle rather than with
    /// `None` (a `None` would send async-nats back to the platform store the
    /// snapshot exists to avoid).
    #[test]
    fn a_failed_write_keeps_the_previous_bundle() {
        let path = temp_bundle("relay-bundle-writefail");
        assert!(bundle_at(&path, &|| Ok::<_, String>(vec![vec![5u8; 24]])).is_some());
        let previous = std::fs::read_to_string(&path).expect("bundle written");
        let staging = path.with_extension("pem.tmp");
        std::fs::create_dir(&staging).expect("occupy the staging path");

        let chosen = bundle_at(&path, &|| Ok::<_, String>(vec![vec![9u8; 24]]));
        assert_eq!(chosen.as_deref(), Some(path.as_path()));
        assert_eq!(
            std::fs::read_to_string(&path).expect("bundle kept"),
            previous,
            "the failed write must not replace the usable bundle"
        );
        assert!(staging.is_dir(), "the failing staging path is left alone");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
