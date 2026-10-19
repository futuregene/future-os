import type { RemoteSlashItem, RemoteSlashMenuGroups } from "./remoteCommands";
import { Archive, Blocks } from "lucide-react";
import { useTranslation } from "react-i18next";
import { remoteSlashItems } from "./remoteCommands";

/**
 * The menu itself: what the host can be asked for, above the box.
 *
 * Above rather than below, because the box sits at the foot of the window and a
 * menu opening downward would open off-screen.
 *
 * The same shape as this app's own `/` menu (`MentionMenus.SlashMenu`): the
 * commands that run, then the skills that are named, with the second group's
 * heading drawn only when it has to be told apart from the first. The skills are
 * there for the same reason the phone has them in its picker: a skill is used by
 * naming it, and a user who has to remember the name has no menu at all.
 */
export function RemoteSlashMenu({
  activeIndex,
  groups,
  onChoose,
}: {
  activeIndex: number;
  groups: RemoteSlashMenuGroups;
  onChoose: (item: RemoteSlashItem) => void;
}) {
  const { t } = useTranslation("remotePeer");
  const items = remoteSlashItems(groups);
  if (items.length === 0)
    return null;
  // The panel's own header names what leads it; the skills get a heading of
  // their own only when a command came first.
  const title = groups.commands.length > 0 ? t("slashCommands") : t("slashSkills");
  return (
    <div
      className="absolute bottom-full left-1 mb-2 max-h-72 w-72 overflow-y-auto rounded-lg border border-line bg-surface p-1 shadow-panel"
      role="listbox"
    >
      <div className="px-2 py-1 text-2xs font-medium text-ink-muted">{title}</div>
      {items.map((item, index) => (
        <div key={item.kind === "command" ? `command:${item.command.id}` : `skill:${item.skill.name}`}>
          {index > 0 && index === groups.commands.length
            ? <div className="px-2 pb-1 pt-2 text-2xs font-medium text-ink-muted">{t("slashSkills")}</div>
            : null}
          <button
            aria-selected={index === activeIndex}
            className={`flex w-full items-start gap-2 rounded-md px-2 py-1.5 text-left text-sm ${
              index === activeIndex ? "bg-surface-subtle text-ink" : "text-ink-soft hover:bg-surface-subtle hover:text-ink"
            }`}
            // Keep the field's caret: the chosen skill is inserted where the word
            // was typed, and the message continues from there. Without this the
            // click moves focus to the button and the next keystroke is lost.
            onMouseDown={event => event.preventDefault()}
            onClick={() => onChoose(item)}
            role="option"
            type="button"
          >
            {/* The icon is looked up here rather than carried on the item, so the
                list a draft matches against stays free of elements. */}
            {item.kind === "command"
              ? (item.command.id === "compact" ? <Archive aria-hidden className="mt-0.5 size-4 shrink-0 text-ink-soft" /> : null)
              : <Blocks aria-hidden className="mt-0.5 size-4 shrink-0 text-ink-soft" />}
            <span className="min-w-0 flex-1">
              <span className="block truncate font-medium">
                {item.kind === "command" ? item.command.label : `/${item.skill.name}`}
              </span>
              {/* The command's hint, or what the skill does — the row a reader
                  decides on. A name alone ("future-peer-review") says nothing. */}
              <span className="block truncate text-xs text-ink-muted">
                {item.kind === "command" ? item.command.description : item.skill.description}
              </span>
            </span>
          </button>
        </div>
      ))}
    </div>
  );
}
