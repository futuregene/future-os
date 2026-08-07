import type { RefObject } from "react";
import { Fragment, useLayoutEffect, useState } from "react";
import { MenuPanel } from "../../../components/ui/MenuPanel";
import { cn } from "../../../lib/cn";

/** Keep the menu this many px clear of the viewport edges. */
const VIEWPORT_MARGIN = 8;

export interface LinkMenuItem {
  /** Style the item as destructive (e.g. delete). */
  danger?: boolean;
  /** Draw a separator line above this item. */
  divider?: boolean;
  label: string;
  onSelect: () => void;
}

/**
 * Cursor-anchored right-click menu shared by the markdown link renderers —
 * `SafeLink` (remote URLs) and `FileLink` (local paths). Pair it with the
 * `useLinkContextMenu` controller, which owns the position state and
 * outside-dismiss wiring; the menu closes itself after an item is chosen.
 */
export function LinkContextMenu({
  controller,
  items,
}: {
  controller: {
    close: () => void;
    layerRef: RefObject<HTMLDivElement | null>;
    position: { x: number; y: number } | null;
  };
  items: LinkMenuItem[];
}) {
  const { close, layerRef, position } = controller;
  // Cursor-anchored by default, but flip/clamp back inside the viewport once we
  // can measure the rendered menu — otherwise a right-click near the panel's
  // right or bottom edge pushes the menu off-screen.
  const [coords, setCoords] = useState<{ x: number; y: number } | null>(null);
  useLayoutEffect(() => {
    if (!position) {
      setCoords(null);
      return;
    }
    const el = layerRef.current;
    if (!el) {
      setCoords(position);
      return;
    }
    const { height, width } = el.getBoundingClientRect();
    const maxX = window.innerWidth - VIEWPORT_MARGIN - width;
    const maxY = window.innerHeight - VIEWPORT_MARGIN - height;
    setCoords({
      x: Math.max(VIEWPORT_MARGIN, Math.min(position.x, maxX)),
      y: Math.max(VIEWPORT_MARGIN, Math.min(position.y, maxY)),
    });
  }, [position, layerRef]);

  if (!position)
    return null;

  // First paint renders at the raw cursor point (needed to measure); the layout
  // effect corrects it before the browser paints, so there's no visible jump.
  const pos = coords ?? position;

  return (
    <MenuPanel
      className="fixed z-50 min-w-44 overflow-hidden py-1"
      ref={layerRef}
      style={{ left: pos.x, top: pos.y }}
    >
      {items.map(item => (
        <Fragment key={item.label}>
          {item.divider ? <div className="my-1 border-t border-line-soft" /> : null}
          <button
            className={cn(
              "block w-full whitespace-nowrap px-3 py-1.5 text-left text-sm",
              item.danger ? "text-danger hover:bg-danger-soft" : "text-ink hover:bg-surface-subtle",
            )}
            onClick={() => {
              close();
              item.onSelect();
            }}
            type="button"
          >
            {item.label}
          </button>
        </Fragment>
      ))}
    </MenuPanel>
  );
}
