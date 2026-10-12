import type { ReactNode } from "react";
import type { DeviceFilter, MergedConversation } from "../../features/remote-peer/mergeConversations";
import type { RemotePeer } from "../../features/remote-peer/remotePeerClient";
import type { StoredThread } from "../../integrations/storage/threadStore";
import type { ThreadTreeRow } from "./threadTree";
import { ChevronDown, ChevronRight } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { groupMergedRows } from "../../features/remote-peer/mergeConversations";
import { iconGlyph, peerBadgeText } from "../../features/remote-peer/peerIcons";
import { useCollapsedWorkspaces } from "./hooks/useCollapsedWorkspaces";
import { RemoteConversationRow } from "./RemoteConversationRow";

/**
 * The merged list's rows: pinned, then workspaces, then chats.
 *
 * The same shape, headings and folding as the local list, because they are the
 * same list to the reader. Where it differs, it says why:
 *
 * - A group is keyed by `(machine, workspace id)`, not by name: a workspace
 *   called "future-os" exists on two machines at once, and merging them would
 *   file one machine's rows under the other's heading.
 * - A group heading carries the machine's icon (after the name), since with
 *   several machines in one list the name alone does not say where it is from.
 * - Fold state is per `(machine, workspace)` and kept in this component, the way
 *   the local list keeps its own — folding a group is a property of looking at
 *   the list, not of the conversations in it.
 *
 * Local rows are supplied by the rail as already-built nodes. That is
 * deliberate: the rail owns their props (selection mode, approvals, run status,
 * unread), and rebuilding them here would mean two places to update whenever a
 * row gains a prop — which is how one list ends up subtly different from the
 * other.
 */
export function MergedRows({
  activeKey,
  filter,
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
  /**
   * Which machines the list is showing.
   *
   * Read for one decision: filtered to a single machine, every remote row is
   * that machine's, so a per-row badge repeats the selector above it.
   */
  filter: DeviceFilter;
  /** Re-read a host's catalogue after one of its rows was acted on. */
  onChangedRemote: () => void;
  onOpenRemote: (conversation: MergedConversation) => void;
  /**
   * Where each local conversation sits in its own tree, keyed by thread id.
   *
   * The merged list is ordered by time, but a local conversation still *has* a
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
  // Fold state, keyed by group. Shared with the local list's workspace folding so
  // a workspace folded in one place is folded in the other — it is the same
  // workspace, and two independent fold states would disagree the moment a device
  // filter changed which list is drawn.
  const { collapsedWorkspaces, toggleWorkspaceCollapsed } = useCollapsedWorkspaces();
  const [collapsedChats, setCollapsedChats] = useState<ReadonlySet<string>>(() => new Set());
  // One machine selected: it is named by the selector directly above the list, so
  // repeating it on every row says nothing and takes the space a title needs.
  const showSource = !(filter.kind === "device" && filter.desktopId !== null);

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
        showSource={showSource}
      />
    );
  }

  /** One foldable heading, shaped like the local list's workspace header. */
  function groupHeader(
    collapsed: boolean,
    onToggle: () => void,
    label: ReactNode,
    glyph: ReactNode,
    key: string,
  ): ReactNode {
    return (
      <div className="group flex h-7 w-full items-center gap-1 rounded-md px-2 text-left" key={key}>
        <button
          aria-expanded={!collapsed}
          aria-label={t(collapsed ? "activityRail.expandWorkspace" : "activityRail.collapseWorkspace")}
          className="inline-flex size-4 shrink-0 items-center justify-center text-ink-muted transition-colors hover:text-ink-soft"
          onClick={onToggle}
          type="button"
        >
          {collapsed ? <ChevronRight className="size-3.5" /> : <ChevronDown className="size-3.5" />}
        </button>
        <span className="min-w-0 flex-1 truncate text-sm font-medium text-ink-soft" data-testid="merged-group-heading">
          {label}
        </span>
        {/* The machine, after the name: the name says which workspace, this says
            whose. It is dropped when only one machine is listed, because the
            selector above already says it. */}
        {glyph}
      </div>
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
      {workspaces.map((group) => {
        const collapsed = collapsedWorkspaces.has(group.key);
        return (
          <div className="mb-3 space-y-0.5" key={group.key}>
            {groupHeader(
              collapsed,
              () => toggleWorkspaceCollapsed(group.key),
              group.label ?? t("activityRail.workspace"),
              showSource ? <GroupGlyph desktopId={group.desktopId} peers={peers} /> : null,
              group.key,
            )}
            {collapsed ? null : group.rows.map(renderRow)}
          </div>
        );
      })}
      {chats.map((group) => {
        const collapsed = collapsedChats.has(group.key);
        const label = group.desktopId === null
          ? t("activityRail.chatHeader")
          : peerName(group.desktopId, peers) ?? t("activityRail.chatHeader");
        return (
          <div className="shrink-0 space-y-0.5" data-testid="merged-conversations" key={group.key}>
            {groupHeader(
              collapsed,
              () => setCollapsedChats((current) => {
                const next = new Set(current);
                if (next.has(group.key))
                  next.delete(group.key);
                else
                  next.add(group.key);
                return next;
              }),
              label,
              showSource ? <GroupGlyph desktopId={group.desktopId} peers={peers} /> : null,
              group.key,
            )}
            {collapsed ? null : group.rows.map(renderRow)}
          </div>
        );
      })}
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
