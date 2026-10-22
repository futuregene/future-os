import { getCurrentWindow } from "@tauri-apps/api/window";

/**
 * Toggle native fullscreen, and **say so when it fails**.
 *
 * The button did nothing for a whole release and nobody could tell why: the call was
 * `isFullscreen().then(full => setFullscreen(!full)).catch(() => {})`, and
 * `core:window:allow-set-fullscreen` was not in `capabilities/default.json` — so the
 * capability layer rejected `setFullscreen`, the rejection was swallowed, and the button
 * silently did nothing. `isFullscreen` *was* allowed (`core:default` grants it), which is
 * why the read half never complained.
 *
 * So: one function for all three call sites (the bar's icon, the menu's row, the
 * `vibe.fullscreen` command), and a failure is **reported to the caller** instead of
 * being dropped. The caller shows a toast; the console keeps the reason.
 */
export async function toggleNativeFullscreen(): Promise<{ ok: true; fullscreen: boolean } | { ok: false; error: unknown }> {
  try {
    const win = getCurrentWindow();
    const full = await win.isFullscreen();
    await win.setFullscreen(!full);
    return { fullscreen: !full, ok: true };
  }
  catch (error) {
    // Keep the reason: a missing permission, a window that refuses fullscreen, or a
    // webview without the API all land here, and they are told apart by this line.
    console.error("vibe: toggling fullscreen failed", error);
    return { error, ok: false };
  }
}

/**
 * Leave native fullscreen, if the window is in it.
 *
 * Called as the panel closes, because **the panel is the only way into fullscreen**:
 * its bar's icon, its menu row and `vibe.fullscreen` are the only three surfaces that
 * offer the toggle, and all three belong to the panel. Close it while fullscreen and a
 * reader is left in a window with no way back — Windows draws no title bar in fullscreen
 * (so there is not even a restore button to undo it with), the macOS
 * `View > Exit Full Screen` menu item is the only other door, and the panel's own key
 * went away with the panel. `Esc` is what closes the panel on the grid layer, so the
 * key a reader reaches for first is the one that walks them into it.
 *
 * So leaving the panel leaves fullscreen with it: the state the reader was in before
 * they pressed the button. Deliberately **not** the panel's unmount cleanup — React
 * StrictMode mounts, unmounts and remounts in development, and a reader who was already
 * fullscreen (the macOS green button) would be dropped out of it every time they opened
 * the panel. The shell calls this from the paths that actually close the panel.
 *
 * Best-effort: a refusal here is not something the reader asked for, so it is logged and
 * not toasted (unlike the toggle's refusal, which is the reader's own request failing).
 */
export async function exitNativeFullscreen(): Promise<void> {
  try {
    const win = getCurrentWindow();
    if (await win.isFullscreen())
      await win.setFullscreen(false);
  }
  catch (error) {
    console.error("vibe: leaving fullscreen failed", error);
  }
}
