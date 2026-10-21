import type { ReactNode } from "react";
import type { RailView } from "./hooks/useRailView";
import { Folder, History, MessageSquare } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "../../lib/cn";

/**
 * The activity rail's view switch: workspaces / chats / recent.
 *
 * A row of its own above the list, with every name spelled out. An icon would
 * have cost the reader a hover to find out what it does, and the point of this
 * control is that the other lists are discoverable at all — the workspace tree
 * is the default and the reader has no other way to learn that another shape
 * exists. It is also the *only* control that answers "which list": the rail's
 * navigation no longer carries a workspace or a chat entry, because two controls
 * answering one question is how "the chats are inside the workspaces" happened.
 *
 * `aria-pressed` rather than a tablist: these are three independent on/off
 * answers to "which list", and a tab role would promise arrow-key movement and
 * a tab panel that this rail does not have.
 */
const OPTIONS: Array<{ id: RailView; icon: ReactNode; labelKey: string }> = [
  { id: "workspace", icon: <Folder className="size-3.5" />, labelKey: "activityRail.viewWorkspace" },
  { id: "chat", icon: <MessageSquare className="size-3.5" />, labelKey: "activityRail.viewChat" },
  { id: "recent", icon: <History className="size-3.5" />, labelKey: "activityRail.viewRecent" },
];

export function RailViewSwitch({
  onChange,
  view,
}: {
  onChange: (view: RailView) => void;
  view: RailView;
}) {
  const { t } = useTranslation("layout");
  return (
    <div
      aria-label={t("activityRail.viewSwitch")}
      className="flex h-7 shrink-0 items-center gap-0.5 rounded-md bg-surface-subtle p-0.5"
      data-testid="rail-view-switch"
      role="group"
    >
      {OPTIONS.map((option) => {
        const selected = view === option.id;
        return (
          <button
            aria-pressed={selected}
            className={cn(
              "flex h-6 min-w-0 flex-1 items-center justify-center gap-1 rounded px-1 text-xs font-medium transition-colors",
              selected
                ? "bg-surface text-ink shadow-sm"
                : "text-ink-muted hover:text-ink-soft",
            )}
            data-testid={`rail-view-${option.id}`}
            key={option.id}
            onClick={() => onChange(option.id)}
            title={t(option.labelKey)}
            type="button"
          >
            {/* The glyph is the tab's icon as much as its decoration: three
                two-character labels in a 224px rail leave no room for a name
                to be readable, and the icon is what tells them apart at a
                glance once the reader has used them once. `aria-hidden`
                because the name beside it says the same thing. */}
            <span aria-hidden className="shrink-0">{option.icon}</span>
            <span className="truncate">{t(option.labelKey)}</span>
          </button>
        );
      })}
    </div>
  );
}
