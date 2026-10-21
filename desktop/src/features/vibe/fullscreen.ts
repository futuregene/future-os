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
