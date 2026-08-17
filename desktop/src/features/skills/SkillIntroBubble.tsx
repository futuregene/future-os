import { PartyPopper } from "lucide-react";
import { useTranslation } from "react-i18next";

interface SkillIntroBubbleProps {
  /** Currently installed skills (drives the title variant). */
  count: number;
  /** 「去看看」 — navigate to the Skills page and dismiss the intro. */
  onGo: () => void;
  /** 「知道了」 — dismiss the intro, stay on the page. */
  onDismiss: () => void;
}

/**
 * One-time intro bubble anchored to the rail's Skills nav entry (the caller
 * renders it inside a `relative` wrapper). Points at the entry with a left
 * caret. Dismissal is explicit only — the two buttons — so an accidental
 * outside click can't permanently hide the intro.
 */
export function SkillIntroBubble({ count, onGo, onDismiss }: SkillIntroBubbleProps) {
  const { t } = useTranslation("layout");

  return (
    <div className="absolute left-full top-0 z-30 ml-3 w-72 rounded-lg border border-line-soft bg-surface p-3 shadow-panel">
      {/* Left-pointing caret aligned with the nav row. */}
      <div className="absolute left-[-5px] top-3.5 size-2.5 rotate-45 border-b border-l border-line-soft bg-surface" />
      <div className="flex items-center gap-1.5">
        <PartyPopper className="size-4 shrink-0 text-accent" />
        <span className="text-sm font-semibold text-ink">
          {count > 0 ? t("skillIntro.titleWithCount", { count }) : t("skillIntro.titleEmpty")}
        </span>
      </div>
      <p className="mt-1.5 text-xs leading-5 text-ink-soft">{t("skillIntro.body")}</p>
      <div className="mt-2.5 flex items-center gap-2">
        <button
          className="inline-flex h-7 items-center rounded-md bg-accent px-3 text-sm font-medium text-white transition-colors hover:bg-accent-hover"
          onClick={onGo}
          type="button"
        >
          {t("skillIntro.go")}
        </button>
        <button
          className="inline-flex h-7 items-center rounded-md px-2 text-sm font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
          onClick={onDismiss}
          type="button"
        >
          {t("skillIntro.dismiss")}
        </button>
      </div>
    </div>
  );
}
