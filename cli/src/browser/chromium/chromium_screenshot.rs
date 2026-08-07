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
