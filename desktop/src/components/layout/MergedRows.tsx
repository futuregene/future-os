import type { ReactNode } from "react";
import type { DeviceFilter, MergedConversation, RemoteWorkspaceTarget } from "../../features/remote-peer/mergeConversations";
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
import { RailNewChatButton } from "./RailNewChatButton";
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
  onRenameRemoteWorkspace,
  onSelectChats,
  onSelectWorkspaceChats,
  onToggleChatSection,
  onToggleRemoteExpanded,
  onToggleWorkspace,
  onToggleWorkspaceSection,
  peers,
  remoteSelection,
  renderLocalGroupHeader,
  renderLocalRow,
  rows,
  selectionMode,
  threadById,
  localRowMeta,
  expandedRemoteRows,
  remoteRowMeta,
  workspaceSectionCollapsed,
}: {
  activeKey: string | null;
  chatSectionCollapsed: boolean;
  /** The rail's fold state: the same instance that draws the local headings. */
  collapsedWorkspaces: ReadonlySet<string>;
  /**
   * Which machines the list is showing.
   *
   * Read for one decision: whether this machine's *creation* controls (new
   * workspace, new chat) are offered at all. Those make something on this
   * machine, which under another device's filter the user could not see.
   */
  filter: DeviceFilter;
  /** Re-read a host's catalogue after one of its rows was acted on. */
  onChangedRemote: () => void;
  onNewChat?: () => void;
  /**
   * Start a conversation on another machine: in a workspace, or with none.
   *
   * `null` is a conversation with no workspace of its own — the same thing the
   * chat section's own control asks for on this machine.
   */
  onNewChatInWorkspace?: (desktopId: string, workspaceId: string | null) => void;
  onNewWorkspace?: () => void;
  onOpenRemote: (conversation: MergedConversation) => void;
  onRenameRemote: (conversation: MergedConversation) => void;
  /** Mark the chats section for a batch action. */
  onSelectChats?: () => void;
  /**
   * Rename a workspace on the machine that owns it.
   *
   * Addressed by host and workspace id, never by name: the name is what is being
   * changed, and two machines can hold the same one.
   */
  onRenameRemoteWorkspace?: (target: RemoteWorkspaceTarget) => void;
  /**
   * Mark one workspace's conversations for a batch action, by group key.
   *
   * The same control the local heading's menu has, for a workspace on another
   * machine: the batch it starts is assembled from the rows under that heading,
   * wherever they live.
   */
  onSelectWorkspaceChats?: (groupKey: string) => void;
  onToggleChatSection: () => void;
  /** Fold or unfold the forks of one conversation on another machine, by row key. */
  onToggleRemoteExpanded: (key: string) => void;
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
  /**
   * Which remote conversations have their forks on screen.
   *
   * Keyed by row key, and held by the rail for the same reason the local
   * expansion is: it is one fold state over the list, and a second `useState`
   * over the same question is how a heading and its rows come to disagree.
   */
  expandedRemoteRows: ReadonlySet<string>;
  /**
   * Where a remote conversation sits in its host's tree, keyed by row key.
   *
   * The remote counterpart of `localRowMeta`, and needed for the same reason: a
   * session on another machine that was forked from one of that machine's
   * sessions still *has* a parent, and without this its fork is drawn as a root
   * — no indent, no fold toggle — while the same fork on this machine has both.
   */
  remoteRowMeta: ReadonlyMap<string, { depth: number; hasChildren: boolean }>;
  peers: RemotePeer[];
  /**
   * The batch, as it applies to a host's rows.
   *
   * The rail owns the selection state (it is one selection over both machines'
   * rows), so what arrives here is the three questions the list asks of it per
   * row: is it ticked, does clicking it tick it, and tick it. Absent when there
   * is no remote row at all.
   */
  remoteSelection?: {
    selected: (conversation: MergedConversation) => boolean;
    selectable: (conversation: MergedConversation) => boolean;
    toggle: (conversation: MergedConversation) => void;
  };
  /** The local rail's own workspace heading, so both lists show one heading. */
  renderLocalGroupHeader: (workspaceId: string) => ReactNode;
  /**
   * This machine's own row factory.
   *
   * `compact` is the *group's* answer — a row inside a workspace is one notch
   * shorter than a top-level one — and the rail is what knows that, so it is
   * passed in rather than guessed at here.
   */
  renderLocalRow: (thread: StoredThread, depth?: number, hasChildren?: boolean, compact?: boolean) => ReactNode;
  rows: MergedConversation[];
  /** Whether a batch is being assembled, so "select chats" is not offered again. */
  selectionMode: boolean;
  threadById: Map<string, StoredThread>;
  workspaceSectionCollapsed: boolean;
}) {
  const { t } = useTranslation("layout");
  const { chats, pinned, workspaces } = groupMergedRows(rows);
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
      // `compact` travels with the row, and it is the *group's* answer, not the
      // row's own: a conversation inside a workspace is one notch shorter than a
      // top-level one. Passing it only to the remote row (as this did) left the
      // two kinds at different heights inside one list — a local workspace row at
      // 32px beside a remote one at 28px — and made the same local conversation
      // change height when the device filter changed, because the local-only list
      // always draws its workspace rows compact and this one did not.
      return renderLocalRow(thread, meta?.depth ?? 0, meta?.hasChildren ?? false, compact);
    }
    const selectable = remoteSelection?.selectable(conversation) === true;
    const meta = remoteRowMeta.get(conversation.key);
    return (
      <RemoteConversationRow
        active={activeKey === conversation.key}
        compact={compact}
        conversation={conversation}
        depth={meta?.depth ?? 0}
        expanded={expandedRemoteRows.has(conversation.key)}
        hasChildren={meta?.hasChildren ?? false}
        key={conversation.key}
        onChanged={onChangedRemote}
        onOpen={(row) => {
          // In selection mode a click ticks the row instead of opening it — the
          // local row's rule, from the same state.
          if (remoteSelection?.selectable(row)) {
            remoteSelection.toggle(row);
            return;
          }
          onOpenRemote(row);
        }}
        onOpenRename={onRenameRemote}
        onToggleExpanded={row => onToggleRemoteExpanded(row.key)}
        onToggleSelection={conversation => remoteSelection?.toggle(conversation)}
        peer={peers.find(peer => peer.desktopId === conversation.desktopId)}
        selected={remoteSelection?.selected(conversation) === true}
        selectionMode={selectable}
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
                            {/* A workspace on another machine takes the same
                                controls the local one has, in the same order:
                                the actions menu first, then the way to start a
                                conversation in it. */}
                            <RemoteWorkspaceMenu
                              desktopId={desktopId}
                              name={group.label ?? t("activityRail.workspace")}
                              onChanged={onChangedRemote}
                              onRename={onRenameRemoteWorkspace
                                ? () => onRenameRemoteWorkspace({
                                    // The heading's *current* name, which is what the
                                    // dialog opens with and what the host is
                                    // addressed by id to replace.
                                    desktopId,
                                    name: group.label ?? "",
                                    workspaceId,
                                  })
                                : undefined}
                              onSelect={selectionMode
                                ? undefined
                                : () => onSelectWorkspaceChats?.(group.key)}
                              pinned={group.pinned}
                              workspaceId={workspaceId}
                            />
                            <RailHeaderButton
                              icon={<Plus className="size-3.5" />}
                              label={t("activityRail.newChatInWorkspace", {
                                name: group.label ?? t("activityRail.workspace"),
                              })}
                              onClick={() => onNewChatInWorkspace?.(desktopId, workspaceId)}
                              // A *group* heading, not a section heading: the plus
                              // is the same control the local workspace heading
                              // draws, revealed the same way. Drawing it plainly
                              // here left a permanently visible glyph in the
                              // remote heading and a hidden one in the local
                              // heading directly above it.
                              revealOnHover
                            />
                          </>
                        )
                      : null}
                    name={group.label ?? t("activityRail.workspace")}
                    onToggle={() => onToggleWorkspace(group.key)}
                    pinned={group.pinned}
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
                  conversations for a batch action, and the batch can hold any
                  machine's rows — a remote row carries the same checkbox a local
                  one does — so it is offered wherever the section lists
                  anything. */}
              {onSelectChats
                ? (
                    <ChatSectionMenu
                      open={chatMenuOpen}
                      onOpenChange={setChatMenuOpen}
                      onSelect={onSelectChats}
                    />
                  )
                : null}
              {/* A conversation lives on one machine, and the list can be
                  showing one or all of them — so this asks only when there is
                  something to ask. Filtering to a host used to leave the reader
                  no way to start a conversation *on* the host they had just
                  chosen to look at. */}
              <RailNewChatButton
                filter={filter}
                onNewChatLocal={onNewChat}
                onNewChatRemote={onNewChatInWorkspace
                  ? (desktopId, workspaceId) => onNewChatInWorkspace(desktopId, workspaceId)
                  : undefined}
                peers={peers}
              />
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
