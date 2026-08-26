import { useEffect, useRef, useState } from "react";

export const PREVIEW_LOADING_DELAY_MS = 200;
export const PREVIEW_LOADING_MIN_VISIBLE_MS = 300;

type PreviewLoadingPhase = "quiet" | "loading" | "ready";

/**
 * Keeps fast previews free of loading-state flashes. A load gets a short quiet
 * window; if it outlasts that window the indicator is shown, and once visible
 * it is held long enough to be perceived as a stable state.
 */
export function usePreviewLoadingGate(loading: boolean) {
  const [phase, setPhase] = useState<PreviewLoadingPhase>(loading ? "quiet" : "ready");
  const shownAtRef = useRef<number | null>(null);

  useEffect(() => {
    if (loading) {
      shownAtRef.current = null;
      setPhase("quiet");
      const showTimer = window.setTimeout(() => {
        shownAtRef.current = performance.now();
        setPhase("loading");
      }, PREVIEW_LOADING_DELAY_MS);
      return () => window.clearTimeout(showTimer);
    }

    const shownAt = shownAtRef.current;
    if (shownAt === null) {
      setPhase("ready");
      return;
    }

    const remaining = PREVIEW_LOADING_MIN_VISIBLE_MS - (performance.now() - shownAt);
    if (remaining <= 0) {
      shownAtRef.current = null;
      setPhase("ready");
      return;
    }

    const readyTimer = window.setTimeout(() => {
      shownAtRef.current = null;
      setPhase("ready");
    }, remaining);
    return () => window.clearTimeout(readyTimer);
  }, [loading]);

  return {
    showContent: phase === "ready",
    showLoading: phase === "loading",
  };
}
