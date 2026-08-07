//! Browser execution subsystem — 1:1 port of `cli/src/browser/`.
//!
//! Modules:
//! - `backend` — session interfaces, deadline, options
//! - `types` — BrowserConfig / connection config / timeouts
//! - `errors` — user-actionable error classes
//! - `browser_state` — config.json read/write + v1→v2 migration
//! - `discovery` — findBrowser(executablePath?)
//! - `tab_order` — cross-protocol tab order reconciliation
//! - `selector` — ref→selector resolution
//! - `input` — keyboard (parseKey) + mouse (centerOf)
//! - `screenshot_writer` — path resolution + file writing
//! - `windows_process` — Windows detached launcher
//! - `scripts` — injected page scripts (snapshot + console hook)
//! - `chromium` — CDP transport/connection/session/manager
//! - `safari` — W3C WebDriver client/manager/session

pub mod backend;
pub mod browser_state;
pub mod chromium;
pub mod discovery;
pub mod errors;
pub mod input;
pub mod safari;
pub mod screenshot_writer;
pub mod scripts;
pub mod selector;
pub mod tab_order;
pub mod types;
pub mod windows_process;

use backend::{BrowserSession, BrowserSessionParams};

/// `createDefaultSession(params)` — CDP → ChromiumSession, webdriver →
/// SafariSession.
pub fn create_default_session(
    params: BrowserSessionParams,
) -> Result<Box<dyn BrowserSession>, String> {
    if params.protocol() == "webdriver" {
        let session = safari::safari_session::SafariSession::new(params)?;
        return Ok(Box::new(session));
    }
    let session = chromium::chromium_session::ChromiumSession::new(params);
    Ok(Box::new(session))
}
