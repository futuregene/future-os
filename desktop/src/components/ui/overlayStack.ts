import { useCallback, useEffect, useRef } from "react";

/**
 * Tracks open Overlay layers so only the topmost handles Escape. window-level
 * keydown listeners can't rely on DOM nesting or stopPropagation (they all fire
 * regardless of order), so each Overlay registers a layer while open and checks
 * `isTop()` before closing on Escape — otherwise a nested dialog's Escape would
 * also close its parent.
 */
const stack: symbol[] = [];

/**
 * True while any Overlay layer is open. Used by global shortcuts that must not
 * fire behind a dialog.
 */
export function hasOpenOverlay(): boolean {
  return stack.length > 0;
}

export function useOverlayLayer(open: boolean) {
  const idRef = useRef<symbol | null>(null);

  useEffect(() => {
    if (!open)
      return;
    const id = Symbol("overlay");
    idRef.current = id;
    stack.push(id);
    return () => {
      const index = stack.lastIndexOf(id);
      if (index !== -1)
        stack.splice(index, 1);
      idRef.current = null;
    };
  }, [open]);

  const isTop = useCallback(
    () => stack.length > 0 && stack[stack.length - 1] === idRef.current,
    [],
  );

  return { isTop };
}
