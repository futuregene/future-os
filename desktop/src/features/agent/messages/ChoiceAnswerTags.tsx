import type { ChoiceAnswerEcho } from "@future-os/markdown";
import { ListChecks } from "lucide-react";
import { cn } from "../../../lib/cn";

/**
 * The answer a `futureos-choice` card printed into a prompt, drawn as a tag.
 *
 * Sending writes the answer as plain text (`[接下来做哪一块？ → 把归因搬进页面]`,
 * design `docs/design/choice-cards.zh-CN.md` §7), so a prompt that picked one row
 * would otherwise read as a bracket sandwich in the middle of the reader's own
 * words. Here the sentence is the same one — nothing is rewritten, and the copy
 * button still yields it — but it is drawn as what it is: a pick, one step in
 * from the prose the reader typed around it (the pill lands at the caret, so the
 * answer sits wherever the reader left the cursor rather than always behind
 * their words).
 *
 * The material is the card's, reduced: the accent hairline and the glyph that
 * head the card, on a tint light enough to sit *inside* the user's bubble rather
 * than beside it.
 */
export function ChoiceAnswerTags({ className, echoes }: { className?: string; echoes: readonly ChoiceAnswerEcho[] }) {
  return (
    <div className={cn("flex flex-wrap items-center gap-1.5", className)} data-choice-answer-tags>
      {echoes.map(echo => (
        <span
          className="inline-flex max-w-full items-start gap-1 rounded-md border border-accent/25 bg-accent/10 px-1.5 py-1 text-[12px] leading-4"
          // A card answers one question per pick, so the question is the row's
          // identity across a re-render.
          key={echo.question}
        >
          <ListChecks aria-hidden="true" className="mt-px size-3.5 shrink-0 text-accent" />
          <span className="min-w-0" data-choice-answer={echo.question}>
            <span className="text-ink-soft">{echo.question}</span>
            <span aria-hidden="true" className="px-1 text-ink-muted">→</span>
            <span className="font-medium text-ink">{echo.answer}</span>
          </span>
        </span>
      ))}
    </div>
  );
}
