import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useState } from "react";

/**
 * Tracks whether the window is in native (macOS green-button) fullscreen.
 *
 * Tauri exposes no dedicated fullscreen event, so we re-query on resize —
 * entering or leaving fullscreen always resizes the window — and only update on
 * an actual change. This is an event subscription (not cancel-safe loading), so
 * it uses a plain effect rather than `useAsyncResource` (gui/CLAUDE.md §4).
 *
 * Used to drop the top-left traffic-light inset when fullscreen hides the
 * traffic lights. Best-effort: on error it stays `false`.
 */
export function useIsFullscreen(): boolean {
  const [isFullscreen, setIsFullscreen] = useState(false);

  useEffect(() => {
    let active = true;
    const win = getCurrentWindow();

    const sync = () => {
      void win
        .isFullscreen()
        .then((value) => {
          if (active)
            setIsFullscreen(value);
        })
        .catch(() => {});
    };

    sync();
    const unlisten = win.onResized(sync);

    return () => {
      active = false;
      void unlisten.then(stop => stop());
    };
  }, []);

  return isFullscreen;
}
