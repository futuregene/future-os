import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import type { ChoiceCardMode, ChoiceCardOption } from "../futureMarkdownTypes";
import { Check, ListChecks } from "lucide-react";
import { useId } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "../../../lib/cn";

export interface ChoiceCardProps {
  /**
   * The situation the question sits in, in the model's own words (§3). It sits
   * under the title so the reader can answer without scrolling back up the reply;
   * the fence may omit it.
   */
  context?: string;
  /**
   * The model's own line about the question, shown under the context when the
   * fence carried one. There is no default subtitle to override any more (§5.1).
   */
  hint?: string;
  mode: ChoiceCardMode;
  /**
   * Picking a row. Omitted on a read-only card (every card but the thread's
   * last), which also turns the rows out of the tab order — a card that looks
   * clickable but answers nothing is worse than an inert one.
   */
  onSelect?: (optionId: string) => void;
  options: ChoiceCardOption[];
  /** Option ids the draft's answer holds; the card derives, it never stores. */
  selectedIds?: string[];
  title: string;
}

/**
 * A `futureos-choice` block: the reply's closing question, drawn as a card.
 *
 * The design (`docs/design/choice-cards.zh-CN.md` §5.1–5.2) is a grouped-row
 * card: a title, a subtitle pointing at the composer, and one row per option
 * carrying a line on what picking it changes. Picking is click-only — the
 * composer's own digits are never taken over (§5.6).
 */
export function ChoiceCard({
  context,
  hint,
  mode,
  onSelect,
  options,
  selectedIds = [],
  title,
}: ChoiceCardProps) {
  const { t } = useTranslation("markdown");
  const titleId = useId();
  const hintId = useId();
  const readonlyId = useId();
  const interactive = Boolean(onSelect);
  const multi = mode === "multi";
  const chosen = options.filter(option => selectedIds.includes(option.id));
  // Roving tabindex (§9): Tab reaches the card, then the arrows walk the rows.
  const focusIndex = Math.max(0, options.findIndex(option => selectedIds.includes(option.id)));
  // The card has no subtitle of its own any more (§5.1): the mode label says
  // single or multi, the checkbox/radio shape says the same thing again, and
  // "pick one and it lands in the input" explained a mechanism the pill makes
  // self-evident — a line of chrome on every card, read once and skipped after
  // that. What the model itself wrote still shows, and a past card adds the
  // notice saying it can no longer be picked.
  const describedBy = [hint ? hintId : null, interactive ? null : readonlyId]
    .filter(Boolean)
    .join(" ") || undefined;

  return (
    <div
      aria-describedby={describedBy}
      aria-disabled={interactive ? undefined : true}
      aria-labelledby={titleId}
      className="w-full max-w-3xl overflow-hidden rounded-[10px] border border-line bg-surface shadow-panel"
      data-choice-card
      data-choice-mode={mode}
      data-choice-state={interactive ? "active" : "readonly"}
      role="group"
    >
      <div className="px-3 pt-2.5 pb-2">
        <div className="flex items-center gap-1.5">
          <ListChecks className="size-3.5 shrink-0 text-ink-muted" />
          <h4 className="min-w-0 flex-1 text-[12px] font-semibold text-ink" id={titleId}>{title}</h4>
          <span className="shrink-0 text-[11px] text-ink-muted">
            {multi ? t("choice.mode.multi") : t("choice.mode.single")}
          </span>
        </div>
        {/*
          The situation, above the instruction. The card has to be answerable on
          its own: a reader who arrives at the bottom of a long reply should not
          have to scroll back up to know what is being decided. It stays a step
          quieter than the title (`ink-soft`, not `ink-muted`) because it is
          content, not chrome — the subtitle below it is the chrome.
        */}
        {context
          ? (
              <p className="mt-1.5 text-[12px] leading-[18px] whitespace-pre-line text-ink-soft" data-choice-context>
                {context}
              </p>
            )
          : null}
        {/*
          The model's own hint, when it wrote one. There is no default any more:
          the mode label above already answers "one or several", and the old
          line promised a mechanism ("lands in the input") that the composer now
          shows by itself. The notice below is not a subtitle — it is a fact a
          past card owes the reader.
        */}
        {hint
          ? (
              <p className="mt-1 text-[11px] leading-4 text-ink-muted" data-choice-hint id={hintId}>{hint}</p>
            )
          : null}
        {interactive
          ? null
          : (
              <p
                className="mt-1 text-[11px] leading-4 text-ink-muted"
                data-choice-readonly
                id={readonlyId}
              >
                {t("choice.hint.readonly")}
              </p>
            )}
      </div>
      <div
        className="px-1.5 pb-1.5"
        role={multi ? "group" : "radiogroup"}
        aria-labelledby={titleId}
        aria-readonly={interactive ? undefined : true}
      >
        {options.map((option, index) => (
          <ChoiceOptionRow
            dimUnselected={!interactive && selectedIds.length > 0}
            focusable={interactive && index === focusIndex}
            interactive={interactive}
            key={option.id}
            mode={mode}
            onSelect={onSelect}
            option={option}
            selected={selectedIds.includes(option.id)}
          />
        ))}
      </div>
      {interactive && chosen.length > 0
        ? (
            <p
              className="border-t border-line-soft bg-surface-subtle px-3 py-2 text-[11px] leading-4 text-ink-muted"
              data-choice-footer
            >
              {t("choice.foot.queued")}
            </p>
          )
        : null}
    </div>
  );
}

