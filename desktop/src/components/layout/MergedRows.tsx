import type { ReactNode } from "react";
import type { DeviceFilter, MergedConversation } from "../../features/remote-peer/mergeConversations";
import type { RemotePeer } from "../../features/remote-peer/remotePeerClient";
import type { StoredThread } from "../../integrations/storage/threadStore";
import type { ThreadTreeRow } from "./threadTree";
import { Plus } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { groupMergedRows } from "../../features/remote-peer/mergeConversations";
import { iconGlyph } from "../../features/remote-peer/peerIcons";
import { RemoteWorkspaceMenu } from "../../features/remote-peer/RemoteWorkspaceMenu";
import { ChatSectionMenu } from "./ActivityRailMenus";
import { RailGroupHeader, RailHeaderButton, RailSectionHeader, RailSectionToggle } from "./RailChrome";
import { RemoteConversationRow } from "./RemoteConversationRow";

/**
 * The merged list's rows: pinned, then workspaces, then chats.
 *
 * The same shape, headings, folding and controls as the local list, because they
 * are the same list to the reader. Where it differs, it says why:
 *
 * - A group is keyed by `(machine, workspace id)`, not by name: a workspace
 *   called "future-os" exists on two machines at once, and merging them would
 *   file one machine's rows under the other's heading.
 * - A group heading carries the machine's icon (after the name), since with
 *   several machines in one list the name alone does not say where it is from.
 *
 * Local rows are supplied by the rail as already-built nodes, and so are local
 * group headings. That is deliberate: the rail owns their props (selection mode,
 * approvals, run status, unread, the workspace menu), and rebuilding them here
 * would mean two places to update whenever either gains a prop — which is how
 * one list ends up subtly different from the other.
 *
 * **The fold state is the rail's, passed in.** This component used to call
 * `useCollapsedWorkspaces` again, which is a *second* `useState` over the same
 * localStorage key — so the rail's copy drove the heading (chevron, label) and
 * this copy drove the rows. They agree only at mount, and then: folding a
 * workspace flipped the heading while its conversations stayed, and expanding
 * one after a restart flipped the heading while the conversations stayed hidden.
 * That is the reported "a local workspace's list is empty", and it is why there
 * is one instance now.
 */
