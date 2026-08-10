//! Chromium screenshot — port of
//! `cli/src/browser/chromium/chromium-screenshot.ts`.
//!
//! Captures via Page.captureScreenshot; full-page uses
//! Page.getLayoutMetrics cssContentSize as the clip.

use super::cdp_connection::CdpSession;
use crate::browser::backend::CaptureScreenshotOptions;
use base64::Engine;
use serde_json::{json, Value};

/// `captureScreenshot(session, options)` → raw image bytes (decoded base64).
pub async fn capture_screenshot(
    session: &CdpSession,
    options: &CaptureScreenshotOptions,
) -> Result<Vec<u8>, String> {
    let mut params = serde_json::Map::new();
    params.insert(
        "format".to_string(),
        Value::String(options.format.to_string()),
    );
    if let Some(quality) = options.quality {
        params.insert("quality".to_string(), json!(quality));
    }

    if options.full_page {
        params.insert("captureBeyondViewport".to_string(), Value::Bool(true));

        // Get full page dimensions
        let metrics = session
            .send("Page.getLayoutMetrics", None)
            .await
            .map_err(|e| e.to_string())?;
        let css_content_size = metrics
            .get("cssContentSize")
            .cloned()
            .unwrap_or(Value::Null);

        // Set clip to full content size
        params.insert(
            "clip".to_string(),
            json!({
                "x": 0,
                "y": 0,
                "width": css_content_size.get("width").and_then(Value::as_f64).unwrap_or(0.0),
                "height": css_content_size.get("height").and_then(Value::as_f64).unwrap_or(0.0),
                "scale": 1,
            }),
        );
    }

    let result = session
        .send("Page.captureScreenshot", Some(&params))
        .await
        .map_err(|e| e.to_string())?;
    let data = result.get("data").and_then(Value::as_str).unwrap_or("");

    // Decode base64 to bytes.
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|e| format!("Failed to decode screenshot: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::chromium::cdp_connection::CdpConnection;
    use crate::test_cdp::MockCdp;

    async fn session_over(mock: &MockCdp) -> (std::sync::Arc<CdpConnection>, CdpSession) {
        let conn = CdpConnection::connect(&mock.ws_url, 5_000).await.unwrap();
        let session = CdpSession::new("S-1", conn.clone());
        (conn, session)
    }

    fn opts(full_page: bool) -> CaptureScreenshotOptions {
        CaptureScreenshotOptions {
            full_page,
            format: "png",
            quality: None,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn simple_capture_decodes_base64() {
        let mock = MockCdp::start().await;
        let (conn, session) = session_over(&mock).await;
        let bytes = capture_screenshot(&session, &opts(false)).await.unwrap();
        assert_eq!(bytes, b"\x89PNG-mock".to_vec());
        let shots = mock.commands_of("Page.captureScreenshot");
        assert_eq!(shots.len(), 1);
        assert_eq!(shots[0]["format"], json!("png"));
        assert!(shots[0].get("quality").is_none());
        assert!(shots[0].get("clip").is_none());
        assert!(mock.commands_of("Page.getLayoutMetrics").is_empty());
        conn.disconnect().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn full_page_uses_layout_metrics_clip() {
        let mock = MockCdp::start().await;
        let (conn, session) = session_over(&mock).await;
        let bytes = capture_screenshot(&session, &opts(true)).await.unwrap();
        assert_eq!(bytes, b"\x89PNG-mock".to_vec());
        let shots = mock.commands_of("Page.captureScreenshot");
        assert_eq!(shots[0]["captureBeyondViewport"], json!(true));
        assert_eq!(shots[0]["clip"]["width"], json!(800.0));
        assert_eq!(shots[0]["clip"]["height"], json!(600.0));
        assert_eq!(shots[0]["clip"]["scale"], json!(1));

        // Missing cssContentSize → zero-size clip defaults.
        mock.state.lock().unwrap().layout_metrics = json!({});
        capture_screenshot(&session, &opts(true)).await.unwrap();
        let shots = mock.commands_of("Page.captureScreenshot");
        assert_eq!(shots[1]["clip"]["width"], json!(0.0));
        conn.disconnect().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn quality_param_and_failure_paths() {
        let mock = MockCdp::start().await;
        let (conn, session) = session_over(&mock).await;
        let mut with_quality = opts(false);
        with_quality.quality = Some(70);
        capture_screenshot(&session, &with_quality).await.unwrap();
        let shots = mock.commands_of("Page.captureScreenshot");
        assert_eq!(shots[0]["quality"], json!(70));

        // Invalid base64 → decode error.
        mock.state.lock().unwrap().screenshot_b64 = "!!!bad!!!".to_string();
        let err = capture_screenshot(&session, &opts(false))
            .await
            .unwrap_err();
        assert!(err.contains("Failed to decode screenshot"), "{err}");

        // getLayoutMetrics failure (full page path).
        mock.state.lock().unwrap().screenshot_b64 =
            base64::engine::general_purpose::STANDARD.encode(b"x");
        mock.state
            .lock()
            .unwrap()
            .fail_methods
            .insert("Page.getLayoutMetrics".to_string());
        let err = capture_screenshot(&session, &opts(true)).await.unwrap_err();
        assert!(err.contains("mock failure"), "{err}");

        // captureScreenshot failure (simple path).
        mock.state
            .lock()
            .unwrap()
            .fail_methods
            .insert("Page.captureScreenshot".to_string());
        let err = capture_screenshot(&session, &opts(false))
            .await
            .unwrap_err();
        assert!(err.contains("mock failure"), "{err}");
        conn.disconnect().await;
    }
}
