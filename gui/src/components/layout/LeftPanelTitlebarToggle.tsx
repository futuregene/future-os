import { PanelLeft } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "../../lib/cn";
import { isMacOS } from "../../lib/platform";
import { useIsFullscreen } from "../../lib/useIsFullscreen";

interface LeftPanelTitlebarToggleProps {
  expanded: boolean;
  onToggle: () => void;
}

export function LeftPanelTitlebarToggle({
  expanded,
  onToggle,
}: LeftPanelTitlebarToggleProps) {
  const { t } = useTranslation("layout");
  // Reserve the top-left inset for the macOS traffic lights, except in
  // fullscreen where the lights are hidden and the inset is dead space.
  const isFullscreen = useIsFullscreen();
  const reserveTrafficLights = isMacOS && !isFullscreen;
  if (expanded)
    return null;

  return (
    <div className={cn("flex h-12 shrink-0 items-center", reserveTrafficLights ? "w-28 pl-16" : "w-28")}>
      <button
        aria-label={t("leftPanelTitlebarToggle.showSidebar")}
        className="inline-flex size-8 items-center justify-center rounded-md border border-transparent text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
        onClick={onToggle}
        onMouseDown={event => event.stopPropagation()}
        title={t("leftPanelTitlebarToggle.showSidebar")}
        type="button"
      >
        <PanelLeft className="size-3.5" />
      </button>
    </div>
  );
}
