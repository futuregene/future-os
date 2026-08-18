import { ArrowRight, GraduationCap, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";

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
 * info-tinted card: icon + title/subtitle on the left, a muted accent-tinted
 * action on the right (deliberately not the solid primary style, so it can't
 * be mistaken for the composer's send button), and a corner × that collapses
 * it (reopen lives on the Skills page, a later change).
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
        <Button
          className="shrink-0 gap-1 border-accent/30 bg-accent-soft text-accent hover:brightness-95"
          disabled={starting}
          onClick={onStart}
          size="sm"
          variant="secondary"
        >
          {t("skillGuide.start")}
          <ArrowRight className="size-3.5" />
        </Button>
      </div>
    </div>
  );
}
