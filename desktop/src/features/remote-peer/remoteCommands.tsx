import type { ReactNode } from "react";
import { Archive } from "lucide-react";
import { useTranslation } from "react-i18next";

/**
 * The commands a remote composer accepts after `/`.
 *
 * A short list, and deliberately not the local one: this app's slash menu offers
 * *tools and skills* its own agent can run, and none of those exist on the host.
 * What a host does accept is a handful of conversation-level operations, of which
 * one — compacting the context — previously had a button in the header, where a
 * conversation-level action reads as a mode switch on the whole conversation
 * rather than as something you ask for.
 */
export interface RemoteCommand {
  id: string;
  /** Localized label. */
  label: string;
  description: string;
  icon: ReactNode;
  run: () => void;
}

export interface RemoteSlashMenuGroups {
  commands: RemoteCommand[];
}

/**
 * The commands a draft's slash word matches.
 *
 * The query is what follows the slash, so `/co` finds Compact — the same
 * substring rule the local menu uses, because a menu that matched differently
 * would make one `/` mean two things in two composers.
 */
export function buildRemoteSlashMenu(query: string, commands: RemoteCommand[]): RemoteSlashMenuGroups {
  const needle = query.toLowerCase();
  return {
    commands: commands.filter(command =>
      !needle
      || command.id.toLowerCase().includes(needle)
      || command.label.toLowerCase().includes(needle)),
  };
}

/**
 * The slash word a draft is currently typing, or null.
 *
 * Only a slash that begins the draft (after whitespace) counts: a `/` inside a
 * sentence — a path, a fraction — must not open a menu over the message being
 * written.
 */
export function slashQuery(draft: string): string | null {
  const match = /(?:^|\s)\/(\S*)$/.exec(draft);
  return match ? match[1]! : null;
}

/** The draft without its slash word, for when a command is chosen. */
export function withoutSlashWord(draft: string): string {
  return draft.replace(/(^|\s)\/\S*$/, "$1").trimEnd();
}

/**
 * The menu itself: one group of host commands, above the box.
 *
 * Above rather than below, because the box is at the bottom of the window and a
 * menu that opened downward would open off-screen.
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
          className={`flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm ${
            index === activeIndex ? "bg-surface-subtle text-ink" : "text-ink-soft hover:bg-surface-subtle hover:text-ink"
          }`}
          key={command.id}
          onClick={() => onChoose(command)}
          role="option"
          aria-selected={index === activeIndex}
          type="button"
        >
          {command.icon}
          <span className="min-w-0 flex-1 truncate">{command.label}</span>
        </button>
      ))}
    </div>
  );
}

/**
 * The commands this composer offers, built from what the host accepts.
 *
 * A command whose host operation is unavailable (no conversation yet, a run in
 * flight) is left out rather than shown disabled: a slash menu is a list of
 * things you can ask for.
 */
export function useRemoteCommands({
  canCompact,
  onCompact,
}: {
  canCompact: boolean;
  onCompact: () => void;
}): RemoteCommand[] {
  const { t } = useTranslation("remotePeer");
  const commands: RemoteCommand[] = [];
  if (canCompact) {
    commands.push({
      description: t("compactHint"),
      icon: <Archive aria-hidden className="size-4 shrink-0 text-ink-soft" />,
      id: "compact",
      label: t("compact"),
      run: onCompact,
    });
  }
  return commands;
}
