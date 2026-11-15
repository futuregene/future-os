import type { ChoiceAnswer } from "../agent/composer/choiceAnswer";
import { ListChecks, Pencil, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { choiceAnswerLabel } from "../agent/composer/choiceAnswer";

interface Props {
  answer: ChoiceAnswer;
  onChange: () => void;
  onRemove: () => void;
}

/**
 * The answer to a `futureos-choice` card, as a chip above a remote field.
 *
 * Why a chip beside the field here, and a pill *inside* the local editor: a
 * remote composer is a plain `textarea`, so there is nowhere inside the field to
 * hold an atomic element, and no `isEditorEmpty` to treat one as content. The
 * chip is therefore a sibling of the field — which means every test that decides
 * whether the draft is empty and whether Send is live has to count the answer
 * itself. That is the half v1 got wrong: it drew a chip in the attachments row
 * while the send test read only the text, so "pick and send" could not work
 * (design `docs/design/choice-cards.zh-CN.md` §5.2.1, §5.4).
 *
 * The look follows the local pill (`accent-soft` fill, `accent` text, one line)
 * so the same answer is recognisably the same thing on both surfaces, and the
 * two controls use the local pill's own labels, so both surfaces say what they do
 * in the same words.
 */
export function RemoteChoiceChip({ answer, onChange, onRemove }: Props) {
  const { t } = useTranslation("markdown");
  const label = choiceAnswerLabel(answer) || answer.title;
  return (
    <span
      className="inline-flex max-w-full items-center gap-1.5 rounded-md border border-accent/20 bg-accent-soft py-1 pr-1 pl-2 text-xs text-accent"
      data-choice-chip
      // Which card this answers: a test, a screenshot or a screen reader can tell
      // two cards' answers apart without reading the reply above them.
      data-choice-title={answer.title}
      title={`${answer.title} → ${label}`}
    >
      <ListChecks aria-hidden className="size-3 shrink-0" />
      <span className="min-w-0 truncate font-medium">{label}</span>
      <button
        aria-label={t("choice.chip.change", { title: answer.title })}
        className="inline-flex size-4 shrink-0 items-center justify-center rounded text-accent/70 transition-colors hover:bg-surface hover:text-accent"
        data-choice-action="change"
        onClick={onChange}
        type="button"
      >
        <Pencil className="size-3" />
      </button>
      <button
        aria-label={t("choice.chip.remove", { title: answer.title })}
        className="inline-flex size-4 shrink-0 items-center justify-center rounded text-accent/70 transition-colors hover:bg-surface hover:text-accent"
        data-choice-action="remove"
        onClick={onRemove}
        type="button"
      >
        <X className="size-3" />
      </button>
    </span>
  );
}
