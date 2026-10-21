import type { ReactNode } from "react";
import { ChevronDown, ChevronUp, ListTree } from "lucide-react";
import { useTranslation } from "react-i18next";

/**
 * The work a settled reply ran, folded above its answer.
 *
 * The `default` output level keeps a reply to the answer it ends on; the
 * reasoning, calls and notes that produced it fold into this one row so the
 * answer sits at the reading edge. Nothing is discarded: opening the row
 * replays exactly the blocks a reader sees with the setting off, in order.
 *
 * Counts describe work, not rows — a revealed step may itself stand for a burst
 * of calls (its own `count`), the same number `ReplySteps` prints.
 */
export function ReplyProcess({
  expanded,
  onToggle,
  thoughts,
  tools,
  children,
}: {
  expanded: boolean;
  onToggle: () => void;
  /** Thinking slices folded away. */
  thoughts: number;
  /** Tool calls folded away (a burst counts every call it stands for). */
  tools: number;
  children: ReactNode;
}) {
  const { t } = useTranslation("agent");
  const summary = [
    tools > 0 ? t("activity.stepTools", { count: tools }) : null,
    thoughts > 0 ? t("activity.stepThoughts", { count: thoughts }) : null,
  ].filter(Boolean).join(" · ");
  const Chevron = expanded ? ChevronUp : ChevronDown;

  return (
    <div className="flex min-w-0 flex-col gap-1 text-13 leading-6 text-ink-muted">
      <button
        aria-expanded={expanded}
        className="flex max-w-full cursor-pointer flex-wrap items-center justify-start gap-1 self-start text-left hover:text-ink"
        onClick={onToggle}
        type="button"
      >
        <ListTree aria-hidden="true" className="size-3.5 shrink-0" />
        <span>{t(expanded ? "process.hide" : "process.show")}</span>
        {summary ? <span className="text-ink-muted">{`· ${summary}`}</span> : null}
        <Chevron aria-hidden="true" className="size-3 shrink-0" />
      </button>
      {expanded ? <div className="min-w-0 space-y-3 pl-6">{children}</div> : null}
    </div>
  );
}
