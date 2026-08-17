import { ArrowRight, GraduationCap, X } from "lucide-react";
import { useTranslation } from "react-i18next";

interface SkillGuideBannerProps {
  /** True while the coach conversation is being created. */
  starting: boolean;
  /** Start the skill-coaching conversation (fetches the platform prompt). */
  onStart: () => void;
  /** The user closed the entry; persists via app settings. */
  onDismiss: () => void;
}

/**
 * Skill-onboarding entry banner under the new-conversation composer. A soft
 * info-tinted card: icon + title/subtitle on the left, "start learning" action
 * on the right, and a corner × that collapses it (reopen lives on the Skills
 * page, a later change).
 */
export function SkillGuideBanner({ starting, onStart, onDismiss }: SkillGuideBannerProps) {
  const { t } = useTranslation("agent");
  return (
    <div className="relative mt-3 w-full max-w-3xl rounded-lg border border-info-line/60 bg-info-soft px-4 py-3">
      <button
        aria-label={t("skillGuide.dismiss")}
        className="absolute right-2 top-2 rounded-md p-1 text-ink-muted transition-colors hover:bg-surface/70 hover:text-ink"
        onClick={onDismiss}
        type="button"
      >
        <X className="size-3.5" />
      </button>
      <div className="flex items-center gap-3 pr-6">
        <div className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-accent-soft text-accent">
          <GraduationCap className="size-5" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="text-sm font-semibold text-ink">{t("skillGuide.title")}</div>
          <div className="mt-0.5 text-xs leading-5 text-ink-muted">{t("skillGuide.subtitle")}</div>
        </div>
        <button
          className="inline-flex h-8 shrink-0 items-center gap-1 rounded-md bg-accent px-3 text-sm font-medium text-white transition-colors hover:bg-accent-hover disabled:cursor-default disabled:bg-accent-disabled"
          disabled={starting}
          onClick={onStart}
          type="button"
        >
          {t("skillGuide.start")}
          <ArrowRight className="size-3.5" />
        </button>
      </div>
    </div>
  );
}
