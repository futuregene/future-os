//! Retrying HTTP calls for channel providers.
//!
//! Two things every provider needs and nobody wants to write twice: honoring a
//! platform's rate limit instead of hammering it, and telling an error that
//! will never succeed (a blocked bot, a deleted channel) apart from one that
//! might. [`ErrorClass`] is the shared vocabulary for the second, and the
//! delivery queue uses it to decide whether to retry a message forever.

use anyhow::{anyhow, Result};
use serde_json::Value;
use std::time::Duration;

/// Whether retrying could ever help.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// Retrying is pointless: the credentials, the target, or the payload is
    /// wrong, or the platform has banned this sender.
    Permanent,
    /// The call may succeed later (rate limit, gateway error, timeout).
    Transient,
}

/// How hard to try before giving up on one HTTP call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(8),
        }
    }
}

impl RetryPolicy {
    /// One attempt, no retry — for calls where a failure is the caller's answer
    /// (a health probe, a best-effort typing indicator).
    pub fn single_attempt() -> Self {
        Self {
            max_attempts: 1,
            ..Self::default()
        }
    }
}

/// One HTTP exchange, with the body kept both parsed and raw.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub text: String,
    pub body: Value,
    /// Response headers with lowercased names, for the platform-specific
    /// metadata a body does not carry (rate-limit buckets, request ids,
    /// pagination cursors).
    pub headers: std::collections::HashMap<String, String>,
    /// `Retry-After` in seconds, when the platform sent one.
    pub retry_after: Option<Duration>,
}

impl HttpResponse {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// One response header, matched case-insensitively.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    pub fn class(&self) -> ErrorClass {
        classify_status(self.status)
    }

    /// The most useful message the platform offered.
    ///
    /// Cross-platform JSON error shapes disagree (`error`, `message`, `msg`,
    /// `description`, `errmsg`, and nested `error.message`), so we look for all
    /// of them before falling back to the raw body.
    pub fn error_message(&self) -> String {
        let candidates = [
            self.body.get("message"),
            self.body.get("msg"),
            self.body.get("errmsg"),
            self.body.get("description"),
            self.body.get("detail"),
            self.body.get("error"),
            self.body
                .get("error")
                .and_then(|error| error.get("message")),
            self.body
                .get("response")
                .and_then(|value| value.get("message")),
        ];
        let text = candidates
            .iter()
            .flatten()
            .find_map(|value| value.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| {
                crate::transport::truncate(&self.text, 300, crate::transport::LengthUnit::Chars)
            });
        format!("HTTP {}: {text}", self.status)
    }
}

/// Status codes worth a second attempt.
pub fn is_retryable(status: u16) -> bool {
    status == 408 || status == 425 || status == 429 || (500..600).contains(&status)
}

/// Classify an HTTP status.
pub fn classify_status(status: u16) -> ErrorClass {
    if is_retryable(status) {
        ErrorClass::Transient
    } else {
        ErrorClass::Permanent
    }
}

/// Parse `Retry-After` (delta-seconds or an HTTP date we treat as unknown).
pub fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let raw = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    let seconds: u64 = raw.trim().parse().ok()?;
    Some(Duration::from_secs(seconds.min(600)))
}

/// Delay before attempt `attempt` (1-based), preferring the platform's own
/// `Retry-After` over our exponential guess.
pub fn backoff_delay(attempt: u32, policy: RetryPolicy, retry_after: Option<Duration>) -> Duration {
    if let Some(wait) = retry_after {
        return wait.min(policy.max_delay);
    }
    let exponent = attempt.saturating_sub(1).min(6);
    policy
        .base_delay
        .saturating_mul(1u32 << exponent)
        .min(policy.max_delay)
}

