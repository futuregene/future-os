//! Native application menu (macOS only).
//!
//! Tauri auto-generates a default menu on macOS whose "About/Hide/Quit" items
//! take the app name from the bundle — or, in dev and unbundled runs, from the
//! lowercase executable name (`futureos`). We build the menu explicitly so the
//! brand name always reads "FutureOS", and so we can add two macOS-only items:
//! "About FutureOS" (opens the in-app About page) and "Restart Webview" (a
//! debug escape hatch to reload a hung/crashed webview without relaunching).
//!
//! Windows/Linux keep Tauri's default behaviour (no window menu, no About), so
//! there is nothing to build for them.

use tauri::{
    menu::{Menu, MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder},
    AppHandle, Wry,
};

/// Menu item id for "About FutureOS" → opens Settings.
pub const MENU_ABOUT: &str = "about";
/// Menu item id for the "Restart Webview" debug action.
pub const MENU_RESTART_WEBVIEW: &str = "restart-webview";

const APP_NAME: &str = "FutureOS";

/// Build the full macOS menu bar with correct "FutureOS" naming.
pub fn build_macos_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let about = MenuItemBuilder::with_id(MENU_ABOUT, format!("About {APP_NAME}")).build(app)?;
    let restart_webview =
        MenuItemBuilder::with_id(MENU_RESTART_WEBVIEW, "Restart Webview").build(app)?;

    let app_menu = SubmenuBuilder::new(app, APP_NAME)
        .item(&about)
        .separator()
        .item(&restart_webview)
        .separator()
        .item(&PredefinedMenuItem::services(app, None)?)
        .separator()
        .item(&PredefinedMenuItem::hide(
            app,
            Some(&format!("Hide {APP_NAME}")),
        )?)
        .item(&PredefinedMenuItem::hide_others(app, Some("Hide Others"))?)
        .item(&PredefinedMenuItem::show_all(app, Some("Show All"))?)
        .separator()
        .item(&PredefinedMenuItem::quit(
            app,
            Some(&format!("Quit {APP_NAME}")),
        )?)
        .build()?;

    let edit_menu = SubmenuBuilder::new(app, "Edit")
        .item(&PredefinedMenuItem::undo(app, None)?)
        .item(&PredefinedMenuItem::redo(app, None)?)
        .separator()
        .item(&PredefinedMenuItem::cut(app, None)?)
        .item(&PredefinedMenuItem::copy(app, None)?)
        .item(&PredefinedMenuItem::paste(app, None)?)
        .item(&PredefinedMenuItem::select_all(app, None)?)
        .build()?;

    let view_menu = SubmenuBuilder::new(app, "View")
        .item(&PredefinedMenuItem::fullscreen(app, None)?)
        .build()?;

    let window_menu = SubmenuBuilder::new(app, "Window")
        .item(&PredefinedMenuItem::minimize(app, None)?)
        .item(&PredefinedMenuItem::maximize(app, None)?)
        .separator()
        .item(&PredefinedMenuItem::close_window(app, None)?)
        .build()?;

    MenuBuilder::new(app)
        .item(&app_menu)
        .item(&edit_menu)
        .item(&view_menu)
        .item(&window_menu)
        .build()
}
