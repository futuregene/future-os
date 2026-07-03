import { PanelLeftOpen } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "../../lib/cn";
import { isMacOS } from "../../lib/platform";

interface LeftPanelTitlebarToggleProps {
  expanded: boolean;
  onToggle: () => void;
}

export function LeftPanelTitlebarToggle({
  expanded,
  onToggle,
}: LeftPanelTitlebarToggleProps) {
  const { t } = useTranslation("layout");
  if (expanded)
    return null;

  return (
    <div className={cn("flex h-12 shrink-0 items-center", isMacOS ? "w-28 pl-16" : "w-28")}>
      <button
        aria-label={t("leftPanelTitlebarToggle.showSidebar")}
        className="inline-flex size-8 items-center justify-center rounded-md border border-transparent text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
        onClick={onToggle}
        onMouseDown={event => event.stopPropagation()}
        title={t("leftPanelTitlebarToggle.showSidebar")}
        type="button"
      >
        <PanelLeftOpen className="size-3.5" />
      </button>
    </div>
  );
}
