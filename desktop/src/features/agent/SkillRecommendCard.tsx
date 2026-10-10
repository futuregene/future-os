import { Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";

/**
 * The skill-recommendation card shown above a composer.
 *
 * Shared by this app's composer and the one for a paired computer: the card is
 * the same decision either way — hold the message, install the skill or send
 * without it — and only the machine the install lands on differs.
 */
export function SkillRecommendCard({
  description,
  installing,
  name,
  onDismiss,
  onInstall,
}: {
  description: string;
  installing: boolean;
  name: string;
  onDismiss: () => void;
  onInstall: () => void;
}) {
  const { t } = useTranslation("agent");
  return (
    <div className="mb-2 flex items-start gap-3 rounded-md border border-focus/40 bg-focus-soft px-3 py-2.5">
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5 text-xs font-medium text-ink">
          <span className="text-focus">{t("composer.skillRecommend.cardTitle")}</span>
          <span className="font-mono text-ink">
            /
            {name}
          </span>
        </div>
        <p className="mt-0.5 line-clamp-2 text-xs text-ink-muted">
          {description}
        </p>
      </div>
      <div className="flex shrink-0 items-center gap-1.5">
        <button
          className="inline-flex items-center gap-1 rounded-md bg-focus px-2.5 py-1 text-xs font-medium text-on-accent transition hover:opacity-90 disabled:opacity-60"
          disabled={installing}
          onClick={onInstall}
          type="button"
        >
          {installing ? <Loader2 className="size-3 animate-spin" /> : null}
          {t("composer.skillRecommend.installAndUse")}
        </button>
        <button
          className="rounded-md border border-line px-2.5 py-1 text-xs text-ink-muted transition hover:bg-surface-raised disabled:opacity-60"
          disabled={installing}
          onClick={onDismiss}
          type="button"
        >
          {t("composer.skillRecommend.dismissAndSend")}
        </button>
      </div>
    </div>
  );
}
