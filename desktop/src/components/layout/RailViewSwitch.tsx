import type { RailView, RailViewOrder } from "./hooks/useRailView";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "../../lib/cn";

/**
 * How far the pointer must travel before a press on a segment becomes a drag.
 *
 * A press that never leaves the segment is a click, and the two views are the
 * only way to change the list — so the switch must not treat a shaky click as a
 * reorder. 4px is below the wobble of a deliberate press and well above it.
 */
const DRAG_THRESHOLD = 4;

interface DragState {
  active: boolean;
  id: RailView;
  pointerId: number;
  startX: number;
  startY: number;
}

/**
 * The activity rail's view switch: workspaces ⇄ recent.
 *
 * A row of its own above the list, with both names spelled out. An icon would
 * have cost the reader a hover to find out what it does, and the point of this
 * control is that the second view is discoverable at all — the tree is the
 * default and the reader has no other way to learn that another shape exists.
 *
 * Either segment can be dragged onto the other to exchange their places, and the
 * order is the reader's from then on (persisted by `useRailView`). Which view is
 * showing is untouched by a drag: dragging rearranges the switch, it does not
 * pick a list.
 *
 * `aria-pressed` rather than a tablist: these are two independent on/off answers
 * to "which list", and a tab role would promise arrow-key movement and a tab
 * panel that this rail does not have. Alt+ArrowLeft/Right is the same exchange
 * without a pointer, so reordering is not mouse-only.
 */
export function RailViewSwitch({
  onChange,
  onMove,
  order,
  view,
}: {
  onChange: (view: RailView) => void;
  /** Exchange `from` with `to`; the caller persists the result. */
  onMove: (from: RailView, to: RailView) => void;
  order: RailViewOrder;
  view: RailView;
}) {
  const { t } = useTranslation("layout");
  const nodesRef = useRef(new Map<RailView, HTMLButtonElement>());
  const dragRef = useRef<DragState | null>(null);
  // A drag ends with a click on the segment it started from; that click must not
  // also switch the list.
  const suppressClickRef = useRef(false);
  const [dragging, setDragging] = useState<RailView | null>(null);

  const labels: Record<RailView, string> = {
    recent: t("activityRail.viewRecent"),
    workspace: t("activityRail.viewWorkspace"),
  };

  /**
   * The segment whose half of the switch the pointer is over.
   *
   * Only `x` counts. The strip is 24px tall and the pointer is held by a hand,
   * not on rails: requiring it to stay inside that band would make the drag
   * slip off halfway across. Whichever half the pointer is in is the half the
   * reader is aiming at.
   */
  function viewAt(x: number): RailView | null {
    for (const id of order) {
      const node = nodesRef.current.get(id);
      if (!node)
        continue;
      const rect = node.getBoundingClientRect();
      if (x >= rect.left && x <= rect.right)
        return id;
    }
    return null;
  }

  function releasePointer(node: HTMLButtonElement | undefined, pointerId: number) {
    // jsdom (and any engine without pointer capture) simply has nothing to
    // release; the handlers work without it.
    try {
      if (node && node.hasPointerCapture?.(pointerId))
        node.releasePointerCapture(pointerId);
    }
    catch { /* The pointer is already gone. */ }
  }

  function handlePointerDown(event: React.PointerEvent<HTMLButtonElement>, id: RailView) {
    // Left button only: a right-click opens the platform menu, not a drag.
    if (event.button !== 0)
      return;
    suppressClickRef.current = false;
    dragRef.current = {
      active: false,
      id,
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
    };
    try {
      event.currentTarget.setPointerCapture?.(event.pointerId);
    }
    catch { /* Without capture the drag still works while over the switch. */ }
  }

  function handlePointerMove(event: React.PointerEvent<HTMLButtonElement>) {
    const current = dragRef.current;
    if (!current || current.pointerId !== event.pointerId)
      return;
    if (!current.active) {
      const distance = Math.hypot(event.clientX - current.startX, event.clientY - current.startY);
      if (distance < DRAG_THRESHOLD)
        return;
      current.active = true;
      setDragging(current.id);
    }
    // Crossing into the other segment exchanges the two. After the exchange the
    // pointer sits over the dragged segment's new place, so it does not bounce
    // back until the reader crosses over again.
    const over = viewAt(event.clientX);
    if (over && over !== current.id)
      onMove(current.id, over);
  }

  function handlePointerEnd(event: React.PointerEvent<HTMLButtonElement>) {
    const current = dragRef.current;
    if (!current || current.pointerId !== event.pointerId)
      return;
    dragRef.current = null;
    setDragging(null);
    releasePointer(nodesRef.current.get(current.id), current.pointerId);
    // The click that follows this pointerup belongs to the drag, not to a
    // deliberate press on the segment.
    if (current.active)
      suppressClickRef.current = true;
  }

  function handleClick(id: RailView) {
    if (suppressClickRef.current) {
      suppressClickRef.current = false;
      return;
    }
    onChange(id);
  }

  /** Alt+Arrow: the same exchange for a reader without a pointer. */
  function handleKeyDown(event: React.KeyboardEvent<HTMLButtonElement>, id: RailView) {
    if (!event.altKey || (event.key !== "ArrowLeft" && event.key !== "ArrowRight"))
      return;
    const step = event.key === "ArrowLeft" ? -1 : 1;
    const neighbor = order[order.indexOf(id) + step];
    if (!neighbor)
      return;
    event.preventDefault();
    onMove(id, neighbor);
  }

  return (
    <div
      aria-label={t("activityRail.viewSwitch")}
      className="mb-2 flex h-7 shrink-0 items-center gap-0.5 rounded-md bg-surface-subtle p-0.5"
      data-testid="rail-view-switch"
      role="group"
    >
      {order.map((id) => {
        const selected = view === id;
        return (
          <button
            aria-pressed={selected}
            className={cn(
              "flex h-6 min-w-0 flex-1 select-none items-center justify-center rounded px-1 text-xs font-medium transition-colors",
              selected
                ? "bg-surface text-ink shadow-sm"
                // The unselected segments sit on `surface-subtle`, where muted
                // ink only clears the 3:1 floor; a navigation label the reader
                // has to pick by name needs the body-text floor, so it uses the
                // soft ink and steps up to full ink on hover.
                : "text-ink-soft hover:text-ink",
              // While dragging, the segment still moves under the pointer but
              // reads as picked up rather than as a resting control.
              dragging === id ? "cursor-grabbing opacity-70" : "hover:cursor-grab",
            )}
            data-testid={`rail-view-${id}`}
            key={id}
            onClick={() => handleClick(id)}
            onKeyDown={event => handleKeyDown(event, id)}
            onPointerCancel={handlePointerEnd}
            onPointerDown={event => handlePointerDown(event, id)}
            onPointerMove={handlePointerMove}
            onPointerUp={handlePointerEnd}
            ref={(node) => {
              if (node)
                nodesRef.current.set(id, node);
              else
                nodesRef.current.delete(id);
            }}
            title={t("activityRail.reorderView")}
            type="button"
          >
            <span className="truncate">{labels[id]}</span>
          </button>
        );
      })}
    </div>
  );
}
