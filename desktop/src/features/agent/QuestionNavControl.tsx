import { ArrowDown, ArrowUp } from "lucide-react";
import { useTranslation } from "react-i18next";

/**
 * "↑ previous question / ↓ next passage" for the transcript's corner.
 *
 * The pair is deliberately not symmetric (see `useQuestionNav`): ↑ walks the
 * reader's own prompts, skipping whatever answer lies between two of them, while
 * ↓ takes the next stop below of either kind — so ↓ reads as the transcript
 * written out, prompt, reply, prompt…
 *
 * Two round buttons on the conversation's own right edge, just above the
 * composer: the shape the mobile client ships, and for the same reason — a
 * control that only appears on hover is a control nobody finds, and the corner
 * above the composer is where the eye already is when a long answer pushes the
 * reader up the page. A direction with nothing to go to stays in place, inert,
 * so the pair never moves under the pointer.
 */
export function QuestionNavControl({
  hasPrevious,
  hasNext,
  onPrevious,
  onNext,
}: {
  hasPrevious: boolean;
  hasNext: boolean;
  onPrevious: () => void;
  onNext: () => void;
}) {
  const { t } = useTranslation("agent");
  return (
    <div className="pointer-events-auto flex items-center gap-1.5">
      <NavButton
        disabled={!hasPrevious}
        icon={<ArrowUp className="size-4" />}
        label={t("thread.questionPrevious")}
        onClick={onPrevious}
      />
      <NavButton
        disabled={!hasNext}
        icon={<ArrowDown className="size-4" />}
        label={t("thread.nextPassage")}
        onClick={onNext}
      />
    </div>
  );
}

function NavButton({
  disabled,
  icon,
  label,
  onClick,
}: {
  disabled: boolean;
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      aria-label={label}
      className="flex size-8 items-center justify-center rounded-full border border-line-soft bg-surface text-ink shadow-panel transition-colors hover:text-accent disabled:cursor-default disabled:bg-surface-subtle disabled:text-ink-muted disabled:shadow-none disabled:hover:text-ink-muted"
      disabled={disabled}
      onClick={onClick}
      title={label}
      type="button"
    >
      {icon}
    </button>
  );
}
