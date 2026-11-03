import { ArrowDown, ArrowUp } from "lucide-react";
import { useTranslation } from "react-i18next";

/**
 * "↑ previous question / ↓ next question" for the transcript's corner.
 *
 * Both directions walk the reader's own prompts, and only those: a press is
 * asking "what else was asked", so it never comes to rest inside an answer — a
 * long reply costs one press to pass, not one per screenful (see
 * `useQuestionNav`).
 *
 * Two round buttons on the conversation's own right edge, just above the
 * composer: the shape the mobile client ships, and for the same reason — a
 * control that only appears on hover is a control nobody finds, and the corner
 * above the composer is where the eye already is when a long answer pushes the
 * reader up the page. A direction with nothing to go to stays in place, inert,
 * so the pair never moves under the pointer.
 *
 * The same walk is on Page Up / Page Down (`useQuestionNav`), so the button's
 * tooltip names the key: the pair is the only place a reader can learn that a
 * page key pages the transcript, and a shortcut nobody can discover is not a
 * shortcut. The bare arrows are not on this walk — they keep the page's own line
 * scroll — so the tooltip must not suggest them.
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
        shortcut="PgUp"
        onClick={onPrevious}
      />
      <NavButton
        disabled={!hasNext}
        icon={<ArrowDown className="size-4" />}
        label={t("thread.questionNext")}
        shortcut="PgDn"
        onClick={onNext}
      />
    </div>
  );
}

function NavButton({
  disabled,
  icon,
  label,
  shortcut,
  onClick,
}: {
  disabled: boolean;
  icon: React.ReactNode;
  label: string;
  /** The key that does the same thing, shown in the tooltip. */
  shortcut: string;
  onClick: () => void;
}) {
  return (
    <button
      aria-label={label}
      className="flex size-8 items-center justify-center rounded-full border border-line-soft bg-surface text-ink shadow-panel transition-colors hover:text-accent disabled:cursor-default disabled:bg-surface-subtle disabled:text-ink-muted disabled:shadow-none disabled:hover:text-ink-muted"
      disabled={disabled}
      onClick={onClick}
      title={`${label}  ${shortcut}`}
      type="button"
    >
      {icon}
    </button>
  );
}
