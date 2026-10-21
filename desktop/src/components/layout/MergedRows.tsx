import type { ReactNode } from "react";
import type { DeviceFilter, MergedConversation, RemoteWorkspaceTarget } from "../../features/remote-peer/mergeConversations";
import type { RemotePeer } from "../../features/remote-peer/remotePeerClient";
import type { StoredThread } from "../../integrations/storage/threadStore";
import type { RailView } from "./hooks/useRailView";
import type { RecentSection, RecentSectionKind } from "./recentOrder";
import { Plus } from "lucide-react";
import { useTranslation } from "react-i18next";
import { groupMergedRows } from "../../features/remote-peer/mergeConversations";
import { iconGlyph } from "../../features/remote-peer/peerIcons";
import { RemoteWorkspaceMenu } from "../../features/remote-peer/RemoteWorkspaceMenu";
import { RailGroupHeader, RailHeaderButton, RailSectionHeader } from "./RailChrome";
import { RemoteConversationRow } from "./RemoteConversationRow";

/**
 * Which list this component draws: one of the rail's tabs, or the search results.
 *
 * Not "tree": that was two lists (workspaces, then chats) wearing one name, and
 * the reason the chats read as living inside the workspaces.
 */
export type MergedListView = RailView | "search";

/**
 * The merged list's rows, one list at a time.
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
  ageLabels,
  collapsedWorkspaces,
  filter,
  onChangedRemote,
  onDeletedRemote,
  onNewChatInWorkspace,
  onOpenRemote,
  onRenameRemote,
  onRenameRemoteWorkspace,
  onSelectWorkspaceChats,
  onToggleRemoteExpanded,
  onToggleWorkspace,
  peers,
  remoteSelection,
  renderLocalGroupHeader,
  onShowMoreRecent,
  recentMore,
  recentSections,
  renderLocalRow,
  rowMeta,
  rowRoots,
  rows,
  searchLabels,
  selectionMode,
  threadById,
  expandedRemoteRows,
  view,
}: {
  activeKey: string | null;
  /** The age label to draw on a row, by row key (the recent view only). */
  ageLabels?: ReadonlyMap<string, string>;
  /**
   * Where each search result lives, by row key: the workspace's name, or the
   * chats tab's own name for a conversation in no workspace.
   *
   * Cross-tab results are one flat list, so the one fact a row loses by leaving
   * its own list is which list it came from — and that is exactly the fact the
   * reader needs to decide whether to open it.
   */
  searchLabels?: ReadonlyMap<string, string>;
  /**
   * The recent view's stretches, already split and windowed.
   *
   * Split by the rail rather than here because the rail also has to walk the
   * same rows with the keyboard: two splits of the same list would be two
   * answers to "which row is next", and the window is a piece of that order.
   */
  recentSections?: RecentSection<MergedConversation>[];
  /** Finished rows the window is holding back, and how to open it further. */
  recentMore?: number;
  onShowMoreRecent?: () => void;
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
  /**
   * One of the host's conversations no longer exists.
   *
   * Forwarded to the row rather than settled here because the rail's selection
   * is what holds the open conversation; this list only knows its rows.
   */
  onDeletedRemote: (conversation: MergedConversation) => void;
  /**
   * Start a conversation on another machine: in a workspace, or with none.
   *
   * `null` is a conversation with no workspace of its own — the same thing the
   * chat section's own control asks for on this machine.
   */
  onNewChatInWorkspace?: (desktopId: string, workspaceId: string | null) => void;
  onOpenRemote: (conversation: MergedConversation) => void;
  onRenameRemote: (conversation: MergedConversation) => void;
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
  /** Fold or unfold the forks of one conversation on another machine, by row key. */
  onToggleRemoteExpanded: (key: string) => void;
  onToggleWorkspace: (key: string) => void;
  /**
   * Which remote conversations have their forks on screen.
   *
   * Keyed by row key, and held by the rail for the same reason the local
   * expansion is: it is one fold state over the list, and a second `useState`
   * over the same question is how a heading and its rows come to disagree.
   */
  expandedRemoteRows: ReadonlySet<string>;
  /**
   * Where each listed row sits in the shared list's tree, by row key.
   *
   * One map for both machines' rows, because it is one tree: a conversation
   * forked from one on the other machine is drawn with the same indent and
   * folded by the same toggle as a local one, and two maps meant two places to
   * be right about a row's depth. Built by the rail from the tree it folds with
   * — a fold decides which rows are *here*, not how deep any of them are.
   */
  rowMeta: ReadonlyMap<string, { depth: number; hasChildren: boolean }>;
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
  renderLocalRow: (
    thread: StoredThread,
    depth?: number,
    hasChildren?: boolean,
    compact?: boolean,
    /** The recent view's second column; absent in the tabbed lists. */
    ageLabel?: string,
    /** The search results' weak origin label; absent everywhere else. */
    qualifier?: string,
  ) => ReactNode;
  /**
   * Which conversation each listed row hangs under, by row key — its root.
   *
   * The rail's tree, handed over rather than rebuilt: the rail is what knows
   * whether a fork's parent is one of *these* rows (the device filter, a fold, a
   * parent that is gone), and a fork is drawn under that row — in its section, at
   * its height — instead of taking a place of its own by time.
   */
  rowRoots: ReadonlyMap<string, MergedConversation>;
  rows: MergedConversation[];
  /** Whether a batch is being assembled, so "select chats" is not offered again. */
  selectionMode: boolean;
  threadById: Map<string, StoredThread>;
  /**
   * Which list to draw.
   *
   * A *tab*, not a shape: the rail stopped having a "tree" mode and a "recent"
   * mode, because a workspace list and a chat list are siblings rather than
   * nested views of one another. So:
   *
   * - `workspace` — the pinned workspace conversations, then the workspace
   *   groups with the conversations inside them.
   * - `chat` — the pinned chats, then the conversations that belong to no
   *   workspace.
   * - `recent` — no headings for where a conversation lives, two stretches
   *   instead ("active", "earlier"), each ranked by one clock (`recentOrder.ts`).
   * - `search` — the results of a query, flat and across the tabs, each row
   *   saying where it lives (`searchLabels`).
   *
   * Every one is drawn from the rows this component is handed, so switching
   * lists moves nothing about a conversation itself.
   */
  view: MergedListView;
}) {
  const { t } = useTranslation("layout");
  // The sections and their order: a fork is a conversation's, so the rows are
  // ranked by the conversation each one hangs under, and a fork lands in that
  // conversation's section rather than one its own fields would pick.
  const { chats, pinned, workspaces } = groupMergedRows(rows, rowRoots);

  // One machine selected: it is named by the selector directly above the list, so
  // repeating it on every row says nothing and takes the space a title needs.
  const oneMachine = filter.kind === "device" && filter.desktopId !== null;

  function renderRow(
    conversation: MergedConversation,
    compact: boolean,
    groupHasSource: boolean,
    /**
     * The flat lists' overrides.
     *
     * A flat list says everything itself: every row sits at depth 0 (there is no
     * parent above it to indent under), carries its own age in the recent view,
     * and says where it lives in the search results. The tabbed lists pass
     * nothing, so their rows keep the depth their tree computed.
     */
    flat?: { age?: string; depth: 0; label?: string },
  ): ReactNode {
    if (conversation.desktopId === null) {
      const thread = threadById.get(conversation.id);
      // A local row without a thread cannot happen (the merge is built from the
      // same list), but rendering nothing beats rendering a broken row.
      if (!thread)
        return null;
      const meta = rowMeta.get(conversation.key);
      // `compact` travels with the row, and it is the *group's* answer, not the
      // row's own: a conversation inside a workspace is one notch shorter than a
      // top-level one. Passing it only to the remote row (as this did) left the
      // two kinds at different heights inside one list — a local workspace row at
      // 32px beside a remote one at 28px — and made the same local conversation
      // change height when the device filter changed, because the local-only list
      // always draws its workspace rows compact and this one did not.
      return renderLocalRow(
        thread,
        flat?.depth ?? meta?.depth ?? 0,
        // A fork has no fold control here: its children are rows of their own in
        // this list, so folding it would promise a subtree that is not under it.
        flat ? false : meta?.hasChildren ?? false,
        compact,
        flat?.age,
        flat?.label,
      );
    }
    const selectable = remoteSelection?.selectable(conversation) === true;
    const meta = rowMeta.get(conversation.key);
    return (
      <RemoteConversationRow
        active={activeKey === conversation.key}
        ageLabel={flat?.age}
        compact={compact}
        conversation={conversation}
        depth={flat?.depth ?? meta?.depth ?? 0}
        expanded={expandedRemoteRows.has(conversation.key)}
        hasChildren={flat ? false : meta?.hasChildren ?? false}
        key={conversation.key}
        onChanged={onChangedRemote}
        onDeletedRemote={onDeletedRemote}
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
        qualifier={flat?.label}
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

  if (view === "search") {
    // One flat list across the tabs, every row saying where it lives. The order
    // is the rail's (it built the rows), because it is also what the keyboard
    // walk steps through.
    return (
      <div className="space-y-0.5" data-testid="search-rows">
        {rows.map(row => (
          <div data-search-row={row.key} key={row.key}>
            {renderRow(row, false, false, {
              depth: 0,
              label: searchLabels?.get(row.key),
            })}
          </div>
        ))}
      </div>
    );
  }

  if (view === "recent" && recentSections) {
    // The second view: two stretches, no headings for where a conversation
    // lives. A row keeps the machine it is on (a merged list is several
    // machines) and loses its indent — there is no parent above it here, and a
    // fork is a row of its own at its own age.
    const sectionLabel: Record<RecentSectionKind, string> = {
      active: t("activityRail.recentActive"),
      earlier: t("activityRail.recentEarlier"),
    };
    return (
      <div className="space-y-0.5" data-testid="recent-rows">
        {recentSections.map(section => (
          <div className="space-y-0.5" data-recent-section={section.kind} key={section.kind}>
            <RailSectionHeader>
              {sectionLabel[section.kind]}
              {/* What the caption counts is the whole stretch, not the drawn
                  part: "Earlier 42" stays true whether 30 or all of them are
                  under it. */}
              <span className="ml-1 tabular-nums text-ink-muted/70" data-recent-count={section.all.length}>
                {section.all.length}
              </span>
            </RailSectionHeader>
            {section.rows.map(row => (
              <div data-recent-row={row.key} data-testid="recent-row" key={row.key}>
                {renderRow(row, false, false, {
                  age: ageLabels?.get(row.key),
                  depth: 0,
                })}
              </div>
            ))}
          </div>
        ))}
        {recentMore !== undefined && recentMore > 0
          ? (
              <button
                aria-label={t("activityRail.recentShowMore", { count: recentMore })}
                className="mt-1 flex h-7 w-full items-center justify-center rounded-md border border-line-soft bg-surface-subtle text-xs font-medium text-accent transition-colors hover:bg-surface"
                data-testid="recent-show-more"
                onClick={onShowMoreRecent}
                type="button"
              >
                {t("activityRail.recentShowMore", { count: recentMore })}
              </button>
            )
          : null}
      </div>
    );
  }

  // One tab at a time. The tab *is* the section heading: it says which list this
  // is, so a heading inside the list repeating its name was the extra level the
  // tabs replaced. What those headings carried moved with them — the tab's own
  // menu (its batch entry) lives in the rail's control row, and the workspace tab
  // offers "new workspace" there too.
  const tabPinned = pinned.filter(row => rowTabOf(row) === view);
  return (
    <>
      {tabPinned.length > 0
        ? (
            <div className="mb-3 space-y-0.5">
              <RailSectionHeader>{t("activityRail.pinnedHeader")}</RailSectionHeader>
              {tabPinned.map(row => renderRow(row, false, false))}
            </div>
          )
        : null}
      {view === "chat"
        ? (
            <div className="shrink-0 space-y-0.5" data-testid="merged-conversations">
              {chats.length === 0
                ? <div className="px-2 py-1 text-xs text-ink-muted">{t("activityRail.noChats")}</div>
                : chats.map(row => renderRow(row, false, false))}
            </div>
          )
        : (
            <div className="space-y-0.5">
              {workspaces.length === 0
                ? <div className="px-2 py-1 text-xs text-ink-muted">{t("activityRail.emptyWorkspaceTab")}</div>
                : null}
              {workspaces.map((group) => {
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
          )}
    </>
  );
}

/** Which tab a conversation belongs to — the same rule the rail uses. */
function rowTabOf(row: MergedConversation): "chat" | "workspace" {
  return row.mode === "workspace" ? "workspace" : "chat";
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
