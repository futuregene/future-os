import type { CSSProperties, ReactNode, Ref } from "react";
import { cn } from "../../lib/cn";

/**
 * The shared floating-menu surface: the rounded, bordered, shadowed panel skin
 * used by every dropdown / popover / context menu (SelectMenu, LinkContextMenu,
 * the activity-rail menus). Positioning, sizing, and z-index stay with the
 * caller via `className` — this only owns the surface look so it lives in one
 * place. `ref` targets the panel element (dismiss layer / drop-up measurement).
 */
export function MenuPanel({
  ref,
  className,
  style,
  children,
}: {
  ref?: Ref<HTMLDivElement>;
  className?: string;
  style?: CSSProperties;
  children: ReactNode;
}) {
  return (
    <div
      className={cn("rounded-lg border border-line-soft bg-surface shadow-panel", className)}
      ref={ref}
      style={style}
    >
      {children}
    </div>
  );
}
