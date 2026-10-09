import type { MergedConversation } from "../../features/remote-peer/mergeConversations";
import type { RemotePeer } from "../../features/remote-peer/remotePeerClient";
import { useTranslation } from "react-i18next";
import { iconGlyph } from "../../features/remote-peer/peerIcons";
import { cn } from "../../lib/cn";

/**
 * A remote conversation in the shared list.
 *
 * Its own component rather than a synthesized `StoredThread` fed to
 * `ThreadListItem`, because almost every affordance of the local row is wrong
 * here: rename/pin/delete act on the *local* store, the run menu reads the
 * local agent, and selection writes local state. A remote row can be opened and
 * read; the actions that make sense on it (rename, pin, delete) belong to the
 * host and are issued as commands, not through the local store.
 *
 * The badge is the icon the user chose for that machine. It replaces the
 * machine's name in the row because the row also holds a title and a relative
 * time, and a full name there would push the title out of view.
 */
export function RemoteConversationRow({
  active,
  conversation,
  onOpen,
  peer,
  unread,
}: {
  active: boolean;
  conversation: MergedConversation;
  onOpen: (conversation: MergedConversation) => void;
  peer: RemotePeer | undefined;
  unread?: boolean;
}) {
  const { t } = useTranslation("remotePeer");

  return (
    <button
      className={cn(
        "group flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-sm transition-colors",
        active ? "bg-surface-subtle text-ink" : "text-ink-soft hover:bg-surface-subtle hover:text-ink",
      )}
      onClick={() => onOpen(conversation)}
      title={`${conversation.title} · ${peer?.name ?? conversation.desktopId ?? ""}`}
      type="button"
    >
      <span aria-hidden className="shrink-0 text-xs" data-testid="remote-source-icon">
        {iconGlyph(peer?.icon)}
      </span>
      <span className="min-w-0 flex-1 truncate">{conversation.title}</span>
      {conversation.streaming
        ? (
            <span
              aria-label={t("statusRunning")}
              className="size-1.5 shrink-0 animate-pulse rounded-full bg-accent"
            />
          )
        : unread
          ? <span aria-label={t("statusUnread")} className="size-1.5 shrink-0 rounded-full bg-accent" />
          : null}
    </button>
  );
}
