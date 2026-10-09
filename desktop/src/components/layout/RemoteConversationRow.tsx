import type { MergedConversation } from "../../features/remote-peer/mergeConversations";
import type { RemotePeer } from "../../features/remote-peer/remotePeerClient";
import { MoreHorizontal } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { iconGlyph } from "../../features/remote-peer/peerIcons";
import { RemoteConversationMenu } from "../../features/remote-peer/RemoteConversationMenu";
import { cn } from "../../lib/cn";
import { useDismissableLayer } from "../../lib/useDismissableLayer";

/**
 * A remote conversation in the shared list.
 *
 * Its own component rather than a synthesized `StoredThread` fed to
 * `ThreadListItem`, because almost every affordance of the local row is wrong
 * here: rename/pin/delete act on the *local* store, the run menu reads the
 * local agent, and selection writes local state. A remote row reads, is
 * prompted, and its actions are commands issued to the host that owns it.
 *
 * The badge is the icon the user chose for that machine. It replaces the
 * machine's name in the row because the row also holds a title and a relative
 * time, and a full name there would push the title out of view.
 */
export function RemoteConversationRow({
  active,
  conversation,
  onChanged,
  onOpen,
  onOpenRename,
  peer,
  unread,
}: {
  active: boolean;
  conversation: MergedConversation;
  /** Re-read the owning host's catalogue after a row action. */
  onChanged: () => void;
  onOpen: (conversation: MergedConversation) => void;
  onOpenRename: (conversation: MergedConversation) => void;
  peer: RemotePeer | undefined;
  unread?: boolean;
}) {
  const { t } = useTranslation("remotePeer");
  const [menuOpen, setMenuOpen] = useState(false);
  const itemRef = useDismissableLayer<HTMLDivElement>({
    enabled: menuOpen,
    onDismiss: () => setMenuOpen(false),
  });

  return (
    <div
      ref={itemRef}
      className={cn(
        "group relative flex h-8 w-full items-center rounded-md text-sm transition-colors",
        active ? "bg-surface-subtle text-ink" : "text-ink-soft hover:bg-surface-subtle hover:text-ink",
      )}
    >
      <button
        className="flex h-8 min-w-0 flex-1 items-center gap-2 px-2 text-left"
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
      <button
        aria-label={t("conversationActions")}
        className={cn(
          "mr-1 flex size-6 shrink-0 items-center justify-center rounded text-ink-muted transition-opacity hover:bg-surface-subtle hover:text-ink",
          // Revealed on hover or while open, but always mounted: a control that
          // appears on hover must still be reachable by keyboard focus.
          menuOpen ? "opacity-100" : "opacity-0 group-hover:opacity-100 focus-visible:opacity-100",
        )}
        onClick={(event) => {
          // Without this the click would also reach the row button underneath
          // and open the conversation the user wanted to act on.
          event.stopPropagation();
          setMenuOpen(open => !open);
        }}
        type="button"
      >
        <MoreHorizontal className="size-3.5" />
      </button>
      {menuOpen
        ? (
            <RemoteConversationMenu
              conversation={conversation}
              onChanged={onChanged}
              onClose={() => setMenuOpen(false)}
              onOpenRename={() => {
                setMenuOpen(false);
                onOpenRename(conversation);
              }}
            />
          )
        : null}
    </div>
  );
}
