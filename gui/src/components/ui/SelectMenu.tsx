import type { ReactNode } from "react";
import { Check } from "lucide-react";
import { cn } from "../../lib/cn";
import { useDismissableLayer } from "../../lib/useDismissableLayer";
import { MenuPanel } from "./MenuPanel";

interface SelectMenuProps {
  /** Whether the floating panel is shown. */
  open: boolean;
  /** Called when the layer requests to close (outside click / Escape). */
  onDismiss: () => void;
  /** Trigger element; wire its `onClick` to toggle `open`. */
  trigger: ReactNode;
  /** Panel contents — typically `SelectMenuItem`s. */
  children: ReactNode;
  /** Extra classes on the anchor wrapper (e.g. responsive visibility). */
  className?: string;
  /** Extra classes on the floating panel (width / max-height / overflow). */
  panelClassName?: string;
  /**
   * Horizontal anchor of the panel relative to the trigger wrapper.
   * "right" (default) pins the panel's right edge to the trigger and grows
   * leftward — right for triggers near the right edge (model / thinking menus).
   * "left" pins the left edge and grows rightward — required for triggers near
   * the left edge (the approval-tier menu), otherwise the panel spills past the
   * left boundary and its leading content is clipped by an ancestor overflow.
   */
  align?: "left" | "right";
}

/**
 * Anchored dropdown menu: a trigger plus a `shadow-panel` popover that closes on
 * outside click or Escape via `useDismissableLayer`. The wrapper spans both the
 * trigger and the panel so clicking the trigger never self-dismisses. Open state
 * is controlled by the caller so sibling menus can coordinate (close on open).
 */
export function SelectMenu({ open, onDismiss, trigger, children, className, panelClassName, align = "right" }: SelectMenuProps) {
  const layerRef = useDismissableLayer<HTMLDivElement>({ enabled: open, onDismiss });
  return (
    <div className={cn("relative", className)} ref={layerRef}>
      {trigger}
      {open
        ? (
            <MenuPanel
              className={cn(
                "absolute bottom-9 z-30 divide-y divide-line-soft",
                align === "left" ? "left-0" : "right-0",
                panelClassName,
              )}
            >
              {children}
            </MenuPanel>
          )
        : null}
    </div>
  );
}

interface SelectMenuItemProps {
  selected: boolean;
  onSelect: () => void;
  children: ReactNode;
  className?: string;
}

/** A menu row with a trailing check mark when `selected`. */
export function SelectMenuItem({ selected, onSelect, children, className }: SelectMenuItemProps) {
  return (
    <button
      className={cn(
        "flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm transition-colors hover:bg-surface-subtle",
        className,
      )}
      onClick={onSelect}
      type="button"
    >
      {children}
      {selected ? <Check className="size-4 shrink-0 text-ink-soft" /> : null}
    </button>
  );
}
