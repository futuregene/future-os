import type { RemoteCatalog, RemoteSessionRow } from "../remote-peer/mergeConversations";
import type { RemotePeer } from "../remote-peer/remotePeerClient";
import { useCallback } from "react";
import { RemoteConversationView } from "../remote-peer/RemoteConversationView";
import { continueRemoteRun } from "../remote-peer/remotePeerClient";
import { useRemoteApprovals } from "../remote-peer/useRemoteApprovals";
import { useRemoteTimeline } from "../remote-peer/useRemoteTimeline";

/**
 * One host's conversation as a cell of the panel.
 *
 * The same adapter shape as `VibeThreadView` for a local conversation: it answers
 * the questions `RemoteConversationView` cannot answer for itself in a grid — which
 * host, whether this is the only conversation of that host on screen — and hands
 * everything else to the view the main window already uses. A second remote
 * transcript would re-earn every scroll, paging and approval bug that view has fixed.
 *
 * What it deliberately does **not** offer: a composer. A remote conversation in the
 * panel is for watching and for answering approvals; dictation stays on this machine,
 * which is also what keeps the panel's "one editable element" rule true across both
 * halves (`VIBE.md` §5).
 */
export function VibeRemoteCell({
  catalog,
  onOpenInShell,
  peer,
  session,
}: {
  catalog: RemoteCatalog;
  onOpenInShell: () => void;
  peer: RemotePeer | undefined;
  session: RemoteSessionRow;
}) {
  const desktopId = catalog.desktopId;
  const timeline = useRemoteTimeline(desktopId, session.sessionId, true);
  const approvals = useRemoteApprovals(desktopId, session.sessionId);

  // A page above the loaded window, and the same request from the top of the
  // transcript: the view already asks for it (`onLoadOlder`), and this is the cell
  // forwarding it. The promise is returned so the view can land on what it fetched.
  const loadOlder = useCallback(() => timeline.loadOlder(), [timeline]);

  return (
    <RemoteConversationView
      approvals={approvals.approvals}
      approvalErrors={approvals.errors}
      approvalPending={approvals.pending}
      chrome="read"
      // No composer in a cell. The panel's input layer is for this machine, and this
      // is what keeps a grid of remote conversations from mounting N editables.
      composer={null}
      desktopId={desktopId}
      error={timeline.error}
      hasMore={timeline.hasMore}
      leftPanelExpanded
      liveReplies={timeline.liveReplies}
      loading={timeline.loading}
      loadingOlder={timeline.loadingOlder}
      messages={timeline.messages}
      onContinueRun={runId => void continueRemoteRun(desktopId, session.sessionId, runId)}
      onDecideApproval={(approval, decision) => void approvals.decide(approval, decision)}
      onFork={() => onOpenInShell()}
      onLoadOlder={loadOlder}
      onOpenUsage={() => onOpenInShell()}
      onRetry={() => void timeline.refresh()}
      onToggleLeftPanel={() => {}}
      olderHistoryFailed={timeline.olderHistoryFailed}
      peer={peer}
      persistedEntryIds={timeline.persistedEntryIds}
      // Several conversations of this host may be on screen at once, so the lane must
      // stay wide: narrowing it would route the other cells' events away from them.
      selectiveEvents={false}
      sessionId={session.sessionId}
      streaming={timeline.streaming}
      title={session.title}
      workspaceName={session.workspaceName ?? null}
    />
  );
}
