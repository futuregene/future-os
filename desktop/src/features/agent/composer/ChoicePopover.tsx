import type { ChoiceCardMode, ChoiceCardOption } from "../../markdown/futureMarkdownTypes";
import { Check, ListChecks } from "lucide-react";
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "../../../lib/cn";

export interface ChoicePopoverProps {
  /**
   * Where to sit, in the editor wrapper's coordinate space: the pill's top and
   * left. The design puts the popover 10px above the pill it belongs to (§5.3)
   * so changing an answer never scrolls the reader back into the transcript.
   */
  anchor: { left: number; top: number };
  mode: ChoiceCardMode;
  options: ChoiceCardOption[];
  /** Close without changing anything (Esc, a click outside, or a re-pick). */
  onClose: () => void;
  onPick: (optionId: string) => void;
  selectedIds: string[];
  title: string;
  /** Width of the editor wrapper, to keep the popover inside it. */
  width: number;
}

const POPOVER_WIDTH = 360;

/**
 * The answer's other entrance (§5.3): the pill's pencil, opening the same
 * question in place.
 *
 * Compact rows — the card's explanatory line is dropped, because the reader has
 * already read it once and is now only changing their mind.
 */
export function ChoicePopover({
  anchor,
  mode,
  onClose,
  onPick,
  options,
  selectedIds,
  title,
  width,
}: ChoicePopoverProps) {
  const { t } = useTranslation("markdown");
  const ref = useRef<HTMLDivElement | null>(null);

  // Esc and a click outside both close without touching the answer (§5.3).
  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    }
    function onPointerDown(event: MouseEvent) {
      if (ref.current && event.target instanceof Node && !ref.current.contains(event.target))
        onClose();
    }
    document.addEventListener("keydown", onKeyDown, true);
    document.addEventListener("mousedown", onPointerDown, true);
    return () => {
      document.removeEventListener("keydown", onKeyDown, true);
      document.removeEventListener("mousedown", onPointerDown, true);
    };
  }, [onClose]);

  const multi = mode === "multi";
  // Keep the 360px panel inside the editor: a pill near the right edge would
  // otherwise push the panel out of the window.
  const left = Math.max(0, Math.min(anchor.left, width - POPOVER_WIDTH));

  return (
    <div
      className="absolute z-30 w-90 rounded-lg border border-line bg-surface p-1 shadow-panel"
      data-choice-popover
      ref={ref}
      role="dialog"
      aria-label={title}
      style={{ bottom: `calc(100% - ${anchor.top}px + 10px)`, left }}
    >
      <div className="flex items-center gap-1.5 px-2 pt-1 pb-1.5">
        <ListChecks className="size-3.5 shrink-0 text-ink-muted" />
        <p className="min-w-0 flex-1 truncate text-[12px] font-semibold text-ink">{title}</p>
        <span className="shrink-0 text-[11px] text-ink-muted">
          {multi ? t("choice.mode.multi") : t("choice.mode.single")}
        </span>
      </div>
      {options.map((option) => {
        const selected = selectedIds.includes(option.id);
        return (
          <button
            aria-checked={selected}
            className={cn(
              "flex h-8 w-full items-center gap-2 rounded-md px-2 text-left transition-colors",
              selected ? "bg-accent-soft" : "hover:bg-surface-subtle",
            )}
            data-choice-popover-option={option.id}
            key={option.id}
            onMouseDown={(event) => {
              // Keep the editor's focus/selection: picking here rewrites the
              // pill in place, it does not start a new insertion.
              event.preventDefault();
              onPick(option.id);
            }}
            role={multi ? "checkbox" : "radio"}
            type="button"
          >
            <span
              className={cn(
                "flex size-3.5 shrink-0 items-center justify-center border",
                // Same shape rule as the card: square for checkbox rows.
                multi ? "rounded-[3px]" : "rounded-full",
                selected ? "border-accent text-accent" : "border-line",
              )}
              aria-hidden="true"
              data-choice-mark={multi ? "box" : "ring"}
            >
              {selected ? <Check className="size-2" strokeWidth={3} /> : null}
            </span>
            <span className={cn("min-w-0 flex-1 truncate text-[13px] text-ink", selected ? "font-medium" : null)}>
              {option.label}
            </span>
          </button>
        );
      })}
    </div>
  );
}
