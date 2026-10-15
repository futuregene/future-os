import type { MouseEvent } from "react";
import { useState } from "react";
import { useKeyboardOwner } from "../../../lib/keyboardOwners";
import { useDismissableLayer } from "../../../lib/useDismissableLayer";

/**
 * Owns the open/close position state and outside-dismiss wiring for a
 * cursor-anchored link context menu. Pair the returned controller with
 * `<LinkContextMenu>` — see `SafeLink` / `FileLink` for usage.
 */
export function useLinkContextMenu() {
  const [position, setPosition] = useState<{ x: number; y: number } | null>(null);
  const layerRef = useDismissableLayer<HTMLDivElement>({
    enabled: position !== null,
    onDismiss: () => setPosition(null),
  });
  // The open menu owns the arrows, not the transcript behind it.
  useKeyboardOwner(position !== null);

  return {
    close: () => setPosition(null),
    layerRef,
    open: (event: MouseEvent<HTMLElement>) => {
      event.preventDefault();
      setPosition({ x: event.clientX, y: event.clientY });
    },
    position,
  };
}
