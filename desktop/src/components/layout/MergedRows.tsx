import type { ReactNode } from "react";
import type { MergedConversation } from "../../features/remote-peer/mergeConversations";
import type { RemotePeer } from "../../features/remote-peer/remotePeerClient";
import type { StoredThread } from "../../integrations/storage/threadStore";
import { useTranslation } from "react-i18next";
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
  onOpenRemote,
  peers,
  renderLocalRow,
  rows,
  threadById,
}: {
  activeKey: string | null;
  filter: unknown;
  onOpenRemote: (conversation: MergedConversation) => void;
  peers: RemotePeer[];
  renderLocalRow: (thread: StoredThread, depth?: number, hasChildren?: boolean) => ReactNode;
  rows: MergedConversation[];
  threadById: Map<string, StoredThread>;
}) {
  const { t } = useTranslation("layout");
  const pinned = rows.filter(row => row.pinned);
  const rest = rows.filter(row => !row.pinned);

  function renderRow(conversation: MergedConversation): ReactNode {
    if (conversation.desktopId === null) {
      const thread = threadById.get(conversation.id);
      // A local row without a thread cannot happen (the merge is built from the
      // same list), but rendering nothing beats rendering a broken row.
      return thread ? renderLocalRow(thread) : null;
    }
    return (
      <RemoteConversationRow
        active={activeKey === conversation.key}
        conversation={conversation}
        key={conversation.key}
        onOpen={onOpenRemote}
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
      <div className="shrink-0 space-y-0.5" data-testid="merged-conversations">
        {rest.map(renderRow)}
      </div>
    </>
  );
}
