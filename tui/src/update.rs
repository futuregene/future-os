//! Best-effort startup update notice. No downloads, installation, or agent RPCs.

use semver::Version;
use serde::Deserialize;
use std::time::Duration;

const RELEASE_MANIFEST_URL: &str = "https://dl.future-os.cn/releases/latest.json";
const NIGHTLY_MANIFEST_URL: &str = "https://dl.future-os.cn/nightly/latest.json";
const CHECK_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_MANIFEST_BYTES: usize = 256 * 1024;

#[derive(Deserialize)]
struct Manifest {
    version: String,
}

fn is_release(version: &Version) -> bool {
    version.major > 0 && version.pre.is_empty() && version.build.is_empty()
}

fn is_numbered_build(version: &Version) -> bool {
    version.major == 0
        && matches!(version.build.as_str(), "test" | "nightly")
        && version.pre.as_str().parse::<u64>().is_ok()
}

fn manifest_url(current: &Version) -> Option<&'static str> {
    if is_release(current) {
        Some(RELEASE_MANIFEST_URL)
    } else if is_numbered_build(current) {
        Some(NIGHTLY_MANIFEST_URL)
    } else {
        // A commit hash/local build has no reliable ordering against published
        // builds. Do not tell source-build users to "upgrade" to an older build.
        None
    }
}

fn newer_version(current: &Version, manifest: Manifest) -> Option<Version> {
    let latest = Version::parse(&manifest.version).ok()?;
    let same_channel = if is_release(current) {
        is_release(&latest)
    } else {
        is_numbered_build(current)
            && is_numbered_build(&latest)
            && latest.build.as_str() == "nightly"
    };
    // Build metadata is not ordered: +test and +nightly share one run counter.
    (same_channel && latest.cmp_precedence(current).is_gt()).then_some(latest)
}

fn notice(current: &Version, latest: &Version) -> String {
    let installer = if cfg!(windows) {
        "install.ps1"
    } else {
        "install.sh"
    };
    let channel_hint = if latest.build.as_str() == "nightly" {
        " (set FUTUREOS_BASE=https://dl.future-os.cn/nightly)"
    } else {
        ""
    };
    format!(
        "New FutureOS version available: v{current} → v{latest}.\n\
         To update, re-run the official installer: https://dl.future-os.cn/{installer}{channel_hint}"
    )
}

async fn check_manifest(current: &Version, url: &str, timeout: Duration) -> Option<String> {
    // The outer deadline also covers client setup, DNS, and reading the body.
    tokio::time::timeout(timeout, async {
        let client = reqwest::Client::builder().timeout(timeout).build().ok()?;
        let mut response = client
            .get(url)
            .header(reqwest::header::CACHE_CONTROL, "no-cache")
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?;
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.ok()? {
            if body.len().saturating_add(chunk.len()) > MAX_MANIFEST_BYTES {
                return None;
            }
            body.extend_from_slice(&chunk);
        }
        let manifest = serde_json::from_slice(&body).ok()?;
        let latest = newer_version(current, manifest)?;
        Some(notice(current, &latest))
    })
    .await
    .ok()
    .flatten()
}

