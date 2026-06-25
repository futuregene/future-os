import { PanelLeftOpen } from "lucide-react";
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
  if (expanded)
    return null;

  return (
    <div className={cn("flex h-12 shrink-0 items-center", isMacOS ? "w-28 pl-[64px]" : "pl-2")}>
      <button
        aria-label="Show sidebar"
        className="inline-flex size-8 -mt-1 items-center justify-center rounded-md border border-transparent text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
        onClick={onToggle}
        onMouseDown={event => event.stopPropagation()}
        title="Show sidebar"
        type="button"
      >
        <PanelLeftOpen className="size-3.5" />
      </button>
    </div>
  );
}
