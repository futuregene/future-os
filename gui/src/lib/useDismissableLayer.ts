import { useEffect, useRef } from "react";

interface DismissableLayerOptions {
  enabled: boolean;
  onDismiss: () => void;
}

export function useDismissableLayer<T extends HTMLElement>({
  enabled,
  onDismiss,
}: DismissableLayerOptions) {
  const ref = useRef<T | null>(null);

  useEffect(() => {
    if (!enabled)
      return;

    function handlePointerDown(event: PointerEvent) {
      const target = event.target;
      if (!(target instanceof Node))
        return;
      if (ref.current?.contains(target))
        return;

      onDismiss();
    }

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        // Stop the Escape here (capture phase, before it bubbles to window) so a
        // parent Overlay's own Escape-to-close doesn't ALSO fire — otherwise one
        // press would dismiss both this layer and the modal containing it.
        event.stopPropagation();
        onDismiss();
      }
    }

    document.addEventListener("pointerdown", handlePointerDown, true);
    document.addEventListener("keydown", handleKeyDown, true);

    return () => {
      document.removeEventListener("pointerdown", handlePointerDown, true);
      document.removeEventListener("keydown", handleKeyDown, true);
    };
  }, [enabled, onDismiss]);

  return ref;
}
