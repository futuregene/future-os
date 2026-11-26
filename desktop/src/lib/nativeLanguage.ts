import i18n, { getLanguage } from "../i18n";
import { invokeCommand } from "../integrations/tauri/invoke";

/**
 * Tell the Rust side which language the window is showing.
 *
 * Native dialogs raised outside a command — the quit confirmation is one — have
 * no invoke of their own to carry a language tag, so the resolved language is
 * pushed once at start and again whenever the user switches. The backend keeps
 * the last tag and builds `quit.*` copy from it; see `src-tauri/src/native_dialog.rs`.
 *
 * Outside a Tauri webview the call throws or rejects, which is not worth
 * reporting (same policy as `syncWindowCaptionColors`).
 */
export function pushNativeLanguage(language: string = getLanguage()): void {
  try {
    void invokeCommand("set_ui_language", { language }).catch(() => {});
  }
  catch {
    // Not running inside Tauri (tests, browser preview).
  }
}

pushNativeLanguage();
i18n.on("languageChanged", () => pushNativeLanguage());
