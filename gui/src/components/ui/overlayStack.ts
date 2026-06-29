import { useCallback, useEffect, useRef } from "react";

/**
 * Tracks open Overlay layers so only the topmost handles Escape. window-level
 * keydown listeners can't rely on DOM nesting or stopPropagation (they all fire
 * regardless of order), so each Overlay registers a layer while open and checks
 * `isTop()` before closing on Escape — otherwise a nested dialog's Escape would
 * also close its parent.
 */
const stack: symbol[] = [];

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
