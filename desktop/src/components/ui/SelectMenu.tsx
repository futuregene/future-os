import type { CSSProperties, ReactNode } from "react";
import { Check } from "lucide-react";
import { useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { cn } from "../../lib/cn";
import { useKeyboardOwner } from "../../lib/keyboardOwners";
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
  /** Float outside a scrolling field, with viewport-aware placement. */
  portal?: boolean;
  /** Minimum floating-panel width when portalled. */
  minPanelWidth?: number;
}

/**
 * Anchored dropdown menu: a trigger plus a `shadow-panel` popover that closes on
 * outside click or Escape via `useDismissableLayer`. The wrapper spans both the
 * trigger and the panel so clicking the trigger never self-dismisses. Open state
 * is controlled by the caller so sibling menus can coordinate (close on open).
 */
export function SelectMenu({ open, onDismiss, trigger, children, className, panelClassName, align = "right", portal = false, minPanelWidth = 224 }: SelectMenuProps) {
  const panelRef = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState<CSSProperties>();
  const layerRef = useDismissableLayer<HTMLDivElement>({ enabled: open, onDismiss, additionalRef: panelRef });
  // The panel is up: its rows are what the keyboard is aimed at (see
  // `lib/keyboardOwners`), so the transcript does not answer the arrows.
  useKeyboardOwner(open);
  useLayoutEffect(() => {
    if (!open || !portal)
      return;
    const update = () => {
      const anchor = layerRef.current?.getBoundingClientRect();
      const panel = panelRef.current;
      if (!anchor || !panel)
        return;
      const below = window.innerHeight - anchor.bottom - 12;
      const above = anchor.top - 12;
      const desired = Math.min(panel.scrollHeight, window.innerHeight * 0.4);
      const down = below >= desired || below >= above;
      const maxHeight = Math.max(0, Math.min(desired, down ? below : above));
      const width = Math.min(window.innerWidth - 16, Math.max(anchor.width, minPanelWidth));
      const left = Math.max(8, Math.min(align === "left" ? anchor.left : anchor.right - width, window.innerWidth - width - 8));
      setPosition({ position: "fixed", top: down ? anchor.bottom + 4 : anchor.top - maxHeight - 4, left, width, maxHeight, zIndex: 60 });
    };
    const dismissOnScroll = (event: Event) => {
      if (event.target instanceof Node && panelRef.current?.contains(event.target))
        return;
      onDismiss();
    };
    update();
    window.addEventListener("resize", update);
    document.addEventListener("scroll", dismissOnScroll, true);
    return () => {
      window.removeEventListener("resize", update);
      document.removeEventListener("scroll", dismissOnScroll, true);
    };
  }, [open, portal, align, minPanelWidth, layerRef, onDismiss]);
  const panel = open
    ? (
        <MenuPanel
          ref={panelRef}
          className={cn(
            "divide-y divide-line-soft",
            !portal && "absolute bottom-9 z-30",
            !portal && (align === "left" ? "left-0" : "right-0"),
            panelClassName,
          )}
          style={portal ? position ?? { position: "fixed", visibility: "hidden" } : undefined}
        >
          {children}
        </MenuPanel>
      )
    : null;
  return (
    <div className={cn("relative", className)} ref={layerRef}>
      {trigger}
      {portal && panel ? createPortal(panel, layerRef.current?.closest("[role=dialog]") ?? document.body) : panel}
    </div>
  );
}

interface SelectMenuItemProps {
  selected: boolean;
  onSelect: () => void;
  children: ReactNode;
  className?: string;
  disabled?: boolean;
  /** Optional native tooltip shown on hover (e.g. a model's description). */
  title?: string;
}

/** A menu row with a trailing check mark when `selected`. */
export function SelectMenuItem({ selected, onSelect, children, className, disabled = false, title }: SelectMenuItemProps) {
  return (
    <button
      className={cn(
        "flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm transition-colors hover:bg-surface-subtle",
        disabled && "cursor-not-allowed opacity-50 hover:bg-transparent",
        className,
      )}
      disabled={disabled}
      onClick={onSelect}
      title={title}
      type="button"
    >
      {children}
      {selected ? <Check className="size-4 shrink-0 text-ink-soft" /> : null}
    </button>
  );
}
