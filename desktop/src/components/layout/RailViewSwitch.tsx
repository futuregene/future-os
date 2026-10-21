import type { RailView } from "./hooks/useRailView";
import { useTranslation } from "react-i18next";
import { cn } from "../../lib/cn";

/**
 * The activity rail's view switch: workspaces ⇄ recent.
 *
 * A row of its own above the list, with both names spelled out. An icon would
 * have cost the reader a hover to find out what it does, and the point of this
 * control is that the second view is discoverable at all — the tree is the
 * default and the reader has no other way to learn that another shape exists.
 *
 * `aria-pressed` rather than a tablist: these are two independent on/off answers
 * to "which list", and a tab role would promise arrow-key movement and a tab
 * panel that this rail does not have.
 */
export function RailViewSwitch({
  onChange,
  view,
}: {
  onChange: (view: RailView) => void;
  view: RailView;
}) {
  const { t } = useTranslation("layout");
  const options: Array<{ id: RailView; label: string }> = [
    { id: "workspace", label: t("activityRail.viewWorkspace") },
    { id: "recent", label: t("activityRail.viewRecent") },
  ];
  return (
    <div
      aria-label={t("activityRail.viewSwitch")}
      className="mb-2 flex h-7 shrink-0 items-center gap-0.5 rounded-md bg-surface-subtle p-0.5"
      data-testid="rail-view-switch"
      role="group"
    >
      {options.map((option) => {
        const selected = view === option.id;
        return (
          <button
            aria-pressed={selected}
            className={cn(
              "flex h-6 min-w-0 flex-1 items-center justify-center rounded px-1 text-xs font-medium transition-colors",
              selected
                ? "bg-surface text-ink shadow-sm"
                : "text-ink-muted hover:text-ink-soft",
            )}
            data-testid={`rail-view-${option.id}`}
            key={option.id}
            onClick={() => onChange(option.id)}
            type="button"
          >
            <span className="truncate">{option.label}</span>
          </button>
        );
      })}
    </div>
  );
}