function ChoiceOptionRow({
  dimUnselected,
  focusable,
  interactive,
  mode,
  onSelect,
  option,
  selected,
}: {
  dimUnselected: boolean;
  focusable: boolean;
  interactive: boolean;
  mode: ChoiceCardMode;
  onSelect?: (optionId: string) => void;
  option: ChoiceCardOption;
  selected: boolean;
}) {
  return (
    <div
      aria-checked={selected}
      aria-disabled={interactive ? undefined : true}
      className={cn(
        "flex w-full items-start gap-2.5 rounded-md px-2 py-2 text-left transition-colors",
        selected ? "bg-accent-soft" : null,
        interactive && !selected ? "hover:bg-surface-subtle" : null,
        // A past card still shows what was picked there; the other rows recede
        // so the highlight is the thing being compared (§2). Nothing is dimmed
        // while no answer is known — a uniformly grey card with no marked row
        // reads as broken rather than as historical.
        dimUnselected && !selected ? "opacity-50" : null,
      )}
      data-choice-option={option.id}
      data-choice-selected={selected ? "true" : undefined}
      onClick={interactive ? () => onSelect?.(option.id) : undefined}
      onKeyDown={interactive ? handleRowKey : undefined}
      role={mode === "multi" ? "checkbox" : "radio"}
      /*
        No `tabindex` at all on a read-only row, rather than `-1`: a div carrying
        `tabindex="-1"` is out of the tab order but still takes focus from script,
        so it can end up looking focused while answering nothing. Omitting the
        attribute is what a disabled radio does — unbrowsable by Tab, still in the
        accessibility tree, unable to take focus — and the row is `aria-disabled`
        anyway.
      */
      tabIndex={interactive ? (focusable ? 0 : -1) : undefined}
    >
      <span
        aria-hidden="true"
        className={cn(
          "mt-0.5 flex size-4 shrink-0 items-center justify-center border transition-colors",
          // Shape is what tells the two cards apart at a glance: a row that may
          // carry several picks wears a checkbox, not the radio circle.
          mode === "multi" ? "rounded-[3px]" : "rounded-full",
          selected ? "border-accent text-accent" : "border-line",
        )}
        data-choice-mark={mode === "multi" ? "box" : "ring"}
      >
        {selected ? <Check className="size-2.5" strokeWidth={3} /> : null}
      </span>
      <span className="min-w-0 flex-1">
        <span
          className={cn(
            "block text-[13px] leading-5 text-ink",
            selected ? "font-medium" : null,
          )}
        >
          {option.label}
        </span>
        {option.description
          ? <span className="mt-0.5 block text-[12px] leading-4 text-ink-muted">{option.description}</span>
          : null}
      </span>
    </div>
  );
}

/**
 * Arrow keys walk the rows, Enter/Space pick (§9): the card is a radio group, so
 * it must behave like one for a keyboard reader. Space needs preventing —
 * otherwise the page scrolls under an already-handled key.
 */
function handleRowKey(event: ReactKeyboardEvent<HTMLDivElement>) {
  const row = event.currentTarget;
  const rows = Array.from(row.parentElement?.children ?? [])
    .filter((child): child is HTMLElement => child instanceof HTMLElement && child.hasAttribute("data-choice-option"));
  const index = rows.indexOf(row);
  const forward = event.key === "ArrowDown" || event.key === "ArrowRight";
  const back = event.key === "ArrowUp" || event.key === "ArrowLeft";
  if (forward || back) {
    event.preventDefault();
    const next = rows[(index + (forward ? 1 : -1) + rows.length) % rows.length];
    next?.focus();
    return;
  }
  if (event.key === " ") {
    event.preventDefault();
    row.click();
  }
}
