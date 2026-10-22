//! Paint the Windows title bar in the page's own colors.
//!
//! Windows draws the frame itself, so a decorated window gets DWM's caption
//! color — the system light/dark gray, tinted by the wallpaper when
//! transparency is on — which reads as a differently colored band sitting above
//! the webview (macOS never shows this seam: `titleBarStyle: "Overlay"` hands
//! the top strip to the app).
//!
//! The palette is a CSS decision: `--color-surface` paints the caption and
//! `--color-ink` the title, with a light block, a `.dark` block and a
//! `[data-palette="…"]` block per theme in `styles/globals.css`. Only the
//! webview knows which of those won, so `lib/theme.ts` sends the two computed
//! colors after every theme or palette change ([`apply`]). [`apply_os_default`]
//! covers the frames before the webview has painted: the saved theme lives in
//! `localStorage`, so at startup the backend can only follow the OS appearance.
//!
//! Windows 11 (21H2+, build 22000) honors both attributes; older builds reject
//! them and keep DWM's default, so a failed call is a no-op — never an error the
//! caller has to handle.

use tauri::Manager;

/// `COLORREF` is `0x00BBGGRR`; the values are `--color-surface` #ffffff and
/// `--color-ink` #172033 from the light block of `styles/globals.css`.
const LIGHT: (u32, u32) = (0x00FF_FFFF, 0x0033_2017);
/// The same two tokens from the `.dark` block: #161b22 and #e6ebf2.
const DARK: (u32, u32) = (0x0022_1B16, 0x00F2_EBE6);

/// Pin the caption and title-text colors from the frontend's rendered palette.
///
/// `caption` and `text` are `#rgb` / `#rrggbb` strings, exactly what
/// `getComputedStyle` returned for the two tokens. Only a malformed color is an
/// error: a missing window and a DWM build that rejects the attributes are both
/// no-ops.
#[cfg(target_os = "windows")]
pub(crate) fn apply<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    caption: &str,
    text: &str,
) -> Result<(), String> {
    let caption = parse_hex_color(caption)?;
    let text = parse_hex_color(text)?;
    if let Some(window) = app.get_webview_window("main") {
        paint(&window, caption, text);
    }
    Ok(())
}

/// Follow the OS appearance until the webview reports the saved theme.
#[cfg(target_os = "windows")]
pub(crate) fn apply_os_default<R: tauri::Runtime>(app: &tauri::App<R>) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};

    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let Ok(hwnd) = window.hwnd() else {
        return;
    };
    let dark = window
        .theme()
        .map(|theme| theme == tauri::Theme::Dark)
        .unwrap_or(false);
    let (caption, text) = if dark { DARK } else { LIGHT };
    // The frame's border and system menu follow the dark-mode attribute, which
    // the webview later keeps in sync through `Window::setTheme`.
    let dark_flag = i32::from(dark);
    // SAFETY: the pointer is a live `i32` of the stated size; a build that does
    // not know the attribute rejects the call and keeps its default.
    unsafe {
        let _ = DwmSetWindowAttribute(
            HWND(hwnd.0 as _),
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            std::ptr::from_ref(&dark_flag).cast(),
            size_of::<i32>() as u32,
        );
    }
    paint(&window, caption, text);
}

#[cfg(target_os = "windows")]
fn paint<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>, caption: u32, text: u32) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR,
    };

    let Ok(hwnd) = window.hwnd() else {
        return;
    };
    // SAFETY: each call passes a pointer to a live `u32` of the stated size.
    unsafe {
        for (attribute, color) in [(DWMWA_CAPTION_COLOR, caption), (DWMWA_TEXT_COLOR, text)] {
            let _ = DwmSetWindowAttribute(
                HWND(hwnd.0 as _),
                attribute,
                std::ptr::from_ref(&color).cast(),
                size_of::<u32>() as u32,
            );
        }
    }
}

/// A CSS color token to a DWM `COLORREF`. Both notations the stylesheet uses are
/// accepted; anything else is a bug on the calling side, not a color to guess.
#[cfg(target_os = "windows")]
fn parse_hex_color(value: &str) -> Result<u32, String> {
    let trimmed = value.trim();
    let hex = trimmed.strip_prefix('#').unwrap_or(trimmed);
    let expanded = match hex.len() {
        6 => hex.to_string(),
        3 => hex.chars().flat_map(|digit| [digit, digit]).collect(),
        _ => return Err(format!("expected #rrggbb, got {trimmed:?}")),
    };
    let channel = |range: std::ops::Range<usize>| {
        u8::from_str_radix(&expanded[range], 16)
            .map_err(|_| format!("expected #rrggbb, got {trimmed:?}"))
    };
    let red = channel(0..2)?;
    let green = channel(2..4)?;
    let blue = channel(4..6)?;
    Ok(u32::from(blue) << 16 | u32::from(green) << 8 | u32::from(red))
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    /// `COLORREF` is BGR-ordered, which is the whole point of having a parser.
    #[test]
    fn parses_the_tokens_the_stylesheet_uses() {
        assert_eq!(parse_hex_color("#ffffff").unwrap(), 0x00FF_FFFF);
        assert_eq!(parse_hex_color("#161b22").unwrap(), 0x0022_1B16);
        assert_eq!(parse_hex_color("#FFF").unwrap(), 0x00FF_FFFF);
        assert_eq!(parse_hex_color(" #172033 ").unwrap(), 0x0033_2017);
    }

    #[test]
    fn rejects_values_that_are_not_hex_colors() {
        for value in ["", "#", "rgb(1 2 3)", "#ff", "#gggggg", "#ffffff0"] {
            assert!(
                parse_hex_color(value).is_err(),
                "{value:?} should be rejected"
            );
        }
    }

    /// Whatever the caller sends, a frame that does not exist is not an error.
    #[test]
    fn applying_to_a_window_less_app_is_a_no_op() {
        let app = tauri::test::mock_app();
        apply_os_default(&app);
        apply(app.handle(), "#ffffff", "#172033").expect("no window is not an error");
    }
}
