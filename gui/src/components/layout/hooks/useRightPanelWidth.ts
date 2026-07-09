import type { MouseEvent, RefObject } from "react";
import { useCallback, useEffect, useRef, useState } from "react";

/**
 * One shared width for the whole app session (sessionStorage → cleared on
 *  app restart, not persisted to disk). Not keyed by thread: switching
 *  conversations keeps the same right-panel width.
 */
const STORAGE_KEY = "future.rightPanelWidth";

/**
 * Default equals the historical fixed width (`w-96` = 384px) and doubles as
 *  the floor. The center chat area shares the same 384px minimum, so on the
 *  smallest supported window (1024px) both panels still fit.
 */
export const RIGHT_PANEL_DEFAULT_WIDTH = 384;
const MIN_RIGHT = 384;
const MIN_CENTER = 384;

function readStoredWidth(): number {
  try {
    const raw = sessionStorage.getItem(STORAGE_KEY);
    const parsed = raw ? Number.parseInt(raw, 10) : Number.NaN;
    return Number.isFinite(parsed) ? parsed : RIGHT_PANEL_DEFAULT_WIDTH;
  }
  catch {
    // sessionStorage may be unavailable (private mode / disabled) — best effort.
    return RIGHT_PANEL_DEFAULT_WIDTH;
  }
}

/**
 * Clamp a desired right-panel width so the center keeps ≥ `MIN_CENTER` and the
 *  right keeps ≥ `MIN_RIGHT`. `centerLeft` is the center pane's left edge (i.e.
 *  the left rail's right edge); the window right edge is the app's right edge.
 */
function clampWidth(desired: number, centerLeft: number): number {
  const available = window.innerWidth - centerLeft;
  // Center is the priority: never let it fall below MIN_CENTER. On a window too
  // narrow to honor both floors, the upper bound wins and the right panel takes
  // whatever is left (Math.max keeps the range non-inverted).
  const maxRight = Math.max(MIN_RIGHT, available - MIN_CENTER);
  return Math.min(Math.max(Math.round(desired), MIN_RIGHT), maxRight);
}

/**
 * Drag-to-resize state for the right context panel. `centerRef` points at the
 * center `<main>` element so the clamp can read its live left edge (which moves
 * when the left rail expands/collapses or the window resizes).
 */
export function useRightPanelWidth(centerRef: RefObject<HTMLElement | null>) {
  const [width, setWidth] = useState<number>(readStoredWidth);
  const [resizing, setResizing] = useState(false);
  const centerLeftRef = useRef(0);
  // Detach for an in-progress resize drag, so unmounting mid-drag removes the
  // window listeners instead of leaking them and setting state on an unmounted
  // hook.
  const dragCleanupRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    try {
      sessionStorage.setItem(STORAGE_KEY, String(width));
    }
    catch {
      // best effort — see readStoredWidth.
    }
  }, [width]);

  const reclamp = useCallback(() => {
    const centerLeft = centerRef.current?.getBoundingClientRect().left ?? 0;
    setWidth(current => clampWidth(current, centerLeft));
  }, [centerRef]);

  // Re-clamp on mount and whenever the window resizes, so a shrunk window (or a
  // width restored from a larger session) can't leave the center below its floor.
  useEffect(() => {
    reclamp();
    window.addEventListener("resize", reclamp);
    return () => window.removeEventListener("resize", reclamp);
  }, [reclamp]);

  const startResize = useCallback((event: MouseEvent) => {
    if (event.button !== 0)
      return;
    event.preventDefault();
    document.getSelection()?.removeAllRanges();
    // The left rail can't move mid-drag, so snapshot the center's left edge once.
    centerLeftRef.current = centerRef.current?.getBoundingClientRect().left ?? 0;
    setResizing(true);

    const onMove = (moveEvent: globalThis.MouseEvent) => {
      setWidth(clampWidth(window.innerWidth - moveEvent.clientX, centerLeftRef.current));
    };
    const onUp = () => {
      setResizing(false);
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      dragCleanupRef.current = null;
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    dragCleanupRef.current = onUp;
  }, [centerRef]);

  // Detach any in-progress resize drag on unmount (the normal path detaches on mouseup).
  useEffect(() => () => dragCleanupRef.current?.(), []);

  const nudge = useCallback((deltaPx: number) => {
    const centerLeft = centerRef.current?.getBoundingClientRect().left ?? 0;
    setWidth(current => clampWidth(current + deltaPx, centerLeft));
  }, [centerRef]);

  return { width, resizing, startResize, reclamp, nudge };
}
