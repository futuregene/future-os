import type { ReactNode } from "react";
import type { MergedConversation } from "../../features/remote-peer/mergeConversations";
import type { RemotePeer } from "../../features/remote-peer/remotePeerClient";
import type { StoredThread } from "../../integrations/storage/threadStore";
import type { ThreadTreeRow } from "./threadTree";
import { useTranslation } from "react-i18next";
import { groupMergedRows } from "../../features/remote-peer/mergeConversations";
import { iconGlyph, peerBadgeText } from "../../features/remote-peer/peerIcons";
import { RemoteConversationRow } from "./RemoteConversationRow";

/**
 * The merged list's rows, in the same two-section shape the local list uses:
 * pinned, then recency.
 *
 * Local rows are supplied by the rail as already-built nodes. That is
 * deliberate: the rail owns their props (selection mode, approvals, run
 * status, unread), and rebuilding them here would mean two places to update
 * whenever a row gains a prop — which is how one list ends up subtly different
 * from the other.
 */
export function MergedRows({
  activeKey,
  onChangedRemote,
  onOpenRemote,
  localRowMeta,
  onRenameRemote,
  peers,
  renderLocalRow,
  rows,
  threadById,
}: {
  activeKey: string | null;
  filter: unknown;
  /** Re-read a host's catalogue after one of its rows was acted on. */
  onChangedRemote: () => void;
  onOpenRemote: (conversation: MergedConversation) => void;
  /**
   * Where each local conversation sits in its own tree, keyed by thread id.
   *
   * The merged list is ordered by time, but a local conversation still has a
   * parent — and without this a fork rendered as a root, with no indent and no
   * fold toggle.
   */
  localRowMeta: Map<string, ThreadTreeRow>;
  onRenameRemote: (conversation: MergedConversation) => void;
  peers: RemotePeer[];
  renderLocalRow: (thread: StoredThread, depth?: number, hasChildren?: boolean) => ReactNode;
  rows: MergedConversation[];
  threadById: Map<string, StoredThread>;
}) {
  const { t } = useTranslation("layout");
  const { chats, pinned, workspaces } = groupMergedRows(rows);

  function renderRow(conversation: MergedConversation): ReactNode {
    if (conversation.desktopId === null) {
      const thread = threadById.get(conversation.id);
      // A local row without a thread cannot happen (the merge is built from the
      // same list), but rendering nothing beats rendering a broken row.
      if (!thread)
        return null;
      const meta = localRowMeta.get(thread.id);
      return renderLocalRow(thread, meta?.depth ?? 0, meta?.hasChildren ?? false);
    }
    return (
      <RemoteConversationRow
        active={activeKey === conversation.key}
        conversation={conversation}
        key={conversation.key}
        onChanged={onChangedRemote}
        onOpen={onOpenRemote}
        onOpenRename={onRenameRemote}
        peer={peers.find(peer => peer.desktopId === conversation.desktopId)}
      />
    );
  }

  if (rows.length === 0) {
    return <div className="px-2 py-1 text-xs text-ink-muted">{t("activityRail.noChats")}</div>;
  }

  return (
    <>
      {pinned.length > 0
        ? (
            <div className="mb-3 space-y-0.5">
              <div className="sticky top-0 z-20 flex h-6 items-center bg-surface px-2 text-xs font-medium text-ink-muted">
                <span>{t("activityRail.pinnedHeader")}</span>
              </div>
              {pinned.map(renderRow)}
            </div>
          )
        : null}
      {workspaces.map(group => (
        <div className="mb-3 space-y-0.5" key={group.key}>
          <div className="flex h-6 items-center gap-1.5 bg-surface px-2 text-xs font-medium text-ink-muted">
            <GroupGlyph desktopId={group.desktopId} peers={peers} />
            <span className="min-w-0 truncate">{group.label ?? t("activityRail.workspace")}</span>
          </div>
          {group.rows.map(renderRow)}
        </div>
      ))}
      {chats.map(group => (
        <div className="shrink-0 space-y-0.5" data-testid="merged-conversations" key={group.key}>
          <div className="sticky top-0 z-20 flex h-6 items-center gap-1.5 bg-surface px-2 text-xs font-medium text-ink-muted">
            <GroupGlyph desktopId={group.desktopId} peers={peers} />
            <span className="min-w-0 truncate">
              {group.desktopId === null
                ? t("activityRail.chatHeader")
                : peerName(group.desktopId, peers) ?? t("activityRail.chatHeader")}
            </span>
          </div>
          {group.rows.map(renderRow)}
        </div>
      ))}
    </>
  );
}

/**
 * Which machine a heading belongs to.
 *
 * Local headings keep the plain indentation the local list has always had; a
 * remote one carries that machine's own icon, because with several machines in
 * one list the name alone does not say which pane a group came from.
 */
function GroupGlyph({ desktopId, peers }: { desktopId: string | null; peers: RemotePeer[] }) {
  if (desktopId === null)
    return null;
  return (
    <span aria-hidden className="shrink-0 text-xs">
      {iconGlyph(peers.find(peer => peer.desktopId === desktopId)?.icon)}
    </span>
  );
}

function peerName(desktopId: string, peers: RemotePeer[]): string | null {
  const peer = peers.find(item => item.desktopId === desktopId);
  if (!peer)
    return null;
  return peerBadgeText(peer, desktopId);
}
