import { Archive } from "lucide-react";
import { useTranslation } from "react-i18next";

/**
 * The first-archive coachmark, anchored beside the archive entry.
 *
 * Shown exactly once per installation (the shell persists `archiveGuideSeen`
 * after the first successful archive), because the question it answers — "where
 * did the conversation I just shelved go?" — is asked once and only once. It
 * points at the entry rather than describing it, so the answer is the control
 * itself.
 *
 * Modelled on `SkillIntroBubble`: an explicit two-button dismissal (no
 * outside-click close), so an accidental click cannot consume the one-time
 * explanation.
 */
export function ArchiveGuideBubble({
  onDismiss,
  onView,
}: {
  onDismiss: () => void;
  onView: () => void;
}) {
  const { t } = useTranslation("layout");
  return (
    <div className="absolute right-0 top-full z-30 mt-2 w-64 rounded-lg border border-line-soft bg-surface p-3 shadow-panel">
      {/* Caret pointing up at the entry this bubble is about. */}
      <div className="absolute right-2.5 top-[-5px] size-2.5 rotate-45 border-l border-t border-line-soft bg-surface" />
      <div className="flex items-center gap-1.5">
        <Archive className="size-4 shrink-0 text-accent" />
        <span className="text-sm font-semibold text-ink">{t("archiveGuide.title")}</span>
      </div>
      <p className="mt-1.5 text-xs leading-5 text-ink-soft">{t("archiveGuide.body")}</p>
      <div className="mt-2.5 flex items-center gap-2">
        <button
          className="inline-flex h-7 items-center rounded-md bg-accent-fill px-3 text-sm font-medium text-white transition-colors hover:bg-accent-fill-hover"
          onClick={onView}
          type="button"
        >
          {t("archiveGuide.viewArchived")}
        </button>
        <button
          className="inline-flex h-7 items-center rounded-md px-2 text-sm font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
          onClick={onDismiss}
          type="button"
        >
          {t("archiveGuide.dismiss")}
        </button>
      </div>
    </div>
  );
}
