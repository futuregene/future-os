//! Core types for the browser execution layer — port of
//! `cli/src/browser/types.ts`.

use serde_json::{Map, Value};

/// `BrowserKind` — "chrome" | "edge" | "chromium" | "safari".
pub type BrowserKind = &'static str;

/// `BrowserProtocol` — "cdp" | "webdriver".
pub type BrowserProtocol = &'static str;

/// `PageId`.
pub type PageId = String;

/// `BrowserConnectionConfig` — discriminated by protocol.
#[derive(Debug, Clone)]
pub enum BrowserConnectionConfig {
    Cdp {
        browser_kind: String,
        endpoint: String,
    },
    Webdriver {
        browser_kind: String,
        endpoint: String,
        session_id: String,
        driver_pid: Option<i64>,
    },
}

impl BrowserConnectionConfig {
    pub fn protocol(&self) -> &'static str {
        match self {
            BrowserConnectionConfig::Cdp { .. } => "cdp",
            BrowserConnectionConfig::Webdriver { .. } => "webdriver",
        }
    }

    pub fn browser_kind(&self) -> &str {
        match self {
            BrowserConnectionConfig::Cdp { browser_kind, .. } => browser_kind,
            BrowserConnectionConfig::Webdriver { browser_kind, .. } => browser_kind,
        }
    }

    pub fn endpoint(&self) -> &str {
        match self {
            BrowserConnectionConfig::Cdp { endpoint, .. } => endpoint,
            BrowserConnectionConfig::Webdriver { endpoint, .. } => endpoint,
        }
    }

    pub fn session_id(&self) -> Option<&str> {
        match self {
            BrowserConnectionConfig::Cdp { .. } => None,
            BrowserConnectionConfig::Webdriver { session_id, .. } => Some(session_id),
        }
    }
}

/// `CURRENT_CONFIG_VERSION`.
pub const CURRENT_CONFIG_VERSION: i64 = 2;

impl Default for BrowserConnectionConfig {
    fn default() -> Self {
        BrowserConnectionConfig::Cdp {
            browser_kind: "chromium".to_string(),
            endpoint: "http://127.0.0.1:9222".to_string(),
        }
    }
}

/// `BrowserConfig`.
#[derive(Debug, Clone, Default)]
pub struct BrowserConfig {
    pub version: i64,
    pub connection: BrowserConnectionConfig,
    pub active_url: Option<String>,
    pub active_page_id: Option<PageId>,
    pub tab_order: Option<Vec<PageId>>,
    pub refs: Option<Map<String, Value>>,
    pub refs_page_id: Option<PageId>,
    pub refs_url: Option<String>,
}

/// `BrowserTimeouts` + `DEFAULT_TIMEOUTS`.
#[derive(Debug, Clone, Copy)]
pub struct BrowserTimeouts {
    pub action_timeout_ms: u64,
    pub navigation_timeout_ms: u64,
}

pub const DEFAULT_TIMEOUTS: BrowserTimeouts = BrowserTimeouts {
    action_timeout_ms: 5_000,
    navigation_timeout_ms: 15_000,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_config_accessors() {
        let cdp = BrowserConnectionConfig::Cdp {
            browser_kind: "edge".to_string(),
            endpoint: "http://localhost:9222".to_string(),
        };
        assert_eq!(cdp.protocol(), "cdp");
        assert_eq!(cdp.browser_kind(), "edge");
        assert_eq!(cdp.endpoint(), "http://localhost:9222");
        assert!(cdp.session_id().is_none());

        let wd = BrowserConnectionConfig::Webdriver {
            browser_kind: "safari".to_string(),
            endpoint: "http://localhost:4444".to_string(),
            session_id: "abc".to_string(),
            driver_pid: Some(1234),
        };
        assert_eq!(wd.protocol(), "webdriver");
        assert_eq!(wd.browser_kind(), "safari");
        assert_eq!(wd.endpoint(), "http://localhost:4444");
        assert_eq!(wd.session_id(), Some("abc"));
    }

    #[test]
    fn default_connection_is_cdp_chromium() {
        let config = BrowserConnectionConfig::default();
        assert_eq!(config.protocol(), "cdp");
        assert_eq!(config.browser_kind(), "chromium");
        assert_eq!(config.endpoint(), "http://127.0.0.1:9222");
        assert_eq!(CURRENT_CONFIG_VERSION, 2);
    }
}
