import type { RefObject } from "react";
import { Fragment } from "react";
import { MenuPanel } from "../../../components/ui/MenuPanel";
import { cn } from "../../../lib/cn";

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
  if (!position)
    return null;

  return (
    <MenuPanel
      className="fixed z-50 min-w-44 overflow-hidden py-1"
      ref={layerRef}
      style={{ left: position.x, top: position.y }}
    >
      {items.map(item => (
        <Fragment key={item.label}>
          {item.divider ? <div className="my-1 border-t border-line-soft" /> : null}
          <button
            className={cn(
              "block w-full px-3 py-1.5 text-left text-sm",
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