/// Send one JSON request, retrying transient failures.
///
/// `headers` are applied to every attempt. A body is only sent when `body` is
/// `Some`, so the same helper serves GET and POST.
pub async fn send_json(
    client: &reqwest::Client,
    method: reqwest::Method,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<&Value>,
    policy: RetryPolicy,
) -> Result<HttpResponse> {
    let mut last_error: Option<String> = None;
    let attempts = policy.max_attempts.max(1);
    for attempt in 1..=attempts {
        let mut request = client.request(method.clone(), url);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        if let Some(payload) = body {
            request = request.json(payload);
        }
        let response = match request.send().await {
            Ok(response) => response,
            Err(error) => {
                // Connection-level failures are transient by definition.
                last_error = Some(format!("request failed: {error}"));
                if attempt < attempts {
                    let wait = backoff_delay(attempt, policy, None);
                    tracing::debug!(url, attempt, ?wait, %error, "retrying channel HTTP call");
                    tokio::time::sleep(wait).await;
                }
                continue;
            }
        };

        let status = response.status().as_u16();
        let retry_after = parse_retry_after(response.headers());
        let headers: std::collections::HashMap<String, String> = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_ascii_lowercase(), value.to_string()))
            })
            .collect();
        let text = response.text().await.unwrap_or_default();
        let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let outcome = HttpResponse {
            status,
            text,
            body: parsed,
            headers,
            retry_after,
        };

        if outcome.is_success() || !is_retryable(status) || attempt == attempts {
            return Ok(outcome);
        }
        last_error = Some(outcome.error_message());
        let wait = backoff_delay(attempt, policy, outcome.retry_after);
        tracing::debug!(url, attempt, ?wait, status, "retrying channel HTTP call");
        tokio::time::sleep(wait).await;
    }
    Err(anyhow!(
        "channel HTTP call to {url} failed after {attempts} attempts: {}",
        last_error.unwrap_or_else(|| "unknown error".into())
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(status: u16, text: &str) -> HttpResponse {
        HttpResponse {
            status,
            text: text.to_string(),
            body: serde_json::from_str(text).unwrap_or(Value::Null),
            headers: std::collections::HashMap::new(),
            retry_after: None,
        }
    }

    #[test]
    fn headers_are_matched_case_insensitively() {
        let mut response = response(200, "{}");
        response
            .headers
            .insert("x-ratelimit-remaining".to_string(), "3".to_string());
        assert_eq!(response.header("X-RateLimit-Remaining"), Some("3"));
        assert_eq!(response.header("x-ratelimit-remaining"), Some("3"));
        assert_eq!(response.header("missing"), None);
    }

    #[test]
    fn retryable_statuses_cover_rate_limits_and_gateways() {
        for status in [408, 425, 429, 500, 502, 503, 504] {
            assert!(is_retryable(status), "{status} should be retryable");
            assert_eq!(classify_status(status), ErrorClass::Transient);
        }
        for status in [400, 401, 403, 404, 409, 410, 413] {
            assert!(!is_retryable(status), "{status} should not be retryable");
            assert_eq!(classify_status(status), ErrorClass::Permanent);
        }
    }

    #[test]
    fn backoff_grows_and_is_capped() {
        let policy = RetryPolicy {
            max_attempts: 6,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(2),
        };
        assert_eq!(backoff_delay(1, policy, None), Duration::from_millis(100));
        assert_eq!(backoff_delay(2, policy, None), Duration::from_millis(200));
        assert_eq!(backoff_delay(3, policy, None), Duration::from_millis(400));
        assert_eq!(backoff_delay(9, policy, None), Duration::from_secs(2));
    }

    #[test]
    fn retry_after_wins_over_the_exponential_guess_but_is_capped() {
        let policy = RetryPolicy::default();
        assert_eq!(
            backoff_delay(1, policy, Some(Duration::from_secs(3))),
            Duration::from_secs(3)
        );
        assert_eq!(
            backoff_delay(1, policy, Some(Duration::from_secs(9999))),
            policy.max_delay
        );
    }

    #[test]
    fn error_message_reads_the_variants_platforms_use() {
        assert_eq!(
            response(400, r#"{"message":"bad token"}"#).error_message(),
            "HTTP 400: bad token"
        );
        assert_eq!(
            response(400, r#"{"msg":"bad token"}"#).error_message(),
            "HTTP 400: bad token"
        );
        assert_eq!(
            response(400, r#"{"errmsg":"bad token"}"#).error_message(),
            "HTTP 400: bad token"
        );
        assert_eq!(
            response(400, r#"{"error":{"message":"bad token"}}"#).error_message(),
            "HTTP 400: bad token"
        );
        assert_eq!(
            response(400, r#"{"description":"bad token"}"#).error_message(),
            "HTTP 400: bad token"
        );
    }

    #[test]
    fn error_message_falls_back_to_the_truncated_body() {
        let long = "x".repeat(1000);
        let message = response(503, &long).error_message();
        assert!(message.starts_with("HTTP 503: "));
        assert!(message.len() < 400, "{}", message.len());
    }

    #[test]
    fn success_covers_the_whole_2xx_range() {
        assert!(response(200, "{}").is_success());
        assert!(response(204, "").is_success());
        assert!(!response(301, "").is_success());
    }
}
