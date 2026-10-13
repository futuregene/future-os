import { getCurrentWindow, UserAttentionType } from "@tauri-apps/api/window";

/**
 * Request the OS draw attention to this window (Dock bounce on macOS,
 * taskbar flash on Windows). Extracted so both local and remote run-finish
 * paths share the exact same call, and so it can be mocked in unit tests.
 */
export function requestUserAttention(): Promise<void> {
  return getCurrentWindow().requestUserAttention(UserAttentionType.Critical);
}