export function MergedRows({
  activeKey,
  chatSectionCollapsed,
  collapsedWorkspaces,
  filter,
  onChangedRemote,
  onNewChat,
  onNewChatInWorkspace,
  onNewWorkspace,
  onOpenRemote,
  onRenameRemote,
  onSelectChats,
  onToggleChatSection,
  onToggleWorkspace,
  onToggleWorkspaceSection,
  peers,
  pinnedWorkspaces,
  renderLocalGroupHeader,
  renderLocalRow,
  rows,
  threadById,
  localRowMeta,
  workspaceSectionCollapsed,
}: {
  activeKey: string | null;
  chatSectionCollapsed: boolean;
  /** The rail's fold state: the same instance that draws the local headings. */
  collapsedWorkspaces: ReadonlySet<string>;
  /**
   * Which machines the list is showing.
   *
   * Read for two decisions: a per-row badge repeats the selector above it when
   * the list is already one machine's, and the local-only controls (new
   * workspace, new chat) are offered only when this machine's rows are in the
   * list at all.
   */
  filter: DeviceFilter;
  /** Re-read a host's catalogue after one of its rows was acted on. */
  onChangedRemote: () => void;
  onNewChat?: () => void;
  /** Start a conversation in a workspace on another machine. */
  onNewChatInWorkspace?: (desktopId: string, workspaceId: string) => void;
  onNewWorkspace?: () => void;
  onOpenRemote: (conversation: MergedConversation) => void;
  onRenameRemote: (conversation: MergedConversation) => void;
  /** Mark this machine's chats for a batch action (a local-only operation). */
  onSelectChats?: () => void;
  onToggleChatSection: () => void;
  onToggleWorkspace: (key: string) => void;
  onToggleWorkspaceSection: () => void;
  /**
   * Where each local conversation sits in its own tree, keyed by thread id.
   *
   * The merged list is ordered by time, but a local conversation still *has* a
   * parent — and without this a fork rendered as a root, with no indent and no
   * fold toggle.
   */
  localRowMeta: Map<string, ThreadTreeRow>;
  peers: RemotePeer[];
  /** This machine's pinned workspaces, so their groups lead the list. */
  pinnedWorkspaces: ReadonlySet<string>;
  /** The local rail's own workspace heading, so both lists show one heading. */
  renderLocalGroupHeader: (workspaceId: string) => ReactNode;
  renderLocalRow: (thread: StoredThread, depth?: number, hasChildren?: boolean) => ReactNode;
  rows: MergedConversation[];
  threadById: Map<string, StoredThread>;
  workspaceSectionCollapsed: boolean;
}) {
  const { t } = useTranslation("layout");
  const { chats, pinned, workspaces } = groupMergedRows(rows, pinnedWorkspaces);
  // The chat section's menu is the rail's own control, which owns its open state
  // through props — so the state lives here, next to the header that draws it.
  const [chatMenuOpen, setChatMenuOpen] = useState(false);
  // One machine selected: it is named by the selector directly above the list, so
  // repeating it on every row says nothing and takes the space a title needs.
  const oneMachine = filter.kind === "device" && filter.desktopId !== null;
  // Creating a workspace or a chat makes one on *this* machine, so the controls
  // that do it belong only where this machine's rows are listed: under a remote
  // device's filter they would add something the user cannot see.
  const showLocalActions = filter.kind === "all" || filter.desktopId === null;

  function renderRow(conversation: MergedConversation, compact: boolean, groupHasSource: boolean): ReactNode {
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
        compact={compact}
        conversation={conversation}
        key={conversation.key}
        onChanged={onChangedRemote}
        onOpen={onOpenRemote}
        onOpenRename={onRenameRemote}
        peer={peers.find(peer => peer.desktopId === conversation.desktopId)}
        // The machine is named once per group, on the heading. Repeating it on
        // every row under that heading is the same word on every line — and the
        // row pays for it in the width its title needs.
        showSource={!oneMachine && !groupHasSource}
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
              <RailSectionHeader>{t("activityRail.pinnedHeader")}</RailSectionHeader>
              {pinned.map(row => renderRow(row, false, false))}
            </div>
          )
        : null}
      <div className="space-y-0.5">
        <RailSectionHeader
          controls={(
            <>
              <RailSectionToggle
                collapsed={workspaceSectionCollapsed}
                label={workspaceSectionCollapsed
                  ? t("activityRail.expandWorkspaceSection")
                  : t("activityRail.collapseWorkspaceSection")}
                onToggle={onToggleWorkspaceSection}
              />
              {showLocalActions && onNewWorkspace
                ? (
                    <RailHeaderButton
                      icon={<Plus className="size-3.5" />}
                      label={t("activityRail.newWorkspace")}
                      onClick={onNewWorkspace}
                    />
                  )
                : null}
            </>
          )}
        >
          {t("activityRail.workspace")}
        </RailSectionHeader>
        {!workspaceSectionCollapsed && workspaces.length === 0
          ? <div className="px-2 py-1 text-xs text-ink-muted">{t("activityRail.noWorkspaceThreads")}</div>
          : null}
        {workspaceSectionCollapsed
          ? null
          : workspaces.map((group) => {
              // A group of this machine's rows is this machine's workspace, and
              // the rail already renders that heading — with its menu, its
              // click-to-select and its pin marker. Using it here is what makes
              // the two lists one list rather than two that resemble each other.
              if (group.desktopId === null && group.workspaceId !== null) {
                const collapsed = collapsedWorkspaces.has(group.workspaceId);
                return (
                  <div className="space-y-0.5" key={group.key}>
                    {renderLocalGroupHeader(group.workspaceId)}
                    {collapsed
                      ? null
                      : group.rows.map(row => renderRow(row, true, false))}
                  </div>
                );
              }
              const collapsed = collapsedWorkspaces.has(group.key);
              const desktopId = group.desktopId;
              const workspaceId = group.workspaceId;
              return (
                <div className="space-y-0.5" key={group.key}>
                  <RailGroupHeader
                    collapsed={collapsed}
                    collapsedLabel={t("activityRail.expandWorkspace")}
                    expandedLabel={t("activityRail.collapseWorkspace")}
                    glyph={oneMachine ? null : <GroupGlyph desktopId={desktopId} peers={peers} />}
                    menu={desktopId && workspaceId
                      ? (
                          <>
                            {/* A workspace on another machine takes the same two
                                controls the local one has: a menu for what the
                                host accepts, and a way to start a conversation
                                in it. */}
                            <RailHeaderButton
                              icon={<Plus className="size-3.5" />}
                              label={t("activityRail.newChatInWorkspace", {
                                name: group.label ?? t("activityRail.workspace"),
                              })}
                              onClick={() => onNewChatInWorkspace?.(desktopId, workspaceId)}
                            />
                            <RemoteWorkspaceMenu
                              desktopId={desktopId}
                              name={group.label ?? t("activityRail.workspace")}
                              onChanged={onChangedRemote}
                              workspaceId={workspaceId}
                            />
                          </>
                        )
                      : null}
                    name={group.label ?? t("activityRail.workspace")}
                    onToggle={() => onToggleWorkspace(group.key)}
                    testId="merged-group-heading"
                  />
                  {collapsed ? null : group.rows.map(row => renderRow(row, true, true))}
                </div>
              );
            })}
      </div>
      <div className="mt-3 flex flex-1 flex-col space-y-0.5">
        <RailSectionHeader
          controls={(
            <>
              <RailSectionToggle
                collapsed={chatSectionCollapsed}
                label={chatSectionCollapsed
                  ? t("activityRail.expandChatSection")
                  : t("activityRail.collapseChatSection")}
                onToggle={onToggleChatSection}
              />
              {/* The chat section's own menu, as the local list has. It marks
                  conversations for a batch action, which is this machine's: a
                  remote conversation has no checkbox because the batch it would
                  join is a local-store operation. Offered only where this
                  machine's rows are listed, since a menu that could only act on
                  rows the filter has hidden would be a dead control. */}
              {showLocalActions && onSelectChats
                ? (
                    <ChatSectionMenu
                      open={chatMenuOpen}
                      onOpenChange={setChatMenuOpen}
                      onSelect={onSelectChats}
                    />
                  )
                : null}
              {showLocalActions && onNewChat
                ? (
                    <RailHeaderButton
                      icon={<Plus className="size-3.5" />}
                      label={t("activityRail.newChatShort")}
                      onClick={onNewChat}
                    />
                  )
                : null}
            </>
          )}
        >
          {t("activityRail.chatHeader")}
        </RailSectionHeader>
        {!chatSectionCollapsed && chats.length === 0
          ? <div className="px-2 py-1 text-xs text-ink-muted">{t("activityRail.noChats")}</div>
          : null}
        <div className="shrink-0 space-y-0.5" data-testid="merged-conversations">
          {chatSectionCollapsed ? null : chats.map(row => renderRow(row, false, false))}
        </div>
      </div>
    </>
  );
}

/**
 * Which machine a heading belongs to.
 *
 * Local headings are the rail's own; a remote one carries that machine's icon,
 * because with several machines in one list the name alone does not say which
 * pane a group came from.
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
