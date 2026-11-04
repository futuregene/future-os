//! Desktop edition discovery. Only an undecided installation contacts the
//! license endpoint; explicit community and commercial choices are sticky.

use crate::AppError;
use future_app_settings::{read_community_edition, write_value, KEY_COMMUNITY_EDITION};
use std::time::Duration;

const LICENSE_URL: &str = "https://dl.future-os.cn/license";

pub(crate) async fn refresh() {
    if let Err(error) = refresh_from(LICENSE_URL).await {
        tracing::warn!(event = "desktop_license", %error, "Desktop edition check failed");
    }
}

async fn refresh_from(url: &str) -> Result<(), AppError> {
    let edition = tokio::task::spawn_blocking(|| {
        let conn = future_app_settings::connect()?;
        read_community_edition(&conn)
    })
    .await
    .map_err(|error| AppError::from(format!("Desktop edition read failed: {error}")))??;
    if edition.is_some() {
        return Ok(());
    }

    // Community responses (including failures) leave the stored state null so
    // a later launch can retry. No window or Agent startup waits on the network.
    if !fetch_commercial(url).await {
        return Ok(());
    }

    let changed = tokio::task::spawn_blocking(|| {
        let mut conn = future_app_settings::connect()?;
        record_commercial(&mut conn, chrono::Utc::now().timestamp_millis())
    })
    .await
    .map_err(|error| AppError::from(format!("Desktop edition write failed: {error}")))??;
    if changed {
        future_host::agent_events::publish_invalidation("app_settings_changed");
    }
    Ok(())
}

async fn fetch_commercial(url: &str) -> bool {
    let result = async {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(5))
            .build()?;
        let response = client.get(url).send().await?;
        if !response.status().is_success() {
            return Ok::<_, reqwest::Error>(false);
        }
        let body = response.text().await?;
        let body = body.trim();
        Ok(!body.is_empty() && body != "community")
    }
    .await;
    result.unwrap_or(false)
}

fn record_commercial(conn: &mut rusqlite::Connection, now: i64) -> Result<bool, AppError> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    // The user may have manually selected community while the request was in
    // flight. Recheck under the write lock so that selection always wins.
    let changed = read_community_edition(&tx)?.is_none();
    if changed {
        write_value(&tx, KEY_COMMUNITY_EDITION, "false", now)?;
    }
    tx.commit()?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[tokio::test]
    async fn http_results_follow_the_license_contract() {
        crate::install_rustls_provider();
        for (status, body, commercial) in [
            ("200 OK", "commercial", true),
            ("200 OK", "some other content", true),
            ("200 OK", " community\r\n", false),
            ("200 OK", "", false),
            ("200 OK", " \r\n\t", false),
            ("204 No Content", "", false),
            ("404 Not Found", "commercial", false),
            ("502 Bad Gateway", "commercial", false),
        ] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/license", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = [0; 4096];
                assert!(stream.read(&mut request).unwrap() > 0);
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            });
            assert_eq!(
                fetch_commercial(&url).await,
                commercial,
                "{status}: {body:?}"
            );
            server.join().unwrap();
        }
        assert!(!fetch_commercial("invalid url").await);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/license", listener.local_addr().unwrap());
        drop(listener);
        assert!(!fetch_commercial(&url).await);
    }

    #[test]
    fn commercial_detection_never_overwrites_a_manual_choice() {
        for initial in [None, Some("null"), Some("true"), Some("false")] {
            let mut conn = rusqlite::Connection::open_in_memory().unwrap();
            future_app_settings::ensure_table(&conn).unwrap();
            if let Some(value) = initial {
                write_value(&conn, KEY_COMMUNITY_EDITION, value, 1).unwrap();
            }
            let expected_change = matches!(initial, None | Some("null"));
            assert_eq!(record_commercial(&mut conn, 2).unwrap(), expected_change);
            assert_eq!(
                read_community_edition(&conn).unwrap(),
                Some(initial == Some("true"))
            );
        }
    }

    #[test]
    fn explicit_editions_do_not_send_a_request() {
        let _home = crate::auth_store::test_support::HomeGuard::new("license_explicit");
        let conn = future_app_settings::connect().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/license", listener.local_addr().unwrap());
        let runtime = tokio::runtime::Runtime::new().unwrap();
        for value in ["true", "false"] {
            write_value(&conn, KEY_COMMUNITY_EDITION, value, 1).unwrap();
            runtime.block_on(refresh_from(&url)).unwrap();
            assert_eq!(
                listener.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
            assert_eq!(
                read_community_edition(&conn).unwrap(),
                Some(value == "true")
            );
        }
    }
}
