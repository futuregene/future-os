import type { PointerEvent } from "react";
import { useCallback, useEffect, useRef, useState } from "react";
import { MIN_CENTER_PANEL_WIDTH, MIN_RIGHT_PANEL_WIDTH } from "./panelGeometry";

const STORAGE_KEY = "future.leftPanelWidth";
export const MIN_LEFT_PANEL_WIDTH = 224;
const MAX_LEFT_PANEL_WIDTH = 480;

function initialWidth(): number {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    const value = raw === null ? Number.NaN : Number(raw);
    if (Number.isFinite(value))
      return value;
  }
  catch { /* Storage is best effort. */ }
  return window.innerWidth >= 1280 ? 288 : window.innerWidth >= 768 ? 256 : 224;
}

export function useLeftPanelWidth(rightExpanded: boolean) {
  const [preferredWidth, setPreferredWidth] = useState(initialWidth);
  const [windowWidth, setWindowWidth] = useState(() => window.innerWidth);
  const [resizing, setResizing] = useState(false);
  const cleanupRef = useRef<(() => void) | null>(null);
  // Reserve the existing right panel's floor; its own hook re-clamps its width.
  const maxWidth = Math.max(
    MIN_LEFT_PANEL_WIDTH,
    Math.min(MAX_LEFT_PANEL_WIDTH, windowWidth - MIN_CENTER_PANEL_WIDTH - (rightExpanded ? MIN_RIGHT_PANEL_WIDTH : 0)),
  );
  const width = Math.min(maxWidth, Math.max(MIN_LEFT_PANEL_WIDTH, preferredWidth));

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, String(preferredWidth));
    }
    catch { /* Storage is best effort. */ }
  }, [preferredWidth]);

  useEffect(() => {
    const onResize = () => setWindowWidth(window.innerWidth);
    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("resize", onResize);
      cleanupRef.current?.();
    };
  }, []);

  const startResize = useCallback((event: PointerEvent) => {
    if (event.button !== 0)
      return;
    event.preventDefault();
    cleanupRef.current?.();
    document.getSelection()?.removeAllRanges();
    const startX = event.clientX;
    const pointerId = event.pointerId;
    const clamp = (value: number) => Math.min(maxWidth, Math.max(MIN_LEFT_PANEL_WIDTH, Math.round(value)));
    const onMove = (move: globalThis.PointerEvent) => {
      if (move.pointerId === pointerId)
        setPreferredWidth(clamp(width + move.clientX - startX));
    };
    const finish = () => {
      setResizing(false);
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onEnd);
      window.removeEventListener("pointercancel", onEnd);
      window.removeEventListener("blur", finish);
      cleanupRef.current = null;
    };
    function onEnd(end: globalThis.PointerEvent) {
      if (end.pointerId === pointerId)
        finish();
    }
    setResizing(true);
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onEnd);
    window.addEventListener("pointercancel", onEnd);
    window.addEventListener("blur", finish);
    cleanupRef.current = finish;
  }, [maxWidth, width]);

  const nudge = (delta: number) => setPreferredWidth(Math.min(maxWidth, Math.max(MIN_LEFT_PANEL_WIDTH, width + delta)));
  return { width, maxWidth, resizing, startResize, nudge };
}
