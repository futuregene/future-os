import type { PointerEvent as ReactPointerEvent } from "react";
import type { FloatingScrollbarState } from "../../lib/useFloatingScrollbar";
import { cn } from "../../lib/cn";

/**
 * Thin, draggable overlay scrollbar thumb. Render as a sibling of the scroll
 * container inside a shared `group relative` wrapper, driven by
 * {@link ../../lib/useFloatingScrollbar}. A wide transparent hit area wraps the
 * hairline thumb so it stays easy to grab; it reveals on scroll (`visible`) or
 * when the surrounding `group` is hovered.
 */
export function FloatingScrollbar({
  scrollbar,
  onPointerDown,
}: {
  scrollbar: FloatingScrollbarState;
  onPointerDown: (event: ReactPointerEvent<HTMLDivElement>) => void;
}) {
  return (
    <div
      className={cn(
        "group/sb absolute right-0 top-0 z-20 flex w-3 touch-none justify-center",
        scrollbar.height > 0 ? "pointer-events-auto cursor-grab active:cursor-grabbing" : "pointer-events-none",
      )}
      style={{ height: `${scrollbar.height}px`, transform: `translateY(${scrollbar.top}px)` }}
      onPointerDown={onPointerDown}
    >
      <div
        className={cn(
          "h-full w-1 rounded-full bg-line transition-[opacity,background-color] duration-300 group-hover:opacity-80 group-hover/sb:bg-ink-muted",
          scrollbar.visible ? "opacity-80" : "opacity-0",
        )}
      />
    </div>
  );
}