/// Called only by the interactive loop; failures intentionally stay silent.
pub(crate) async fn check(current: &str) -> Option<String> {
    let current = Version::parse(current).ok()?;
    let url = manifest_url(&current)?;
    check_manifest(&current, url, CHECK_TIMEOUT).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn selects_published_channels_only() {
        for (version, expected) in [
            ("1.2.3", Some(RELEASE_MANIFEST_URL)),
            ("0.0.2-100+test", Some(NIGHTLY_MANIFEST_URL)),
            ("0.0.2-100+nightly", Some(NIGHTLY_MANIFEST_URL)),
            ("0.0.2-abcdef", None),
            ("0.0.2-abcdef+dev", None),
            ("0.0.2-abcdef+local", None),
            ("0.0.2-abcdef+local.dirty", None),
            ("0.0.2-abcdef+nightly", None),
            ("0.0.0", None),
            ("1.2.3-beta", None),
        ] {
            assert_eq!(manifest_url(&Version::parse(version).unwrap()), expected);
        }
    }

    #[test]
    fn compares_versions_without_downgrades_or_cross_channel_notices() {
        for (current, latest, expected) in [
            ("1.2.3", "1.2.4", true),
            ("1.9.0", "1.10.0", true),
            ("1.2.3", "2.0.0", true),
            ("1.2.3", "1.2.3", false),
            ("2.0.0", "1.9.0", false),
            ("1.2.3", "2.0.0-beta", false),
            ("1.2.3", "1.2.3+metadata", false),
            ("1.2.3", "0.0.2-999+nightly", false),
            ("0.0.2-9+nightly", "0.0.2-10+nightly", true),
            ("0.0.2-9+test", "0.0.2-10+nightly", true),
            ("0.0.2-10+test", "0.0.2-9+nightly", false),
            ("0.0.2-10+test", "0.0.2-10+nightly", false),
            ("0.0.2-10+nightly", "0.0.2-10+nightly", false),
            ("0.0.2-10+nightly", "0.0.2-11+test", false),
            ("0.0.2-10+nightly", "0.0.3-1+nightly", true),
            ("0.0.3-1+nightly", "0.0.2-999+nightly", false),
            ("0.0.2-10+nightly", "1.0.0", false),
            ("1.2.3", "invalid\u{1b}[2J", false),
            ("1.2.3", "", false),
        ] {
            let result = newer_version(
                &Version::parse(current).unwrap(),
                Manifest {
                    version: latest.into(),
                },
            );
            assert_eq!(result.is_some(), expected, "{current} -> {latest}");
        }
    }

    #[test]
    fn notice_has_versions_and_platform_installer_with_channel_hint() {
        for (current, latest) in [("1.2.3", "1.2.4"), ("0.0.2-9+test", "0.0.2-10+nightly")] {
            let message = notice(
                &Version::parse(current).unwrap(),
                &Version::parse(latest).unwrap(),
            );
            assert!(message.contains(current));
            assert!(message.contains(latest));
            assert!(message.contains(if cfg!(windows) {
                "install.ps1"
            } else {
                "install.sh"
            }));
            assert_eq!(
                message.contains("FUTUREOS_BASE="),
                latest.ends_with("+nightly")
            );
        }
    }

    async fn serve(
        status: &str,
        body: &str,
        delay: Duration,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/latest.json", listener.local_addr().unwrap());
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let _ = socket.read(&mut request).await;
            tokio::time::sleep(delay).await;
            let _ = socket.write_all(response.as_bytes()).await;
        });
        (url, task)
    }

    #[tokio::test]
    async fn http_check_only_notifies_for_a_valid_newer_manifest() {
        for (status, body, expected) in [
            ("200 OK", r#"{"version":"1.2.4","assets":{}}"#, true),
            ("200 OK", r#"{"version":"1.2.3"}"#, false),
            ("200 OK", r#"{"version":"1.2.2"}"#, false),
            ("200 OK", r#"{"version":42}"#, false),
            ("200 OK", "{}", false),
            ("200 OK", "not json", false),
            ("503 Unavailable", r#"{"version":"1.2.4"}"#, false),
        ] {
            let (url, server) = serve(status, body, Duration::ZERO).await;
            let result =
                check_manifest(&Version::parse("1.2.3").unwrap(), &url, CHECK_TIMEOUT).await;
            server.await.unwrap();
            assert_eq!(result.is_some(), expected, "{status}: {body}");
        }
    }

    #[tokio::test]
    async fn http_timeout_and_oversized_manifest_are_silent() {
        let current = Version::parse("1.2.3").unwrap();
        let (url, server) = serve("200 OK", "{}", Duration::from_secs(60)).await;
        assert!(check_manifest(&current, &url, Duration::from_millis(50))
            .await
            .is_none());
        server.abort();
        let (url, server) = serve(
            "200 OK",
            &" ".repeat(MAX_MANIFEST_BYTES + 1),
            Duration::ZERO,
        )
        .await;
        assert!(check_manifest(&current, &url, CHECK_TIMEOUT)
            .await
            .is_none());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn invalid_and_local_builds_skip_network() {
        assert!(check("not a version").await.is_none());
        assert!(check("0.0.2-abcdef+local").await.is_none());
    }
}
