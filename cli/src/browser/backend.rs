//! Backend interfaces and types — port of `cli/src/browser/backend.ts`.

use crate::browser::types::{BrowserTimeouts, PageId};
use serde_json::Value;

// ── Deadline ────────────────────────────────────────────────────────

/// `createDeadline(timeoutMs)` — wall-clock deadline with JS semantics.
pub struct Deadline {
    start_ms: u64,
    timeout_ms: u64,
}

impl Deadline {
    pub fn new(timeout_ms: u64) -> Self {
        let start_ms = crate::utils::time::now_millis();
        Deadline {
            start_ms,
            timeout_ms,
        }
    }

    pub fn expired(&self) -> bool {
        crate::utils::time::now_millis().saturating_sub(self.start_ms) >= self.timeout_ms
    }

    pub fn elapsed_ms(&self) -> u64 {
        crate::utils::time::now_millis().saturating_sub(self.start_ms)
    }

    /// `remainingMs()` — clamped to 0.
    pub fn remaining_ms(&self) -> u64 {
        self.timeout_ms.saturating_sub(self.elapsed_ms())
    }
}

// ── Evaluate ────────────────────────────────────────────────────────

/// `EvaluateRequest`.
pub enum EvaluateRequest {
    Expression {
        expression: String,
    },
    Function {
        function_declaration: String,
        arguments: Vec<Value>,
    },
}

// ── Session params ──────────────────────────────────────────────────

/// `BrowserSessionParams` — discriminated by protocol.
#[derive(Clone)]
pub enum BrowserSessionParams {
    Cdp {
        browser_kind: String,
        endpoint: String,
        timeouts: BrowserTimeouts,
        active_page_id: Option<PageId>,
        init_tab_order: Option<Vec<PageId>>,
    },
    Webdriver {
        endpoint: String,
        session_id: String,
        timeouts: BrowserTimeouts,
        active_page_id: Option<PageId>,
    },
}

impl BrowserSessionParams {
    pub fn protocol(&self) -> &'static str {
        match self {
            BrowserSessionParams::Cdp { .. } => "cdp",
            BrowserSessionParams::Webdriver { .. } => "webdriver",
        }
    }

    pub fn endpoint(&self) -> &str {
        match self {
            BrowserSessionParams::Cdp { endpoint, .. } => endpoint,
            BrowserSessionParams::Webdriver { endpoint, .. } => endpoint,
        }
    }

    pub fn timeouts(&self) -> BrowserTimeouts {
        match self {
            BrowserSessionParams::Cdp { timeouts, .. } => *timeouts,
            BrowserSessionParams::Webdriver { timeouts, .. } => *timeouts,
        }
    }

    pub fn browser_kind(&self) -> String {
        match self {
            BrowserSessionParams::Cdp { browser_kind, .. } => browser_kind.clone(),
            BrowserSessionParams::Webdriver { .. } => "safari".to_string(),
        }
    }

    pub fn active_page_id(&self) -> Option<&PageId> {
        match self {
            BrowserSessionParams::Cdp { active_page_id, .. } => active_page_id.as_ref(),
            BrowserSessionParams::Webdriver { active_page_id, .. } => active_page_id.as_ref(),
        }
    }
}

// ── Options ─────────────────────────────────────────────────────────

/// `OpenPageOptions`.
#[derive(Debug, Default, Clone, Copy)]
pub struct OpenPageOptions {
    pub wait_until: Option<&'static str>,
}

/// `ClickOptions`.
#[derive(Debug, Default, Clone, Copy)]
pub struct ClickOptions {
    pub timeout_ms: Option<u64>,
}

/// `TypeOptions`.
#[derive(Debug, Default, Clone, Copy)]
pub struct TypeOptions {
    pub clear: Option<bool>,
    pub submit: Option<bool>,
    pub timeout_ms: Option<u64>,
}

/// `PressOptions`.
#[derive(Debug, Default, Clone, Copy)]
pub struct PressOptions {
    pub timeout_ms: Option<u64>,
}

/// `CaptureScreenshotOptions`.
#[derive(Debug, Clone, Copy)]
pub struct CaptureScreenshotOptions {
    pub full_page: bool,
    pub format: &'static str,
    pub quality: Option<u64>,
}

// ── Resolved target ─────────────────────────────────────────────────

/// `ResolvedTarget` — re-exported from `selector` (same shape as backend.ts).
pub use crate::browser::selector::ResolvedTarget;

// ── Internal result types ───────────────────────────────────────────

/// `InternalPageInfo`.
#[derive(Debug, Clone)]
pub struct InternalPageInfo {
    pub page_id: PageId,
    pub title: String,
    pub url: String,
}

