//! Chromium CDP backend — port of `cli/src/browser/chromium/`.

pub mod cdp_connection;
pub mod cdp_event_router;
pub mod cdp_transport;
pub mod chromium_console_hook;
pub mod chromium_endpoint;
pub mod chromium_manager;
pub mod chromium_navigation;
pub mod chromium_page;
pub mod chromium_screenshot;
pub mod chromium_session;
pub mod execution_context;
pub mod target_registry;
