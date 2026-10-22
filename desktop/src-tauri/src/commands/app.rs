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
}
