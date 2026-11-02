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

use crate::console;
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
/// homelab or an internal deployment). `FUTURE_RELAY_CA` / `--relay-ca` name
/// that CA, and the bundle then carries the platform store *plus* it.
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
                    console::eprintln!(
                        "remote: the relay CA file holds no certificate: {}",
                        ca.display()
                    );
                    return None;
                }
                Err(error) => {
                    console::eprintln!(
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
                    console::eprintln!("remote: could not write the relay trust bundle: {error}");
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
                console::eprintln!("remote: writing the relay trust bundle failed: {error}");
                existing_bundle(path)
            }
        },
        Ok(_) => {
            console::eprintln!("remote: the platform trust store returned no certificates");
            existing_bundle(path)
        }
        Err(error) => {
            console::eprintln!("remote: reading the platform trust store failed: {error}");
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

    /// The operator's own relay CA is handed to async-nats *together with* the
    /// platform store, in one file: async-nats loads one bundle per connection,
    /// so a private root has to travel inside the snapshot rather than replace
    /// it. This is the branch that only runs when `FUTURE_RELAY_CA` names a file,
    /// and it is also the only path that reads the platform store here.
    #[test]
    fn an_operator_relay_ca_is_bundled_with_the_platform_store() {
        let _home = crate::auth_store::test_support::HomeGuard::new("relay-ca-bundle");
        let dir = std::env::temp_dir().join(unique("relay-ca-extra"));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let ca = dir.join("relay-ca.pem");
        // The bundle code only looks for the marker; the bytes are the TLS
        // backend's business (see `bundle_with_extra`). Deliberately written
        // **without** a trailing newline: the bundle has to append one (an
        // operator's file may end anywhere), and the assertion below is what
        // pins that append — drop it and a PEM reader downstream sees a
        // `-----END CERTIFICATE-----` glued to the next thing in the file.
        std::fs::write(
            &ca,
            "-----BEGIN CERTIFICATE-----\nZGVyaXZlZA==\n-----END CERTIFICATE-----",
        )
        .expect("write the operator CA");

        std::env::set_var("FUTURE_RELAY_CA", &ca);
        let bundle = root_bundle().expect("a bundle is produced from the extra CA");
        let text = std::fs::read_to_string(&bundle).expect("the bundle is readable");

        assert!(
            text.contains("-----BEGIN CERTIFICATE-----"),
            "the operator CA is in the bundle"
        );
        assert!(
            text.ends_with('\n'),
            "the bundle ends on a line boundary so a strict PEM reader accepts it"
        );
        // The file is named after this process and lives in the temp directory:
        // it is derived public material, not part of the app's data directory.
        assert_eq!(
            bundle.file_name().and_then(|name| name.to_str()),
            Some(format!("future-relay-ca-{}.pem", std::process::id()).as_str())
        );

        std::env::remove_var("FUTURE_RELAY_CA");
        let _ = std::fs::remove_file(&bundle);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // The refusal arms of `bundle_with_extra` are not exercised here on
    // purpose: the bundle is a `OnceLock`, so only one construction happens per
    // process and whichever test runs first decides which arm executes.
    // `fixtures` below gives each of them a process of its own for exactly that
    // reason.
}

/// The arms of [`bundle_with_extra`] that only a *fresh* process can take.
///
/// The snapshot is a `OnceLock`: whichever construction wins decides which arm
/// the rest of the test binary ever sees, so an in-process test can cover at
/// most one of them and the others read `DA:0` no matter how many tests are
/// written. Each case therefore re-executes this same test binary with
/// `--exact` and one case marker in its environment — the same shape
/// `agent_host`'s block fixtures use — and asserts on what that process
/// reported, which is also the only way a `console::eprintln!` arm is
/// observable at all.
#[cfg(test)]
mod fixtures {
    use super::*;
    use std::process::{Command, Output, Stdio};

    const CASE: &str = "FUTURE_RELAY_CA_BUNDLE_CASE";

    /// One isolated home and the command that re-executes this binary for it.
    ///
    /// Every case builds its own command in its own test, so there is no
    /// "unknown case" arm to leave uncovered: the case string only reaches the
    /// child, which switches on nothing.
    struct Fixture {
        dir: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let parent =
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/host-fixtures");
            std::fs::create_dir_all(&parent).expect("fixture parent");
            Fixture {
                dir: tempfile::tempdir_in(&parent).expect("fixture dir"),
            }
        }

        fn write(&self, name: &str, text: &str) -> PathBuf {
            let path = self.dir.path().join(name);
            std::fs::write(&path, text).expect("write fixture file");
            path
        }

        fn absent(&self, name: &str) -> PathBuf {
            self.dir.path().join(name)
        }

        fn command(&self, case: &str) -> Command {
            let mut command = Command::new(std::env::current_exe().expect("test binary"));
            command
                .args([
                    "--exact",
                    "remote::trust::fixtures::bundle_fixture",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(CASE, case)
                .env("HOME", self.dir.path())
                .env("USERPROFILE", self.dir.path())
                .env("FUTURE_HOME", self.dir.path().join(".future"))
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            command
        }
    }

    /// Run one case and hand back what it printed. The child's own exit status
    /// is asserted here, once, so every case inherits "it ran at all" without
    /// restating it.
    fn run(case: &str, mut command: Command) -> (String, String) {
        let output: Output = command.output().expect("run the fixture");
        // Hoisted: an `assert!` message argument is evaluated only on failure,
        // so an inline `from_utf8_lossy` would be a residual line of its own.
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        assert!(output.status.success(), "{case}: {stdout} {stderr}");
        (stdout, stderr)
    }

    /// A file the operator named that holds no certificate. The bundle must be
    /// refused with the file named: a silent `None` here looks exactly like "no
    /// operator CA", and async-nats would then do the platform load the
    /// operator was trying to add a root beside.
    #[test]
    fn a_relay_ca_file_that_holds_no_certificate_is_refused_by_name() {
        let fixture = Fixture::new();
        let ca = fixture.write("relay-ca.pem", "this is not a certificate\n");
        let mut command = fixture.command("no-certificate");
        command.env("FUTURE_RELAY_CA", &ca);
        let (_, reported) = run("no-certificate", command);
        let named = reported.contains("holds no certificate");
        assert!(named, "{reported}");
        let path_named = reported.contains("relay-ca.pem");
        assert!(path_named, "{reported}");
    }

    /// A file the operator named that cannot even be read (a typo, a path a
    /// sandboxed Desktop cannot see). Same sentence, and the path is in it.
    #[test]
    fn a_relay_ca_file_that_cannot_be_read_is_refused_by_name() {
        let fixture = Fixture::new();
        let ca = fixture.absent("absent.pem");
        let mut command = fixture.command("unreadable");
        command.env("FUTURE_RELAY_CA", &ca);
        let (_, reported) = run("unreadable", command);
        let named = reported.contains("could not read the relay CA file");
        assert!(named, "{reported}");
        let path_named = reported.contains("absent.pem");
        assert!(path_named, "{reported}");
        assert!(!ca.exists(), "the fixture proved a missing file");
    }

    /// The bundle cannot be written — the only arm that leaves async-nats to do
    /// the platform load. Here the temp directory the bundle is written into
    /// does not exist, which is what a Desktop with an unusable `TMPDIR` sees.
    #[test]
    fn a_bundle_that_cannot_be_written_leaves_the_platform_load_in_place() {
        let fixture = Fixture::new();
        let ca = fixture.write(
            "relay-ca.pem",
            "-----BEGIN CERTIFICATE-----\nZGVyaXZlZA==\n-----END CERTIFICATE-----\n",
        );
        let tmp = fixture.absent("no-such-tmp");
        let mut command = fixture.command("unwritable");
        command.env("FUTURE_RELAY_CA", &ca).env("TMPDIR", &tmp);
        let (_, reported) = run("unwritable", command);
        let named = reported.contains("could not write the relay trust bundle");
        assert!(named, "{reported}");
        // The bundle is written into the temp directory the environment names,
        // so a `TMPDIR` that does not exist is what the write fails on.
        assert!(
            !tmp.exists(),
            "the fixture's temp directory must stay absent"
        );
    }

    /// The child half: one case per process, one construction of the bundle.
    #[test]
    fn bundle_fixture() {
        let Ok(case) = std::env::var(CASE) else {
            return;
        };
        let bundle = root_bundle();
        assert!(
            bundle.is_none(),
            "{case}: a refused CA must not produce a bundle: {bundle:?}"
        );
    }
}
