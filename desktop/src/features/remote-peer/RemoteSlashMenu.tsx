import type { RemoteCommand, RemoteSlashMenuGroups } from "./remoteCommands";
import { Archive } from "lucide-react";
import { useTranslation } from "react-i18next";

/**
 * The menu itself: one group of host commands, above the box.
 *
 * Above rather than below, because the box sits at the foot of the window and a
 * menu opening downward would open off-screen.
 */
export function RemoteSlashMenu({
  activeIndex,
  groups,
  onChoose,
}: {
  activeIndex: number;
  groups: RemoteSlashMenuGroups;
  onChoose: (command: RemoteCommand) => void;
}) {
  const { t } = useTranslation("remotePeer");
  if (groups.commands.length === 0)
    return null;
  return (
    <div
      className="absolute bottom-full left-1 mb-2 w-64 overflow-hidden rounded-lg border border-line bg-surface p-1 shadow-panel"
      role="listbox"
    >
      <div className="px-2 py-1 text-[11px] font-medium text-ink-muted">{t("slashCommands")}</div>
      {groups.commands.map((command, index) => (
        <button
          aria-selected={index === activeIndex}
          className={`flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm ${
            index === activeIndex ? "bg-surface-subtle text-ink" : "text-ink-soft hover:bg-surface-subtle hover:text-ink"
          }`}
          key={command.id}
          onClick={() => onChoose(command)}
          role="option"
          type="button"
        >
          {/* The icon is looked up here rather than carried on the command, so
              the list a draft matches against stays free of elements. */}
          {command.id === "compact"
            ? <Archive aria-hidden className="size-4 shrink-0 text-ink-soft" />
            : null}
          <span className="min-w-0 flex-1 truncate">{command.label}</span>
        </button>
      ))}
    </div>
  );
}
