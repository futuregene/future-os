//! App metadata, window chrome and store lifecycle Tauri commands.

use serde::Serialize;

use crate::{build_info, store};

/// Version + release/dev channel, so the frontend can show a "test build" hint
/// and gate the environment switcher (test-only). `isRelease` is derived from
/// the version by the backend so the rule lives in exactly one place.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildInfo {
    pub version: String,
    pub is_release: bool,
}

#[tauri::command]
pub fn app_build_info() -> BuildInfo {
    BuildInfo {
        version: build_info::VERSION.to_string(),
        is_release: build_info::is_release(),
    }
}

#[tauri::command]
pub fn initialize_app_store() -> Result<(), crate::AppError> {
    store::initialize_app_store()
}

/// Paint the Windows caption in the colors the webview just rendered.
///
/// The frame belongs to DWM, and the palette is a CSS decision, so the frontend
/// hands over the computed `--color-surface` / `--color-ink` values after every
/// theme or palette change; see [`crate::window_caption`]. Other platforms draw
/// their own chrome and ignore the call, which keeps the invoke site
/// platform-agnostic.
#[tauri::command]
#[cfg_attr(not(target_os = "windows"), allow(unused_variables))]
pub fn set_window_caption_colors(
    app: tauri::AppHandle,
    caption: String,
    text: String,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        crate::window_caption::apply(&app, &caption, &text)
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(())
    }
}

/// Register the language the window is showing.
///
/// Native dialogs raised from Rust outside a command — today the quit
/// confirmation — have no invoke of their own to carry a language tag, so the
/// frontend pushes the language it resolved here on start and on every change;
/// see [`crate::native_dialog`].
#[tauri::command]
pub fn set_ui_language(language: String) -> Result<(), String> {
    crate::native_dialog::set_language(&language);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_info_mirrors_the_crate_constant() {
        let info = app_build_info();
        assert_eq!(info.version, crate::build_info::VERSION);
        assert_eq!(info.is_release, crate::build_info::is_release());
    }

    #[test]
    fn initialize_creates_a_fresh_store() {
        let _home = crate::auth_store::test_support::HomeGuard::new("cmd_app_init");
        initialize_app_store().expect("initialize the store");
    }

    /// The command is the only path from the window's resolved language to the
    /// dialogs Rust raises on its own: a registration that did not reach the
    /// registry would leave the quit confirmation in the previous language.
    #[test]
    fn the_ui_language_command_reaches_the_dialog_registry() {
        let _serialized = crate::native_dialog::language_test_guard();
        set_ui_language("zh-CN".into()).expect("record a Chinese tag");
        assert_eq!(
            crate::native_dialog::locale(),
            crate::native_dialog::Locale::Zh
        );
        set_ui_language("en-US".into()).expect("record an English tag");
        assert_eq!(
            crate::native_dialog::locale(),
            crate::native_dialog::Locale::En
        );
    }
}
