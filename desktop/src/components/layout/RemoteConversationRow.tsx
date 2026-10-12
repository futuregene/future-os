import type { MergedConversation } from "../../features/remote-peer/mergeConversations";
import type { RemotePeer } from "../../features/remote-peer/remotePeerClient";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { iconGlyph } from "../../features/remote-peer/peerIcons";
import { RemoteConversationMenu } from "../../features/remote-peer/RemoteConversationMenu";
import { useDismissableLayer } from "../../lib/useDismissableLayer";
import { RailRow, RailRowIndicator, RailRowMenuButton, RailRowTitle } from "./RailRow";

/**
 * A remote conversation in the shared list.
 *
 * Its own component rather than a synthesized `StoredThread` fed to
 * `ThreadListItem`, because almost every affordance of the local row is wrong
 * here: rename/pin/delete act on the *local* store, the run menu reads the
 * local agent, and selection writes local state. A remote row reads, is
 * prompted, and its actions are commands issued to the host that owns it.
 *
 * It is built on the same `RailRow` as the local row, though — the frame, the
 * run indicator's reserved slot and the actions trigger are shared, because
 * those are what a reader sees when scanning a list that holds both kinds. The
 * parts that differ are the parts that mean something different: the tree toggle
 * (a remote session has no local tree), the selection checkbox (selection is a
 * local-store operation) and the machine's icon.
 */
export function RemoteConversationRow({
  active,
  compact,
  conversation,
  depth = 0,
  onChanged,
  onOpen,
  onOpenRename,
  peer,
  showSource,
  unread,
}: {
  active: boolean;
  /** Whether this row sits inside a workspace group, as the local row does. */
  compact?: boolean;
  conversation: MergedConversation;
  depth?: number;
  /** Re-read the owning host's catalogue after a row action. */
  onChanged: () => void;
  onOpen: (conversation: MergedConversation) => void;
  onOpenRename: (conversation: MergedConversation) => void;
  peer: RemotePeer | undefined;
  /**
   * Whether the row still needs to say which machine it is on.
   *
   * False when the list is already filtered to one machine: every row is that
   * machine's, so the badge repeats what the selector above says. The title
   * does not carry the machine either way (the row has a title and a time
   * already), so the badge is only ever a qualifier.
   */
  showSource?: boolean;
  unread?: boolean;
}) {
  const { t } = useTranslation("remotePeer");
  const [menuOpen, setMenuOpen] = useState(false);
  const itemRef = useDismissableLayer<HTMLDivElement>({
    enabled: menuOpen,
    onDismiss: () => setMenuOpen(false),
  });

  return (
    <RailRow
      active={active}
      compact={compact}
      depth={depth}
      rowRef={itemRef}
      title={`${conversation.title} · ${peer?.name ?? conversation.desktopId ?? ""}`}
      onContextMenu={() => setMenuOpen(true)}
      onSelect={() => onOpen(conversation)}
    >
      <RailRowTitle>{conversation.title}</RailRowTitle>
      {/* The machine, after the title: the title is what the row is about, and a
          leading glyph would push every title in the list one indent in. */}
      {showSource === false
        ? null
        : (
            <span aria-hidden className="pointer-events-none shrink-0 text-xs" data-testid="remote-source-icon">
              {iconGlyph(peer?.icon)}
            </span>
          )}
      {/* A remote host reports that a run is in flight, so the row uses the same
          indicator the local one does for it — an animated ring in the reserved
          slot, not a dot of its own size in a slot of its own size. */}
      <RailRowIndicator status={conversation.streaming ? "running" : unread ? "completed" : undefined} unread={unread} />
      <RailRowMenuButton
        label={t("conversationActions")}
        menuOpen={menuOpen}
        onToggle={() => setMenuOpen(open => !open)}
      />
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
    </RailRow>
  );
}
