//! Native application menu (macOS only).
//!
//! Tauri auto-generates a default menu on macOS whose "About/Hide/Quit" items
//! take the app name from the bundle — or, in dev and unbundled runs, from the
//! lowercase executable name (`futureos`). We build the menu explicitly so the
//! brand name always reads "FutureOS", and so we can add the macOS-only items the
//! rest of the app needs: "About FutureOS" (opens the in-app About page),
//! "Restart Webview" (reloads a hung/crashed webview in place without
//! relaunching), and the two entry points of the File menu — New Conversation and
//! Settings.
//!
//! **No item carries an accelerator, deliberately.** Every one of these actions is
//! a *command* in the webview's table (`features/keyboard/catalog.ts`), and a user
//! may rebind it. An accelerator written into the menu would keep printing the
//! old key after the rebinding — a menu item that names ⌘N while ⌘N does nothing
//! is worse than one that names no key at all. The menu item emits an event; the
//! window runs the command, with whatever key the user gave it.
//!
//! Windows/Linux keep Tauri's default behaviour (no window menu, no About), so
//! there is nothing to build for them.

use tauri::{
    menu::{Menu, MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder},
    Runtime,
};

/// Menu item id for "About FutureOS" → opens Settings.
pub const MENU_ABOUT: &str = "about";
/// Menu item id for the "Restart Webview" recovery action.
pub const MENU_RESTART_WEBVIEW: &str = "restart-webview";
/// Menu item id for "New Conversation" → the `chat.new` command.
pub const MENU_NEW_CONVERSATION: &str = "new-conversation";
/// Menu item id for "Settings…" → the `settings.open` command.
pub const MENU_SETTINGS: &str = "settings-general";

const APP_NAME: &str = "FutureOS";

/// Build the full macOS menu bar with correct "FutureOS" naming.
pub fn build_macos_menu<R: Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<Menu<R>> {
    let about = MenuItemBuilder::with_id(MENU_ABOUT, format!("About {APP_NAME}")).build(app)?;
    let restart_webview =
        MenuItemBuilder::with_id(MENU_RESTART_WEBVIEW, "Restart Webview").build(app)?;
    // A "File" menu of the app's own two entry points. `with_id` only — no
    // accelerator, for the reason in the module comment.
    let new_conversation =
        MenuItemBuilder::with_id(MENU_NEW_CONVERSATION, "New Conversation").build(app)?;
    let settings = MenuItemBuilder::with_id(MENU_SETTINGS, "Settings…").build(app)?;

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

    let file_menu = SubmenuBuilder::new(app, "File")
        .item(&new_conversation)
        .separator()
        .item(&settings)
        .separator()
        .item(&PredefinedMenuItem::close_window(app, None)?)
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
        .item(&file_menu)
        .item(&edit_menu)
        .item(&view_menu)
        .item(&window_menu)
        .build()
}

// NOTE: build_macos_menu cannot run in unit tests — muda requires menu
// construction on the main thread (test threads are workers). Documented in
// coverage/baseline-100/acceptance-waivers.md as W-menu.
