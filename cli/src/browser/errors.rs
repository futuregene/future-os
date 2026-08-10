//! Unified error classes for browser operations — port of
//! `cli/src/browser/errors.ts`. Errors carry enough context for the CLI
//! facade to produce user-actionable messages without exposing protocol
//! internals.

/// `BrowserError` — base error with a machine code.
#[derive(Debug, Clone)]
pub struct BrowserError {
    pub message: String,
    pub code: &'static str,
}

impl std::fmt::Display for BrowserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for BrowserError {}

impl From<BrowserError> for String {
    fn from(e: BrowserError) -> String {
        e.to_string()
    }
}

impl BrowserError {
    fn new(message: impl Into<String>, code: &'static str) -> Self {
        BrowserError {
            message: message.into(),
            code,
        }
    }
}

/// `InvalidBrowserConfigError`.
pub fn invalid_browser_config_error(message: impl Into<String>) -> BrowserError {
    BrowserError::new(
        format!("Invalid browser config: {}", message.into()),
        "invalid_config",
    )
}

/// `BrowserNotFoundError` — detail optional.
pub fn browser_not_found_error(detail: Option<&str>) -> BrowserError {
    let msg = match detail {
        Some(d) => format!("Browser not found: {d}"),
        None => "Could not find Chrome or Edge. Install Chrome, or pass executablePath to browser start."
            .to_string(),
    };
    BrowserError::new(msg, "browser_not_found")
}

/// `BrowserConnectionError`.
pub fn browser_connection_error(endpoint: &str, reason: &str) -> BrowserError {
    BrowserError::new(
        format!("Cannot connect to browser at {endpoint}: {reason}"),
        "browser_connection_error",
    )
}

/// `BrowserPermissionError` — carries the remedy command.
#[derive(Debug, Clone)]
pub struct BrowserPermissionError {
    pub error: BrowserError,
    pub remedy_command: String,
}

impl std::fmt::Display for BrowserPermissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.error.message)
    }
}

impl std::error::Error for BrowserPermissionError {}

/// `SelectorError` — ref/selector resolution failures.
#[derive(Debug, Clone)]
pub struct SelectorError {
    pub error: BrowserError,
    pub selector: Option<String>,
}

impl SelectorError {
    pub fn new(message: impl Into<String>, code: &'static str, selector: Option<String>) -> Self {
        SelectorError {
            error: BrowserError::new(message, code),
            selector,
        }
    }
}

impl std::fmt::Display for SelectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.error.message)
    }
}

impl std::error::Error for SelectorError {}