/// `InternalTabInfo`.
#[derive(Debug, Clone)]
pub struct InternalTabInfo {
    pub page_id: PageId,
    pub index: usize,
    pub title: String,
    pub url: String,
    pub active: bool,
}

/// `InternalActionResult`.
#[derive(Debug, Clone)]
pub struct InternalActionResult {
    pub page_id: PageId,
    pub title: String,
    pub url: String,
    pub did_navigate: bool,
}

/// `InternalTypeResult`.
#[derive(Debug, Clone)]
pub struct InternalTypeResult {
    pub page_id: PageId,
    pub typed: String,
    pub submitted: bool,
}

// ── Tabs ────────────────────────────────────────────────────────────

/// `TabsAction`.
#[derive(Debug, Clone)]
pub enum TabsAction {
    List,
    New { url: Option<String> },
    Select { index: usize },
    Close { index: usize },
}

/// `InternalTabsResult`.
#[derive(Debug, Clone)]
pub enum InternalTabsResult {
    List {
        tabs: Vec<InternalTabInfo>,
    },
    New {
        page: InternalPageInfo,
        index: usize,
    },
    Select {
        page: InternalPageInfo,
    },
    Close {
        url: String,
        index: usize,
    },
}

// ── BrowserSession ──────────────────────────────────────────────────

/// `BrowserSession` — object-safe session interface (async-trait).
#[async_trait::async_trait]
pub trait BrowserSession: Send {
    fn kind(&self) -> &'static str;
    fn protocol(&self) -> &'static str;

    async fn open(
        &mut self,
        url: &str,
        options: OpenPageOptions,
    ) -> Result<InternalPageInfo, String>;

    async fn click(
        &mut self,
        target: &ResolvedTarget,
        options: ClickOptions,
    ) -> Result<InternalActionResult, String>;

    async fn r#type(
        &mut self,
        target: &ResolvedTarget,
        text: &str,
        options: TypeOptions,
    ) -> Result<InternalTypeResult, String>;

    async fn press(
        &mut self,
        key: &str,
        target: Option<&ResolvedTarget>,
        options: PressOptions,
    ) -> Result<InternalActionResult, String>;

    async fn tabs(&mut self, action: &TabsAction) -> Result<InternalTabsResult, String>;

    async fn evaluate(&mut self, request: &EvaluateRequest) -> Result<Value, String>;

    async fn capture_screenshot(
        &mut self,
        options: &CaptureScreenshotOptions,
    ) -> Result<Vec<u8>, String>;

    async fn disconnect(&mut self) -> Result<(), String>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::types::DEFAULT_TIMEOUTS;

    #[test]
    fn deadline_semantics() {
        // Zero timeout expires immediately.
        let expired = Deadline::new(0);
        assert!(expired.expired());
        assert_eq!(expired.remaining_ms(), 0);
        // Generous timeout: not expired, remaining clamps near the timeout.
        let pending = Deadline::new(60_000);
        assert!(!pending.expired());
        assert!(pending.remaining_ms() <= 60_000);
        assert!(pending.remaining_ms() > 59_000);
        assert!(pending.elapsed_ms() < 1_000);
    }

    #[test]
    fn session_params_accessors() {
        let cdp = BrowserSessionParams::Cdp {
            browser_kind: "chrome".to_string(),
            endpoint: "http://127.0.0.1:9222".to_string(),
            timeouts: DEFAULT_TIMEOUTS,
            active_page_id: Some("p1".to_string()),
            init_tab_order: Some(vec!["p1".to_string()]),
        };
        assert_eq!(cdp.protocol(), "cdp");
        assert_eq!(cdp.endpoint(), "http://127.0.0.1:9222");
        assert_eq!(cdp.browser_kind(), "chrome");
        assert_eq!(cdp.active_page_id().map(String::as_str), Some("p1"));
        assert_eq!(cdp.timeouts().action_timeout_ms, 5_000);

        let wd = BrowserSessionParams::Webdriver {
            endpoint: "http://127.0.0.1:4444".to_string(),
            session_id: "s1".to_string(),
            timeouts: DEFAULT_TIMEOUTS,
            active_page_id: None,
        };
        assert_eq!(wd.protocol(), "webdriver");
        assert_eq!(wd.endpoint(), "http://127.0.0.1:4444");
        // WebDriver params are always Safari.
        assert_eq!(wd.browser_kind(), "safari");
        assert!(wd.active_page_id().is_none());
        assert_eq!(wd.timeouts().navigation_timeout_ms, 15_000);
    }
}
