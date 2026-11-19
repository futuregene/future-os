import type { CSSProperties, ReactNode } from "react";
import { CircleHelp } from "lucide-react";
import { useCallback, useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { cn } from "../../lib/cn";
import { useDismissableLayer } from "../../lib/useDismissableLayer";

/** Wide enough for the two or three lines a settings row's explanation runs to. */
const PANEL_WIDTH = 320;
/** Keeps the panel off the viewport edge, matching the menus' own gutter. */
const VIEWPORT_GUTTER = 12;
/** Space between the icon and the panel it opened. */
const ANCHOR_GAP = 6;

/**
 * The little "?" beside a settings label, whose explanation opens on **click**.
 *
 * A hover tooltip was the wrong shape for this text: a settings explanation is
 * something to read (two or three lines, and here it defines a term the row's
 * number depends on), and `title` gave the reader a native popup that vanished
 * the moment the pointer moved towards the input it describes — unreadable, and
 * unreachable from the keyboard and from touch. So the trigger is a real button
 * that toggles a panel: it stays up while the reader looks at the field, closes
 * on a second click, on a click anywhere else, or on Escape.
 *
 * Portalled and `position: fixed` on purpose. The settings card is
 * `overflow-hidden` and the row sits at the card's bottom edge, so a panel drawn
 * in place would be clipped to its first line.
 */
export function HelpTip({
  children,
  className,
  label,
}: {
  /** The explanation itself. Plain text — the panel has no interactive parts. */
  children: ReactNode;
  className?: string;
  /** Accessible name of the trigger, since the icon alone says nothing. */
  label: string;
}) {
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState<CSSProperties>();
  const panelRef = useRef<HTMLDivElement | null>(null);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const panelId = useId();
  const dismiss = useCallback(() => setOpen(false), []);
  const layerRef = useDismissableLayer<HTMLSpanElement>({ enabled: open, onDismiss: dismiss, additionalRef: panelRef });

  // Measured after mount, because the panel's height decides whether it fits
  // below the icon at all: a row near the window's bottom flips it upward
  // instead of hanging half of it off the edge.
  useLayoutEffect(() => {
    if (!open) {
      setPosition(undefined);
      return;
    }
    const update = () => {
      const anchor = triggerRef.current?.getBoundingClientRect();
      const panel = panelRef.current;
      if (!anchor || !panel) {
        return;
      }
      const below = window.innerHeight - anchor.bottom - VIEWPORT_GUTTER;
      const above = anchor.top - VIEWPORT_GUTTER;
      const height = panel.offsetHeight;
      const down = below >= height || below >= above;
      const left = Math.max(
        VIEWPORT_GUTTER,
        Math.min(anchor.left, window.innerWidth - PANEL_WIDTH - VIEWPORT_GUTTER),
      );
      setPosition({
        position: "fixed",
        top: Math.round(down ? anchor.bottom + ANCHOR_GAP : anchor.top - height - ANCHOR_GAP),
        left: Math.round(left),
        width: PANEL_WIDTH,
        zIndex: 60,
      });
    };
    update();
    // Reposition rather than dismiss on scroll: the icon is inside the settings
    // page's own scroller, and scrolling to read the field must not take the
    // explanation away.
    window.addEventListener("resize", update);
    document.addEventListener("scroll", update, true);
    return () => {
      window.removeEventListener("resize", update);
      document.removeEventListener("scroll", update, true);
    };
  }, [open]);

  return (
    <span className={cn("inline-flex", className)} ref={layerRef}>
      <button
        aria-controls={panelId}
        aria-expanded={open}
        aria-label={label}
        className="inline-flex items-center rounded-full text-ink-muted transition-colors hover:text-ink-soft focus-visible:ring-2 focus-visible:ring-focus focus-visible:outline-none"
        onClick={() => setOpen(value => !value)}
        ref={triggerRef}
        type="button"
      >
        <CircleHelp className="size-3.5" />
      </button>
      {open
        ? createPortal(
            <div
              className="rounded-lg border border-line-soft bg-surface px-3 py-2 text-xs leading-5 text-ink-soft shadow-panel"
              id={panelId}
              ref={panelRef}
              // Not `tooltip`: this one is opened by a click and stays, which is
              // what a note is — a tooltip is defined by hover/focus.
              role="note"
              style={position ?? { position: "fixed", visibility: "hidden" }}
            >
              {children}
            </div>,
            document.body,
          )
        : null}
    </span>
  );
}
