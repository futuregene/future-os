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