/// `UnknownRefError`.
pub fn unknown_ref_error(r#ref: &str) -> SelectorError {
    SelectorError::new(
        format!("Unknown browser ref \"{ref}\". Run browser command snapshot first.", ref = r#ref),
        "unknown_ref",
        Some(r#ref.to_string()),
    )
}

/// `ElementNotFoundError`.
pub fn element_not_found_error(selector: &str) -> SelectorError {
    SelectorError::new(
        format!("Element not found: \"{selector}\""),
        "element_not_found",
        Some(selector.to_string()),
    )
}

/// `ElementNotInteractableError`.
pub fn element_not_interactable_error(selector: &str, reason: &str) -> SelectorError {
    SelectorError::new(
        format!("Element not interactable: \"{selector}\" — {reason}"),
        "element_not_interactable",
        Some(selector.to_string()),
    )
}

/// `StrictModeViolationError`.
pub fn strict_mode_violation_error(selector: &str, count: usize) -> SelectorError {
    SelectorError::new(
        format!(
            "Strict mode violation: \"{selector}\" resolved to {count} elements. Use a more specific selector."
        ),
        "strict_mode_violation",
        Some(selector.to_string()),
    )
}

/// `OperationTimeoutError`.
pub fn operation_timeout_error(
    operation: &str,
    timeout_ms: u64,
    context: Option<&str>,
) -> BrowserError {
    let extra = context.map(|c| format!(" ({c})")).unwrap_or_default();
    BrowserError::new(
        format!("Timed out after {timeout_ms}ms waiting for {operation}{extra}"),
        "operation_timeout",
    )
}

/// `UnsupportedCapabilityError`.
pub fn unsupported_capability_error(
    browser_kind: &str,
    operation: &str,
    alternative: Option<&str>,
) -> BrowserError {
    let extra = alternative
        .map(|a| format!("\nAlternative: {a}"))
        .unwrap_or_default();
    BrowserError::new(
        format!("{operation} is not supported on {browser_kind}.{extra}"),
        "unsupported_capability",
    )
}

/// `BrowserClosedError`.
pub fn browser_closed_error(page_id: Option<&str>) -> BrowserError {
    let detail = page_id.map(|p| format!(" (page: {p})")).unwrap_or_default();
    BrowserError::new(
        format!("Browser or page was closed during operation{detail}"),
        "browser_closed",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_error_display_and_conversion() {
        let err = invalid_browser_config_error("bad kind");
        assert_eq!(err.code, "invalid_config");
        assert_eq!(err.to_string(), "Invalid browser config: bad kind");
        // `std::error::Error` is implemented (source is None).
        assert!(std::error::Error::source(&err).is_none());
        // From<BrowserError> for String.
        let s: String = err.into();
        assert_eq!(s, "Invalid browser config: bad kind");
    }

    #[test]
    fn browser_not_found_with_and_without_detail() {
        let with = browser_not_found_error(Some("/opt/chrome"));
        assert_eq!(with.code, "browser_not_found");
        assert_eq!(with.to_string(), "Browser not found: /opt/chrome");
        let without = browser_not_found_error(None);
        assert!(without.to_string().starts_with("Could not find Chrome or Edge."));
    }

    #[test]
    fn connection_error_message() {
        let err = browser_connection_error("localhost:9222", "refused");
        assert_eq!(err.code, "browser_connection_error");
        assert_eq!(
            err.to_string(),
            "Cannot connect to browser at localhost:9222: refused"
        );
    }

    #[test]
    fn permission_error_display() {
        let err = BrowserPermissionError {
            error: BrowserError::new("denied", "browser_permission"),
            remedy_command: "safaridriver --enable".to_string(),
        };
        assert_eq!(err.to_string(), "denied");
        assert_eq!(err.remedy_command, "safaridriver --enable");
        assert!(std::error::Error::source(&err).is_none());
    }

    #[test]
    fn selector_error_variants() {
        let unknown = unknown_ref_error("e5");
        assert_eq!(unknown.error.code, "unknown_ref");
        assert_eq!(unknown.selector.as_deref(), Some("e5"));
        assert_eq!(
            unknown.to_string(),
            "Unknown browser ref \"e5\". Run browser command snapshot first."
        );
        assert!(std::error::Error::source(&unknown).is_none());

        let not_found = element_not_found_error("#btn");
        assert_eq!(not_found.error.code, "element_not_found");
        assert_eq!(not_found.to_string(), "Element not found: \"#btn\"");

        let not_interactable = element_not_interactable_error("#btn", "covered");
        assert_eq!(not_interactable.error.code, "element_not_interactable");
        assert_eq!(
            not_interactable.to_string(),
            "Element not interactable: \"#btn\" — covered"
        );

        let strict = strict_mode_violation_error(".item", 3);
        assert_eq!(strict.error.code, "strict_mode_violation");
        assert_eq!(
            strict.to_string(),
            "Strict mode violation: \".item\" resolved to 3 elements. Use a more specific selector."
        );
    }

    #[test]
    fn timeout_error_with_and_without_context() {
        let plain = operation_timeout_error("navigation", 5000, None);
        assert_eq!(plain.code, "operation_timeout");
        assert_eq!(plain.to_string(), "Timed out after 5000ms waiting for navigation");
        let ctx = operation_timeout_error("selector", 100, Some("iframe"));
        assert_eq!(
            ctx.to_string(),
            "Timed out after 100ms waiting for selector (iframe)"
        );
    }

    #[test]
    fn unsupported_capability_with_and_without_alternative() {
        let plain = unsupported_capability_error("safari", "PDF export", None);
        assert_eq!(plain.code, "unsupported_capability");
        assert_eq!(plain.to_string(), "PDF export is not supported on safari.");
        let alt = unsupported_capability_error("safari", "PDF export", Some("use chromium"));
        assert_eq!(
            alt.to_string(),
            "PDF export is not supported on safari.\nAlternative: use chromium"
        );
    }

    #[test]
    fn browser_closed_with_and_without_page() {
        let plain = browser_closed_error(None);
        assert_eq!(plain.code, "browser_closed");
        assert_eq!(
            plain.to_string(),
            "Browser or page was closed during operation"
        );
        let paged = browser_closed_error(Some("p1"));
        assert_eq!(
            paged.to_string(),
            "Browser or page was closed during operation (page: p1)"
        );
    }
}
