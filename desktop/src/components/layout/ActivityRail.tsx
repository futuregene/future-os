import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import type { DeviceFilter, MergedConversation, RemoteWorkspaceTarget } from "../../features/remote-peer/mergeConversations";
import type { RemotePeer } from "../../features/remote-peer/remotePeerClient";
import type { StoredThread, StoredWorkspace } from "../../integrations/storage/threadStore";
import type { FutureSessionStatus } from "./hooks/useFutureAccount";
import type { RemoteIndicator } from "./hooks/useRemoteStatus";
import type { ThreadRunInfo } from "./hooks/useThreadStore";
import type { TreeNode } from "./threadTree";
import {
  Blocks,
  ChevronDown,
  ChevronRight,
  Folder,
  ListChecks,
  MessageSquare,
  MonitorSmartphone,
  PanelLeft,
  Pin,
  Plus,
  Sparkles,
  SquarePen,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useCommand } from "../../features/keyboard/commands";
import { groupMergedRows, mergeConversations, rowKey } from "../../features/remote-peer/mergeConversations";
import { peersDotState } from "../../features/remote-peer/peerStatus";
import { SkillIntroBubble } from "../../features/skills/SkillIntroBubble";
import { listInstalledSkills } from "../../integrations/skills/skillsClient";
import { cn } from "../../lib/cn";
import { onFutureEvent } from "../../lib/futureEvents";
import { isMacOS } from "../../lib/platform";
import { useFloatingScrollbar } from "../../lib/useFloatingScrollbar";
import { useIsFullscreen } from "../../lib/useIsFullscreen";
import { startWindowDrag } from "../../lib/windowDrag";
import { FloatingScrollbar } from "../ui/FloatingScrollbar";
import { IconButton } from "../ui/IconButton";
import { ActivityRailAccountFooter } from "./ActivityRailAccountFooter";
import { ChatSectionMenu, WorkspaceHeaderMenu } from "./ActivityRailMenus";
import { ActivityRailSelectionToolbar } from "./ActivityRailSelectionToolbar";
import { ConversationSearch } from "./ConversationSearch";
import { DeviceSelector } from "./DeviceSelector";
import { useCollapsedWorkspaces } from "./hooks/useCollapsedWorkspaces";
import { usePendingApprovalCounts } from "./hooks/usePendingApprovalCounts";
import { useRailSelection } from "./hooks/useRailSelection";
import { MergedRows } from "./MergedRows";
import { RailDisclosure, RailHeaderButton } from "./RailChrome";
import { ThreadListItem } from "./ThreadListItem";
import { buildThreadTree, buildTree, threadParentIds, visibleThreadRows, visibleTreeRows } from "./threadTree";

export type ActivitySection = "chat" | "workspace" | "skill" | "tasks" | "remote" | "settings";

interface ActivityRailProps {
  active: ActivitySection;
  expanded: boolean;
  floating?: boolean;
  activeThreadId: string | null;
  hasUpdate?: boolean;
  threads: StoredThread[];
  threadRunStatuses: Record<string, ThreadRunInfo | undefined>;
  threadStreamingStatuses: Record<string, boolean>;
  unreadThreadIds: Set<string>;
  workspaces: StoredWorkspace[];
  onChange: (section: ActivitySection) => void;
  onBatchDeleteThreads: (threads: StoredThread[]) => void;
  /**
   * Delete a batch of conversations on the hosts that own them.
   *
   * The remote half of `onBatchDeleteThreads`: one selection can hold both
   * machines' rows, and they are removed by different code — a local store here,
   * a command per conversation there.
   */
  onDeleteRemoteConversations: (rows: MergedConversation[]) => void;
  /**
   * A single conversation was deleted from its own row's menu.
   *
   * Separate from the batch above because it is a separate action: the batch
   * removes what a selection holds, while this is the one row the user acted
   * on — and only the shell knows whether that row is the conversation it is
   * currently drawing.
   */
  onRemoteConversationDeleted: (conversation: MergedConversation) => void;
  onDeleteThread: (thread: StoredThread) => void;
  onNewChat: (workspaceId?: string) => void;
  onOpenModels: () => void;
  onNewWorkspace: () => void;
  onRenameThread: (thread: StoredThread) => void;
  onRenameWorkspace: (workspace: StoredWorkspace) => void;
  onTogglePinWorkspace: (workspace: StoredWorkspace) => void;
  onDeleteWorkspace: (workspace: StoredWorkspace) => void;
  onRestoreThread: (thread: StoredThread) => void;
  onSelectWorkspace: (workspace: StoredWorkspace, threads: StoredThread[]) => void;
  onSelectThread: (thread: StoredThread) => void;
  onTogglePinThread: (thread: StoredThread) => void;
  onToggleExpanded: () => void;
  /** Remote bridge connection state for the nav indicator dot. */
  remoteIndicator?: RemoteIndicator;
  /** The remote desktops this machine is paired with (client role). */
  remotePeers?: RemotePeer[];
  /** Each connected remote desktop's session snapshot. */
  remoteCatalogs?: import("../../features/remote-peer/mergeConversations").RemoteCatalog[];
  /** Which machines the conversation list shows. */
  deviceFilter?: DeviceFilter;
  onChangeDeviceFilter?: (filter: DeviceFilter) => void;
  /** Open a conversation that lives on another machine. */
  onOpenRemoteConversation?: (conversation: MergedConversation) => void;
  /** Rename a conversation on the machine that owns it. */
  onRenameRemoteConversation?: (conversation: MergedConversation) => void;
  /** Rename a workspace on the machine that owns it. */
  onRenameRemoteWorkspace?: (target: RemoteWorkspaceTarget) => void;
  /** Re-read paired hosts and their catalogues (after a remote row action). */
  onRemoteConversationsChanged?: () => void;
  /**
   * Start a conversation in a workspace on another machine.
   *
   * The remote counterpart of the local heading's own "new chat" control: the
   * workspace is on that host, so the request has to name it there rather than
   * carry a path this machine cannot resolve.
   */
  onNewChatOnRemote?: (desktopId: string, workspaceId: string | null) => void;
  /** Jump to the Remote Desktops screen. */
  onManageDesktops?: () => void;
  /** The remote conversation currently open (client role), if any. */
  activeRemoteKey?: string | null;
  /** FutureOS credit balance (null when signed out). */
  futureBalance?: number | null;
  /** Signed-in FutureOS email (null when signed out). Drives the account menu. */
  userEmail?: string | null;
  /** Platform-verified account state. Controls FutureOS-only navigation. */
  futureSessionStatus?: FutureSessionStatus;
  /** Community edition deliberately keeps account identity and billing out of the footer. */
  communityEdition?: boolean;
  /** Opens the recharge page in the system browser. */
  onRecharge?: () => void;
  /** Opens the Settings dialog on the "Check for updates" tab. */
  onOpenUpdate?: () => void;
  /** The user acknowledged the Skills intro bubble; dot + bubble hide once set. */
  skillIntroDismissed: boolean;
  onDismissSkillIntro: () => void;
}

// Data / Skill entries are temporarily hidden from the navigation:
// these modules have been deprioritised (see PLAN.md "Next Priorities").
// Section handling logic is preserved; only the left-nav items are removed —
// add them back to restore. (Research was removed entirely; see PRODUCT.md §4.9.)
const featureItems: Array<{ id: ActivitySection; label: string; icon: LucideIcon }> = [];

export function ActivityRail({
  active,
  activeThreadId,
  expanded,
  floating,
  hasUpdate,
  threads,
  threadRunStatuses,
  threadStreamingStatuses,
  unreadThreadIds,
  workspaces,
  onChange,
  onBatchDeleteThreads,
  onDeleteRemoteConversations,
  onRemoteConversationDeleted,
  onDeleteThread,
  onNewChat,
  onOpenModels,
  onNewWorkspace,
  onRenameThread,
  onRenameWorkspace,
  onTogglePinWorkspace,
  onDeleteWorkspace,
  onRestoreThread,
  onSelectWorkspace,
  onSelectThread,
  onTogglePinThread,
  onToggleExpanded,
  remoteIndicator,
  remotePeers = [],
  remoteCatalogs = [],
  deviceFilter = { kind: "all" },
  onChangeDeviceFilter,
  onOpenRemoteConversation,
  onRenameRemoteConversation,
  onRenameRemoteWorkspace,
  onRemoteConversationsChanged,
  onNewChatOnRemote,
  onManageDesktops,
  activeRemoteKey = null,
  futureBalance,
  userEmail,
  communityEdition,
  onRecharge,
  onOpenUpdate,
  skillIntroDismissed,
  onDismissSkillIntro,
}: ActivityRailProps) {
  const { t } = useTranslation("layout");
  const noMatchesText = t("activityRail.noMatches");
  // Pending approvals across all threads — badged on rail items so a background
  // conversation's approval is visible without opening it.
  const pendingApprovalCounts = usePendingApprovalCounts();
  // Shared overlay scrollbar for the conversation list, matching the chat view.
  const listScrollbar = useFloatingScrollbar();
  // Connection indicator overlaid on the Remote nav icon.
  //
  // It answers for the *paired* machines when there are any (the phone's own
  // rule): blue only when every one of them is up **and** drivable, amber when
  // some are — a machine held by another paired device answers and still cannot
  // contribute a row — and red when none are. Because the merged conversation
  // list is short of exactly the machines it cannot read, and that is the fact
  // the reader has to be able to see without opening the page. With nothing
  // paired it keeps its other meaning: this machine's own host bridge (blue while
  // devices can connect in, amber while it is being established, red when it is
  // down, nothing before pairing).
  const peersDot = peersDotState(remotePeers);
  const dotState: "up" | "partial" | "down" | null = peersDot
    ?? (remoteIndicator === "connected"
      ? "up"
      : remoteIndicator === "connecting"
        ? "partial"
        : remoteIndicator === "disconnected"
          ? "down"
          : null);
  const remoteDot = dotState
    ? (
        <span
          className={cn(
            "absolute -right-1 -top-1 size-2 rounded-full",
            dotState === "up"
              ? "bg-accent"
              : dotState === "partial"
                ? "bg-warning"
                : "bg-danger",
            dotState === "partial" && "animate-pulse",
          )}
          data-testid="remote-status-dot"
          data-state={dotState}
        />
      )
    : null;
  // Reserve the top-left inset for the macOS traffic lights, except in
  // fullscreen where the lights are hidden and the inset is dead space.
  const isFullscreen = useIsFullscreen();
  const reserveTrafficLights = isMacOS && !isFullscreen;
  const [openThreadMenuId, setOpenThreadMenuId] = useState<string | null>(null);
  const [openWorkspaceMenuId, setOpenWorkspaceMenuId] = useState<string | null>(null);
  const [expandedThreads, setExpandedThreads] = useState<Set<string>>(() => new Set());
  const toggleThreadExpanded = useCallback((thread: StoredThread) => {
    setExpandedThreads((current) => {
      const next = new Set(current);
      if (next.has(thread.id))
        next.delete(thread.id);
      else next.add(thread.id);
      return next;
    });
  }, []);
  /**
   * The same fold state for conversations on another machine.
   *
   * Its own set rather than one over both: a local row is addressed by thread id
   * and a remote one by its row key, and one set keyed two ways is how a local
   * thread and a remote session that happen to share an id would fold together.
   * Kept here, next to the local fold state, because it is one question over the
   * list and the rows are the rail's to draw.
   */
  const [expandedRemoteThreads, setExpandedRemoteThreads] = useState<Set<string>>(() => new Set());
  const toggleRemoteThreadExpanded = useCallback((key: string) => {
    setExpandedRemoteThreads((current) => {
      const next = new Set(current);
      if (next.has(key))
        next.delete(key);
      else next.add(key);
      return next;
    });
  }, []);
  // Persisted across restarts (see useCollapsedWorkspaces).
  const { collapsedWorkspaces, toggleWorkspaceCollapsed } = useCollapsedWorkspaces();
  // Collapse state for the two top-level list sections (Workspace / Chat),
  // independent of the per-workspace group collapse above.
  const [workspaceSectionCollapsed, setWorkspaceSectionCollapsed] = useState(false);
  const [chatSectionCollapsed, setChatSectionCollapsed] = useState(false);
  // Chat section header menu.
  const [chatSectionMenuOpen, setChatSectionMenuOpen] = useState(false);
  // Text search for the conversation list. Not persisted: the list is huge
  // and a leftover query after restart would feel like a stale filter.
  const [searchQuery, setSearchQuery] = useState("");
  const [searchOpen, setSearchOpen] = useState(false);
  const closeSearch = useCallback(() => {
    setSearchOpen(false);
    setSearchQuery("");
  }, []);
  // Attention state for the Skills nav entry: pulsed for a few seconds when
  // the skill-guide banner is dismissed (its notice says the tutorial can be
  // reopened from the Skills page).
  const [skillAttention, setSkillAttention] = useState(false);
  useEffect(() => {
    if (!skillAttention)
      return;
    const timer = window.setTimeout(setSkillAttention, 4000, false);
    return () => window.clearTimeout(timer);
  }, [skillAttention]);
  useEffect(() => onFutureEvent("skill-guide-dismissed", () => setSkillAttention(true)), []);
  // Installed-skills count for the nav badge + intro bubble. Refreshes on
  // "skills-changed" (install/uninstall); stays null while loading or when
  // the agent is unreachable (badge/bubble wait for a later launch).
  const [skillCount, setSkillCount] = useState<number | null>(null);
  useEffect(() => {
    let cancelled = false;
    const load = () => {
      listInstalledSkills()
        .then((skills) => {
          if (!cancelled)
            setSkillCount(skills.length);
        })
        .catch(() => {
          // Backend not connected — leave the count unknown this session.
        });
    };
    load();
    const unsubscribe = onFutureEvent("skills-changed", load);
    return () => {
      cancelled = true;
      unsubscribe();
    };
  }, []);

  const handleThreadMenuOpenChange = useCallback((thread: StoredThread, open: boolean) => {
    setOpenThreadMenuId(open ? thread.id : null);
  }, []);

  // Derived thread lists are memoized: the rail re-renders whenever any
  // run/streaming status changes, but these depend only on the thread list
  // and selection scope — recomputing the sort + filters (O(W×T) with the
  // workspace grouping) on every render was wasted churn (M3).
  const visibleThreads = useMemo(
    () => sortThreads(threads.filter(thread => thread.status === "active")),
    [threads],
  );

  const threadTree = useMemo(() => buildThreadTree(visibleThreads), [visibleThreads]);
  const threadScopes = useMemo(() => {
    const allExpanded = new Set(visibleThreads.map(thread => thread.id));
    const scopes = new Map<string, string>();
    for (const root of threadTree) {
      const scope = root.item.mode === "chat" ? "chat" : root.item.workspaceId;
      for (const { thread } of visibleThreadRows([root], allExpanded)) scopes.set(thread.id, scope);
    }
    return scopes;
  }, [threadTree, visibleThreads]);

  /**
   * This machine's pinned workspaces, by id.
   *
   * Read by the merged list's local rows, so a pinned workspace leads there too,
   * and by the local heading's pin marker. One source for both: the merged list
   * orders a group by the pin its *rows* carry, and this is what a row of this
   * machine's carries.
   */
  const pinnedWorkspaceIds = useMemo(
    () => new Set(workspaces.filter(workspace => workspace.pinned).map(workspace => workspace.id)),
    [workspaces],
  );

  /**
   * This machine's workspaces, by when *they* were last touched.
   *
   * The same key `list_workspaces` orders its own rows by
   * (`COALESCE(last_opened_at, updated_at)`), read here so a local group holds
   * its place in the merged list exactly as it holds it in the local one: a
   * conversation arriving inside a workspace must not move the workspace.
   */
  const workspaceOrder = useMemo(
    () => new Map(workspaces.map(workspace => [
      workspace.id,
      workspace.lastOpenedAt ?? workspace.updatedAt,
    ])),
    [workspaces],
  );

  const normalizedQuery = searchQuery.trim().toLowerCase();
  const queryActive = normalizedQuery.length > 0;

  /**
   * Which thread each of this machine's threads was forked from.
   *
   * The rail's own tree resolves it (a fork names its parent by agent session id),
   * and the merged list is handed the same answer so a local fork is hung under
   * the row it came from — the local list's own tree, in the list that holds both
   * machines' rows.
   */
  const threadParents = useMemo(() => threadParentIds(visibleThreads), [visibleThreads]);

  const remoteRows = useMemo(
    () => mergeConversations(
      visibleThreads.map(thread => ({
        id: thread.id,
        title: thread.title,
        mode: thread.mode,
        pinned: thread.pinned,
        status: thread.status,
        workspaceId: thread.workspaceId,
        workspacePinned: pinnedWorkspaceIds.has(thread.workspaceId),
        workspaceOrderedAt: workspaceOrder.get(thread.workspaceId) ?? null,
        lastMessageAt: thread.lastMessageAt,
        updatedAt: thread.updatedAt,
        createdAt: thread.createdAt,
        parentThreadId: threadParents.get(thread.id) ?? null,
      })),
      remoteCatalogs,
      deviceFilter,
      // Group headings for the local half of the merged list.
      new Map(workspaces.map(workspace => [workspace.id, workspace.name])),
    ),
    [visibleThreads, remoteCatalogs, deviceFilter, workspaces, workspaceOrder, pinnedWorkspaceIds, threadParents],
  );
  const hasRemoteRows = remoteRows.some(row => row.desktopId !== null);
  /**
   * The merged mode, and the one rule that turns it on.
   *
   * A user with no remote desktops keeps today's list *exactly*: grouped by
   * workspace, chat section, selection mode. Only once a remote row is actually
   * present does the list become the flat, time-ordered merged view, because that
   * view is a different information architecture — the workspace grouping stops
   * being meaningful when a group name can exist on two machines at once.
   */
  const mergedMode = hasRemoteRows || (deviceFilter.kind === "device" && deviceFilter.desktopId !== null);

  // Filtering: include a row if its title or its workspace name matches.
  const rowMatchesQuery = useCallback((row: MergedConversation): boolean => {
    if (!queryActive)
      return true;
    if (row.title.toLowerCase().includes(normalizedQuery))
      return true;
    if (row.workspaceName && row.workspaceName.toLowerCase().includes(normalizedQuery))
      return true;
    return false;
  }, [normalizedQuery, queryActive]);

  const visibleRemoteRows = useMemo(
    () => (mergedMode ? remoteRows : []),
    [mergedMode, remoteRows],
  );
  /**
   * Which batch each listed remote row belongs to.
   *
   * The same scopes the local rows use, in the merged list's own vocabulary: the
   * chat section is one scope and a workspace is one, keyed by the group key the
   * list already groups and folds by. A batch is then "the rows under this
   * heading" wherever they live.
   */
  const remoteScopes = useMemo(
    () => new Map(visibleRemoteRows.map(row => [
      row.key,
      row.mode === "workspace" ? `${row.desktopId ?? ""}::${row.workspaceId ?? ""}` : "chat",
    ])),
    [visibleRemoteRows],
  );

  const filteredRemoteRows = useMemo(
    () => (mergedMode ? remoteRows.filter(row => rowMatchesQuery(row)) : []),
    [mergedMode, remoteRows, rowMatchesQuery],
  );

  /**
   * The shared list's tree: where each row sits, what a fold hides, and the
   * conversation each row hangs under.
   *
   * One tree over **both machines' rows**, not one per half. A fork's parent is a
   * row of this list or it is not, and only the list can answer that: this
   * machine's sessions name their parent by agent session id (resolved to a local
   * row key in the merge) and a host's by its own session id, so the two halves
   * are two branches of the same question, drawn with the same indent and folded
   * by the same toggle.
   *
   * This replaces what the list used to do for a folded local parent — a second
   * walk whose only job was to hide that parent's descendants — because the rows
   * it now hides are the same ones, for either machine, and the answer is the tree
   * the rows are drawn from.
   */
  const mergedRowTree = useMemo(() => {
    const nodes = buildTree(filteredRemoteRows, row => ({
      key: row.key,
      parentKey: row.parentKey ?? null,
      pinned: row.pinned,
    }));
    // One fold state, in the list's vocabulary: a local row's own expansion is
    // keyed by thread id and a host's by row key, so the local half is translated
    // into the key its merged row carries.
    const expanded = new Set(expandedRemoteThreads);
    for (const threadId of expandedThreads) expanded.add(rowKey(null, threadId));
    const visible = visibleTreeRows(nodes, row => row.key, expanded);
    const meta = new Map(visible.map(row => [row.item.key, row] as const));
    // Which conversation each row hangs under. Section and order are decided
    // outside this component (`groupMergedRows`), so the rail's tree — the only
    // place that knows a fork's parent is a row *of this list* — is handed over
    // with them rather than rebuilt from the parent links.
    const roots = new Map<string, MergedConversation>();
    const tag = (branch: TreeNode<MergedConversation>[], root: MergedConversation) => {
      for (const node of branch) {
        roots.set(node.item.key, root);
        tag(node.children, root);
      }
    };
    for (const node of nodes) tag([node], node.item);
    return { meta, roots, rows: visible.map(row => row.item) };
  }, [expandedRemoteThreads, expandedThreads, filteredRemoteRows]);

  const threadMatchesQuery = useCallback((thread: StoredThread): boolean => {
    if (!queryActive)
      return true;
    if (thread.title.toLowerCase().includes(normalizedQuery))
      return true;
    const workspace = workspaces.find(ws => ws.id === thread.workspaceId);
    if (workspace && workspace.name.toLowerCase().includes(normalizedQuery))
      return true;
    return false;
  }, [normalizedQuery, queryActive, workspaces]);

  const filteredVisibleThreads = useMemo(
    () => visibleThreads.filter(thread => threadMatchesQuery(thread)),
    [visibleThreads, threadMatchesQuery],
  );

  const {
    deleteSelected,
    deselectAll,
    enterSelectionMode,
    exitSelectionMode,
    handleRowSelect,
    isRemoteRowSelectable,
    isThreadInScope,
    selectAll,
    selectedCount,
    selectedRemoteKeys,
    selectedThreadIds,
    selectionMode,
    toggleRemoteSelection,
    toggleThreadSelection,
    totalCount,
  } = useRailSelection({
    onBatchDeleteThreads,
    onDeleteRemoteConversations,
    onSelectThread,
    remoteScopes,
    threadScopes,
    visibleRemoteRows: mergedMode ? filteredRemoteRows : visibleRemoteRows,
    visibleThreads: mergedMode ? filteredVisibleThreads : visibleThreads,
  });

  /** Whether this thread should show a selection checkbox. */
  function threadSelectionMode(thread: StoredThread): boolean {
    return selectionMode && isThreadInScope(thread) && (!searchOpen || threadMatchesQuery(thread));
  }

  // Pinned threads are hoisted into a single global section (regardless of
  // workspace/chat); the per-group lists show only the unpinned rest.
  const pinnedThreads = useMemo(() => visibleThreadRows(threadTree.filter(node => node.item.pinned), expandedThreads), [threadTree, expandedThreads]);
  const chatThreads = useMemo(() => visibleThreadRows(threadTree.filter(node => node.item.mode === "chat" && !node.item.pinned), expandedThreads), [threadTree, expandedThreads]);
  const workspaceRoots = useMemo(() => threadTree.filter(node => node.item.mode === "workspace" && !node.item.pinned), [threadTree]);
  // The store keeps its recency order (the new-conversation picker reads
  // `workspaces[0]` as "most recent"); pinning is this list's own rule, so the
  // groups are ordered here. Within each half the store's order is kept.
  const workspaceGroups = useMemo(() => {
    const groups = workspaces.filter(
      workspace => workspace.kind === "user" || workspaceRoots.some(node => node.item.workspaceId === workspace.id),
    );
    return [...groups.filter(workspace => workspace.pinned), ...groups.filter(workspace => !workspace.pinned)]
      .map((workspace) => {
        const roots = workspaceRoots.filter(node => node.item.workspaceId === workspace.id);
        return {
          workspace,
          rows: visibleThreadRows(roots, expandedThreads),
          threads: visibleThreadRows(roots, new Set(visibleThreads.map(thread => thread.id))).map(row => row.thread),
        };
      })
      .filter((group) => {
        if (normalizedQuery.length === 0)
          return true;
        return group.workspace.name.toLowerCase().includes(normalizedQuery)
          || group.rows.some(row => threadMatchesQuery(row.thread));
      })
      .map(group => ({
        ...group,
        rows: group.rows.filter(({ thread }) => threadMatchesQuery(thread)),
        threads: group.threads.filter(thread => threadMatchesQuery(thread)),
      }));
  }, [workspaceRoots, workspaces, expandedThreads, visibleThreads, threadMatchesQuery, normalizedQuery]);

  const filteredPinnedThreads = useMemo(
    () => pinnedThreads.filter(({ thread }) => threadMatchesQuery(thread)),
    [pinnedThreads, threadMatchesQuery],
  );
  const filteredChatThreads = useMemo(
    () => chatThreads.filter(({ thread }) => threadMatchesQuery(thread)),
    [chatThreads, threadMatchesQuery],
  );

  // Memoized rather than computed inline: these three are dependencies of the
  // walkable-row list below (and of the row builders), and a fresh array every
  // render would rebuild all of it on every keystroke elsewhere in the app.
  const visibleWorkspaceGroups = useMemo(
    () => (workspaceSectionCollapsed ? [] : workspaceGroups),
    [workspaceSectionCollapsed, workspaceGroups],
  );
  const visiblePinnedThreads = useMemo(
    () => (searchOpen || queryActive ? filteredPinnedThreads : pinnedThreads),
    [filteredPinnedThreads, pinnedThreads, queryActive, searchOpen],
  );
  const visibleChatThreads = useMemo(
    () => (chatSectionCollapsed ? [] : filteredChatThreads),
    [chatSectionCollapsed, filteredChatThreads],
  );
  const hasAnyVisibleRow = useMemo(() => {
    if (!queryActive)
      return true;
    if (mergedMode)
      return filteredRemoteRows.length > 0;
    return filteredPinnedThreads.length > 0
      || filteredChatThreads.length > 0
      || workspaceGroups.some(group => group.rows.length > 0);
  }, [queryActive, mergedMode, filteredRemoteRows, filteredPinnedThreads, filteredChatThreads, workspaceGroups]);

  /**
   * Which row is "current". A local thread is current while a list is showing
   * (both the chat and the workspace section draw conversation rows); a remote
   * row is current once its conversation is the one on screen.
   *
   * The workspace section used to be excluded, so a workspace thread was never
   * "current": the keyboard walk had no place to start and always jumped to the
   * first (or last) row — which is exactly the "within a workspace" step this
   * navigation needs.
   */
  const activeConversationKey = (active === "chat" || active === "workspace") && activeThreadId
    ? `local::${activeThreadId}`
    : activeRemoteKey;

  /**
   * One local row, built once and reused by both list modes. Extracted rather
   * than duplicated so a future row prop cannot be added to one list and
   * forgotten in the other.
   */
  function renderLocalRow(
    thread: StoredThread,
    depth = 0,
    hasChildren = false,
    compact = false,
  ): React.ReactNode {
    return (
      <ThreadListItem
        active={thread.id === activeThreadId && active === "chat"}
        archived={thread.status === "archived"}
        key={thread.id}
        compact={compact}
        menuOpen={openThreadMenuId === thread.id}
        pendingApprovalCount={pendingApprovalCounts.get(thread.id)}
        runStatus={threadRunStatuses[thread.id]}
        isStreaming={threadStreamingStatuses[thread.id]}
        selected={selectedThreadIds.has(thread.id)}
        selectionMode={threadSelectionMode(thread)}
        thread={thread}
        depth={depth}
        hasChildren={hasChildren}
        expanded={expandedThreads.has(thread.id)}
        onToggleExpanded={toggleThreadExpanded}
        unread={unreadThreadIds.has(thread.id)}
        onDeleteThread={onDeleteThread}
        onMenuOpenChange={handleThreadMenuOpenChange}
        onRenameThread={onRenameThread}
        onRestoreThread={onRestoreThread}
        onSelectThread={handleRowSelect}
        onTogglePinThread={onTogglePinThread}
        onToggleSelection={threadSelectionMode(thread) ? toggleThreadSelection : undefined}
      />
    );
  }

  const threadById = useMemo(
    () => new Map(visibleThreads.map(thread => [thread.id, thread])),
    [visibleThreads],
  );

  /**
   * Open a merged-list row: this machine's thread, or the host's conversation.
   *
   * A callback rather than a function declaration, because both walks below are
   * memoized rows producers and this is what they call to act.
   */
  const openRow = useCallback((row: MergedConversation) => {
    if (row.desktopId === null) {
      const thread = threadById.get(row.id);
      if (thread)
        onSelectThread(thread);
      return;
    }
    onOpenRemoteConversation?.(row);
  }, [onOpenRemoteConversation, onSelectThread, threadById]);

  /**
   * The conversation rows the list draws, in the order it draws them — what the
   * fine move (`thread.previous` / `thread.next`) walks.
   *
   * Built here rather than in the shell because the order is this list's own
   * (pinned section first, then the section's groups, a fork under the
   * conversation it came from), and a second copy of it anywhere else would be a
   * copy that drifts. Both list modes produce the same shape, so the commands do
   * not care which one is on screen.
   */
  const walkableRows = useMemo(() => {
    if (mergedMode) {
      return mergedRowTree.rows.map(row => ({
        key: row.key,
        open: () => openRow(row),
      }));
    }
    const rows = active === "workspace"
      ? [...visiblePinnedThreads, ...visibleWorkspaceGroups.flatMap(group => group.rows)]
      : [...visiblePinnedThreads, ...visibleChatThreads];
    return rows.map(row => ({
      key: `local::${row.thread.id}`,
      open: () => onSelectThread(row.thread),
    }));
  }, [
    active,
    mergedMode,
    mergedRowTree.rows,
    onSelectThread,
    openRow,
    visibleChatThreads,
    visiblePinnedThreads,
    visibleWorkspaceGroups,
  ]);

  /**
   * The coarse stops: **a workspace, or one conversation** — the two kinds of
   * thing the rail lists, and the level "which workspace / which chat" answers.
   *
   * This list spans *both* sections on purpose. A chat conversation and a
   * workspace are siblings in the user's mental model even though the rail draws
   * them in two sections, so the coarse move has to cross that boundary; opening
   * a conversation is what switches the section, because the shell derives it
   * from the conversation's own mode. The fine move then walks inside whatever
   * the coarse move landed in — a workspace's conversations, or a conversation's
   * forks — which is the second level of the same model.
   *
   * Each stop carries the conversation ids that count as "on" it, so stepping
   * backwards from a nested conversation returns to its own workspace rather
   * than to the next one over.
   */
  const groupStops = useMemo(() => {
    if (mergedMode) {
      // The merged list draws its rows in groups — pinned, then workspaces, then
      // chats — and `groupMergedRows` is what decides them. Reusing it is the
      // point: a second grouping here would be a second opinion about where a
      // workspace begins.
      const { chats, pinned, workspaces } = groupMergedRows(mergedRowTree.rows, mergedRowTree.roots);
      const stops: { keys: Set<string>; open: () => void }[] = [];
      // A pinned conversation leads the list on its own, so it is a stop of its
      // own; a workspace is one stop holding all its conversations.
      for (const row of pinned)
        stops.push({ keys: new Set([row.key]), open: () => openRow(row) });
      for (const group of workspaces) {
        const first = group.rows[0];
        if (first)
          stops.push({ keys: new Set(group.rows.map(row => row.key)), open: () => openRow(first) });
      }
      // A chat is its own stop; a fork is not (it belongs to the conversation it
      // came from), so only a row that is its own root counts.
      for (const row of chats) {
        if (mergedRowTree.roots.get(row.key)?.key !== row.key)
          continue;
        stops.push({ keys: new Set([row.key]), open: () => openRow(row) });
      }
      return stops;
    }
    const descendantIds = (node: (typeof threadTree)[number]): Set<string> => {
      const ids = new Set<string>([node.item.id]);
      for (const child of node.children) {
        for (const id of descendantIds(child))
          ids.add(id);
      }
      return ids;
    };
    // A pinned workspace conversation is drawn twice over: hoisted into the
    // pinned section *and* listed under its workspace. It belongs to the
    // workspace's stop only — otherwise two stops would both claim it, "next"
    // would keep landing on the same conversation, and the walk would stall.
    const workspaceThreadIds = new Set(
      threads
        .filter(thread => thread.mode === "workspace" && thread.workspaceId)
        .map(thread => thread.id),
    );
    const stops: { keys: Set<string>; open: () => void }[] = [];
    // Pinned conversations lead both lists, so they lead this walk — but only the
    // ones that are not already a workspace's business.
    for (const node of threadTree.filter(node => node.item.pinned)) {
      const keys = new Set(
        [...descendantIds(node)]
          .filter(id => !workspaceThreadIds.has(id))
          .map(id => `local::${id}`),
      );
      if (keys.size === 0)
        continue;
      stops.push({ keys, open: () => onSelectThread(node.item) });
    }
    for (const group of workspaceGroups) {
      const first = group.rows[0]?.thread;
      stops.push({
        keys: new Set(
          threads
            .filter(thread => thread.mode === "workspace" && thread.workspaceId === group.workspace.id)
            .map(thread => `local::${thread.id}`),
        ),
        open: () => {
          if (first)
            onSelectThread(first);
          else
            onSelectWorkspace(group.workspace, []);
        },
      });
    }
    for (const node of threadTree.filter(node => node.item.mode === "chat" && !node.item.pinned)) {
      stops.push({
        keys: new Set([...descendantIds(node)].map(id => `local::${id}`)),
        open: () => onSelectThread(node.item),
      });
    }
    return stops;
  }, [
    mergedMode,
    mergedRowTree.rows,
    onSelectThread,
    onSelectWorkspace,
    mergedRowTree.roots,
    openRow,
    threads,
    threadTree,
    workspaceGroups,
  ]);

  /** Move the selection by one row; from nothing, `next` opens the first row. */
  const stepConversation = useCallback((delta: 1 | -1) => {
    if (walkableRows.length === 0)
      return;
    const index = walkableRows.findIndex(row => row.key === activeConversationKey);
    const target = index === -1
      ? walkableRows[delta === 1 ? 0 : walkableRows.length - 1]
      : walkableRows[index + delta];
    target?.open();
  }, [activeConversationKey, walkableRows]);

  /**
   * Move to the next workspace or conversation.
   *
   * It lands on that stop's first conversation in both directions: the coarse
   * move means "take me to that workspace / that conversation", and the fine
   * move is then available to walk inside it. With nothing selected it starts
   * from the first (or last) stop, like the fine move does.
   */
  const stepGroup = useCallback((delta: 1 | -1) => {
    if (groupStops.length === 0)
      return;
    const index = activeConversationKey === null
      ? -1
      : groupStops.findIndex(stop => stop.keys.has(activeConversationKey));
    if (index === -1) {
      groupStops[delta === 1 ? 0 : groupStops.length - 1]?.open();
      return;
    }
    groupStops[index + delta]?.open();
  }, [activeConversationKey, groupStops]);

  // Only while the rail is showing a list: on Skills, Tasks or Remote there is no
  // order to walk, and the keys stay unclaimed instead of moving an invisible
  // selection.
  const listShowing = active === "chat" || active === "workspace";
  useCommand("thread.previous", () => stepConversation(-1), listShowing && walkableRows.length > 0);
  useCommand("thread.next", () => stepConversation(1), listShowing && walkableRows.length > 0);
  // A single stop has nowhere to step to, so the keys stay free rather than being
  // consumed for nothing.
  useCommand("group.previous", () => stepGroup(-1), listShowing && groupStops.length > 1);
  useCommand("group.next", () => stepGroup(1), listShowing && groupStops.length > 1);
  // Filtering the list is a jump to the field, not a search of its own: the
  // command palette (a separate, later entry) navigates, this narrows.
  useCommand("rail.search", () => setSearchOpen(true), listShowing);

  /**
   * One workspace's heading, built once and used by both list modes.
   *
   * The merged list is handed this same node for this machine's workspaces, so a
   * workspace heading is not merely *like* the local list's — with its folder,
   * its pin marker, its menu and its click-to-select — it is the one the local
   * list renders. A second, similar heading in the merged list is how the two
   * lists drifted apart in the first place.
   */
  function renderWorkspaceHeader(workspace: StoredWorkspace, groupThreads: StoredThread[]): React.ReactNode {
    const collapsed = collapsedWorkspaces.has(workspace.id);
    return (
      // Right-click anywhere on the heading opens the same actions menu as the
      // `...` button.
      <div
        // The heading's group is *named*: its controls (the actions menu, the
        // "new chat" plus) are revealed by hovering this heading. The rail's
        // list area is itself an unnamed `group` — what the floating scrollbar
        // reveals on — so naming this one is what keeps the pointer anywhere
        // over the list from revealing every heading's controls at once.
        className="group/workspace flex h-7 w-full items-center gap-1 rounded-md px-2 text-left transition-colors hover:bg-surface-subtle"
        onContextMenu={(event) => {
          event.preventDefault();
          setOpenWorkspaceMenuId(workspace.id);
        }}
      >
        <RailDisclosure
          collapsed={collapsed}
          label={collapsed ? t("activityRail.expandWorkspace") : t("activityRail.collapseWorkspace")}
          onToggle={() => toggleWorkspaceCollapsed(workspace.id)}
        />
        <button
          className="flex min-w-0 flex-1 items-center gap-1 text-left"
          onClick={() => onSelectWorkspace(workspace, groupThreads)}
          type="button"
        >
          <Folder className="size-4 shrink-0 text-ink-soft" />
          {/* Named for the merged list's sake: the merged list renders *this*
              heading for this machine's workspaces, and its own assertions have
              to be able to address it (a heading whose rows come from another
              component is the pair that can disagree). */}
          <span
            className="min-w-0 flex-1 truncate text-sm font-medium text-ink-soft"
            data-testid="local-group-heading"
            title={workspace.name}
          >
            {workspace.name}
          </span>
          {/* A pinned group leads the list; the marker stays visible on hover
              like the row's own pin state, without a second control. */}
          {workspace.pinned
            ? <Pin aria-hidden className="size-3.5 shrink-0 text-accent" />
            : null}
        </button>
        <WorkspaceHeaderMenu
          open={openWorkspaceMenuId === workspace.id}
          workspace={workspace}
          onDelete={onDeleteWorkspace}
          onOpenChange={open => setOpenWorkspaceMenuId(open ? workspace.id : null)}
          onRename={onRenameWorkspace}
          onTogglePin={onTogglePinWorkspace}
          onSelect={selectionMode ? undefined : () => enterSelectionMode(workspace.id)}
        />
        <RailHeaderButton
          icon={<Plus className="size-3.5" />}
          label={t("activityRail.newChatInWorkspace", { name: workspace.name })}
          onClick={() => onNewChat(workspace.id)}
          // A group heading folds its controls away: the pointer crosses this
          // heading on the way to the list, and the group is already named by
          // the row below it. The remote workspace heading draws the same
          // control from the same component, so the two can no longer disagree
          // about whether it is visible.
          revealOnHover
        />
      </div>
    );
  }

  const toggleLabel = floating
    ? t("activityRail.pinSidebar")
    : expanded
      ? t("activityRail.collapseSidebar")
      : t("activityRail.expandSidebar");

  return (
    <nav
      className={cn(
        "relative flex h-full flex-col bg-surface transition-[width] duration-200",
        floating
          ? "w-full rounded-r-lg border-r border-line-soft/70 shadow-sidebar-floating"
          : "shrink-0 border-r border-line-soft/70",
        expanded ? "w-full" : "w-14 items-center",
      )}
    >
      <div
        className={cn(
          "relative flex h-12 shrink-0 select-none items-center px-2",
          expanded ? "justify-start" : "justify-center",
        )}
        onMouseDown={startWindowDrag}
      >
        <button
          aria-label={toggleLabel}
          title={toggleLabel}
          className={cn(
            "inline-flex size-8 items-center justify-center rounded-md border border-transparent text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink",
            // macOS reserves the top-left for the traffic lights; other platforms
            // (and macOS fullscreen, where the lights hide) sit near the edge.
            expanded && (reserveTrafficLights ? "absolute left-20 top-2" : "absolute left-2 top-2"),
          )}
          onClick={onToggleExpanded}
          type="button"
        >
          <PanelLeft className="size-3.5" />
        </button>
      </div>
      <div className={cn("flex min-h-0 flex-1 flex-col p-2", expanded ? "w-full" : "items-center gap-2")}>
        {expanded
          ? (
              <>
                <div className="mb-3 shrink-0 space-y-0.5">
                  <NavButton icon={SquarePen} label={t("activityRail.newChat")} onClick={() => onNewChat()} primary />
                  <NavButton icon={Sparkles} label={t("activityRail.models")} onClick={onOpenModels} />
                  <div className="relative">
                    <NavButton
                      attention={skillAttention}
                      badge={skillCount != null && skillCount > 0 ? skillCount : null}
                      dot={!skillIntroDismissed}
                      icon={Blocks}
                      label={t("activityRail.skills")}
                      active={active === "skill"}
                      onClick={() => {
                        setSkillAttention(false);
                        onChange("skill");
                      }}
                    />
                    {!skillIntroDismissed && skillCount !== null
                      ? (
                          <SkillIntroBubble
                            count={skillCount}
                            onDismiss={onDismissSkillIntro}
                            onGo={() => {
                              onDismissSkillIntro();
                              setSkillAttention(false);
                              onChange("skill");
                            }}
                          />
                        )
                      : null}
                  </div>
                  {/* Task sits with the other standing entries: it is something
                      you set up and keep, like the remote connection below. */}
                  <NavButton icon={ListChecks} label={t("activityRail.tasks")} active={active === "tasks"} onClick={() => onChange("tasks")} />
                  {/* One entry for both directions of remote.

                      They used to be two, and the labels were the problem:
                      "Phone Control" and "Remote Desktops" are a word apart,
                      neither says which way the connection goes, and neither is
                      even accurate now that a *computer* can be either side.
                      The page inside separates the directions in words.

                      Always shown, unlike the old host-only entry which needed a
                      FutureOS sign-in: the client half does not, so hiding the
                      entry hid a feature that was usable. The section that does
                      need a sign-in says so itself. */}
                  <NavButton
                    active={active === "remote"}
                    icon={MonitorSmartphone}
                    indicator={remoteDot}
                    label={t("activityRail.remote")}
                    onClick={() => onChange("remote")}
                  />
                </div>
                {featureItems.length > 0
                  ? (
                      <div className="mb-3 shrink-0 space-y-0.5">
                        {featureItems.map(item => (
                          <NavButton
                            key={item.id}
                            icon={item.icon}
                            label={item.label}
                            active={active === item.id}
                            onClick={() => onChange(item.id)}
                          />
                        ))}
                      </div>
                    )
                  : null}
                <div className="mb-2 flex shrink-0 items-center gap-1">
                  {/* The search field takes the whole row while it is open: the
                      two were sharing it half-and-half, which squeezed the
                      device picker down to its glyph and the field down to a
                      few characters. Search still filters within the selected
                      device; closing it brings the picker back. */}
                  {onChangeDeviceFilter && remotePeers.length > 0 && !searchOpen
                    ? (
                        <div className="min-w-0 flex-1">
                          <DeviceSelector
                            filter={deviceFilter}
                            onChange={onChangeDeviceFilter}
                            onManage={() => onManageDesktops?.()}
                            peers={remotePeers}
                          />
                        </div>
                      )
                    : null}
                  {selectionMode
                    ? null
                    : (
                        <ConversationSearch
                          onClose={closeSearch}
                          onQueryChange={setSearchQuery}
                          onToggleOpen={() => setSearchOpen(current => !current)}
                          open={searchOpen}
                          query={searchQuery}
                        />
                      )}
                </div>
                <div className="flex min-h-0 flex-1 flex-col">
                  <div className="group relative -mx-2 flex min-h-0 flex-1">
                    <div
                      ref={listScrollbar.scrollRef}
                      className="floating-scrollbar min-h-0 flex-1 overflow-y-auto px-2"
                      data-activity-rail-scroll="true"
                      onScroll={listScrollbar.handleScroll}
                    >
                      <div ref={listScrollbar.contentRef} className="flex min-h-full flex-col" data-activity-rail-content="true">
                        {!hasAnyVisibleRow
                          ? (
                              <div className="px-2 py-1 text-xs text-ink-muted" data-testid="no-matches">{noMatchesText}</div>
                            )
                          : null}
                        {mergedMode
                          ? (
                              <MergedRows
                                activeKey={activeConversationKey}
                                chatSectionCollapsed={chatSectionCollapsed}
                                collapsedWorkspaces={collapsedWorkspaces}
                                filter={deviceFilter}
                                onChangedRemote={() => onRemoteConversationsChanged?.()}
                                onDeletedRemote={conversation => onRemoteConversationDeleted(conversation)}
                                onNewChat={() => onNewChat()}
                                onNewChatInWorkspace={(desktopId, workspaceId) =>
                                  onNewChatOnRemote?.(desktopId, workspaceId)}
                                onNewWorkspace={onNewWorkspace}
                                onOpenRemote={conversation => onOpenRemoteConversation?.(conversation)}
                                onRenameRemote={conversation => onRenameRemoteConversation?.(conversation)}
                                onRenameRemoteWorkspace={target => onRenameRemoteWorkspace?.(target)}
                                onSelectChats={() => enterSelectionMode("chat")}
                                onSelectWorkspaceChats={groupKey => enterSelectionMode(groupKey)}
                                onToggleChatSection={() => setChatSectionCollapsed(value => !value)}
                                onToggleRemoteExpanded={toggleRemoteThreadExpanded}
                                onToggleWorkspace={toggleWorkspaceCollapsed}
                                onToggleWorkspaceSection={() => setWorkspaceSectionCollapsed(value => !value)}
                                peers={remotePeers}
                                remoteSelection={{
                                  selectable: isRemoteRowSelectable,
                                  selected: row => selectedRemoteKeys.has(row.key),
                                  toggle: toggleRemoteSelection,
                                }}
                                expandedRemoteRows={expandedRemoteThreads}
                                rowMeta={mergedRowTree.meta}
                                rowRoots={mergedRowTree.roots}
                                renderLocalGroupHeader={(workspaceId) => {
                                  const found = visibleWorkspaceGroups.find(item => item.workspace.id === workspaceId);
                                  return found ? renderWorkspaceHeader(found.workspace, found.threads) : null;
                                }}
                                renderLocalRow={renderLocalRow}
                                rows={mergedRowTree.rows}
                                selectionMode={selectionMode}
                                threadById={threadById}
                                workspaceSectionCollapsed={workspaceSectionCollapsed}
                              />
                            )
                          : null}
                        {mergedMode
                          ? null
                          : (
                              <>
                                {visiblePinnedThreads.length > 0
                                  ? (
                                      <div className="mb-3 space-y-0.5">
                                        <div className="sticky top-0 z-20 flex h-6 items-center bg-surface px-2 text-xs font-medium text-ink-muted">
                                          <span>{t("activityRail.pinnedHeader")}</span>
                                        </div>
                                        {visiblePinnedThreads.map(({ thread, depth, hasChildren }) => (
                                          <ThreadListItem
                                            active={thread.id === activeThreadId}
                                            archived={thread.status === "archived"}
                                            key={thread.id}
                                            menuOpen={openThreadMenuId === thread.id}
                                            pendingApprovalCount={pendingApprovalCounts.get(thread.id)}
                                            runStatus={threadRunStatuses[thread.id]}
                                            selected={selectedThreadIds.has(thread.id)}
                                            selectionMode={threadSelectionMode(thread)}
                                            thread={thread}
                                            depth={depth}
                                            hasChildren={hasChildren}
                                            expanded={expandedThreads.has(thread.id)}
                                            onToggleExpanded={toggleThreadExpanded}
                                            isStreaming={threadStreamingStatuses[thread.id]}
                                            unread={unreadThreadIds.has(thread.id)}
                                            onDeleteThread={onDeleteThread}
                                            onMenuOpenChange={handleThreadMenuOpenChange}
                                            onRenameThread={onRenameThread}
                                            onRestoreThread={onRestoreThread}
                                            onSelectThread={handleRowSelect}
                                            onTogglePinThread={onTogglePinThread}
                                            onToggleSelection={threadSelectionMode(thread) ? toggleThreadSelection : undefined}
                                          />
                                        ))}
                                      </div>
                                    )
                                  : null}
                                <div className="space-y-0.5">
                                  <div className="sticky top-0 z-20 flex h-6 items-center justify-between bg-surface px-2 text-xs font-medium text-ink-muted">
                                    <span>{t("activityRail.workspace")}</span>
                                    <div className="flex items-center gap-0.5">
                                      <SectionToggle
                                        collapsed={workspaceSectionCollapsed}
                                        label={workspaceSectionCollapsed
                                          ? t("activityRail.expandWorkspaceSection")
                                          : t("activityRail.collapseWorkspaceSection")}
                                        onToggle={() => setWorkspaceSectionCollapsed(value => !value)}
                                      />
                                      <button
                                        aria-label={t("activityRail.newWorkspace")}
                                        className="inline-flex size-5 items-center justify-center rounded text-ink-muted transition-colors hover:bg-surface-subtle hover:text-ink-soft"
                                        onClick={onNewWorkspace}
                                        title={t("activityRail.newWorkspace")}
                                        type="button"
                                      >
                                        <Plus className="size-3.5" />
                                      </button>
                                    </div>
                                  </div>
                                  {!workspaceSectionCollapsed && workspaceGroups.length === 0 && (filteredChatThreads.length === 0 || chatSectionCollapsed)
                                    ? (
                                        <div className="px-2 py-1 text-xs text-ink-muted" data-testid="no-matches-workspace">{noMatchesText}</div>
                                      )
                                    : null}
                                  {visibleWorkspaceGroups.map(({ workspace, threads: groupThreads, rows }) => {
                                    const collapsed = collapsedWorkspaces.has(workspace.id);
                                    return (
                                      <div key={workspace.id} className="space-y-0.5">
                                        {renderWorkspaceHeader(workspace, groupThreads)}
                                        {!collapsed && groupThreads.length > 0
                                          ? (
                                              <div className="space-y-0.5">
                                                {rows.map(({ thread, depth, hasChildren }) => (
                                                  <ThreadListItem
                                                    active={thread.id === activeThreadId}
                                                    archived={thread.status === "archived"}
                                                    key={thread.id}
                                                    menuOpen={openThreadMenuId === thread.id}
                                                    pendingApprovalCount={pendingApprovalCounts.get(thread.id)}
                                                    runStatus={threadRunStatuses[thread.id]}
                                                    selected={selectedThreadIds.has(thread.id)}
                                                    selectionMode={threadSelectionMode(thread)}
                                                    thread={thread}
                                                    depth={depth}
                                                    hasChildren={hasChildren}
                                                    expanded={expandedThreads.has(thread.id)}
                                                    onToggleExpanded={toggleThreadExpanded}
                                                    isStreaming={threadStreamingStatuses[thread.id]}
                                                    unread={unreadThreadIds.has(thread.id)}
                                                    compact
                                                    onDeleteThread={onDeleteThread}
                                                    onMenuOpenChange={handleThreadMenuOpenChange}
                                                    onRenameThread={onRenameThread}
                                                    onRestoreThread={onRestoreThread}
                                                    onSelectThread={handleRowSelect}
                                                    onTogglePinThread={onTogglePinThread}
                                                    onToggleSelection={threadSelectionMode(thread) ? toggleThreadSelection : undefined}
                                                  />
                                                ))}
                                              </div>
                                            )
                                          : null}
                                      </div>
                                    );
                                  })}
                                </div>
                                <div className="mt-3 flex flex-1 flex-col space-y-0.5">
                                  <div className="sticky top-0 z-20 flex h-6 items-center justify-between bg-surface px-2 text-xs font-medium text-ink-muted">
                                    <span>{t("activityRail.chatHeader")}</span>
                                    <div className="flex items-center gap-0.5">
                                      <SectionToggle
                                        collapsed={chatSectionCollapsed}
                                        label={chatSectionCollapsed
                                          ? t("activityRail.expandChatSection")
                                          : t("activityRail.collapseChatSection")}
                                        onToggle={() => setChatSectionCollapsed(value => !value)}
                                      />
                                      {!selectionMode
                                        ? (
                                            <ChatSectionMenu
                                              open={chatSectionMenuOpen}
                                              onOpenChange={setChatSectionMenuOpen}
                                              onSelect={() => enterSelectionMode("chat")}
                                            />
                                          )
                                        : null}
                                      <button
                                        aria-label={t("activityRail.newChatShort")}
                                        className="inline-flex size-5 items-center justify-center rounded text-ink-muted transition-colors hover:bg-surface-subtle hover:text-ink-soft"
                                        onClick={() => onNewChat()}
                                        title={t("activityRail.newChatShort")}
                                        type="button"
                                      >
                                        <Plus className="size-3.5" />
                                      </button>
                                    </div>
                                  </div>
                                  {!chatSectionCollapsed && filteredChatThreads.length === 0
                                    ? (
                                        <div className="px-2 py-1 text-xs text-ink-muted" data-testid="no-matches">{noMatchesText}</div>
                                      )
                                    : null}
                                  <div className="shrink-0 space-y-0.5">
                                    {visibleChatThreads.map(({ thread, depth, hasChildren }) => renderLocalRow(thread, depth, hasChildren))}
                                  </div>
                                </div>
                              </>
                            )}
                      </div>
                    </div>
                    <FloatingScrollbar
                      scrollbar={listScrollbar.scrollbar}
                      onPointerDown={listScrollbar.handleThumbPointerDown}
                    />
                  </div>
                  {selectionMode
                    ? (
                        <ActivityRailSelectionToolbar
                          selectedCount={selectedCount}
                          totalCount={totalCount}
                          onCancel={exitSelectionMode}
                          onDelete={deleteSelected}
                          onToggleAll={selectedCount === totalCount && totalCount > 0 ? deselectAll : selectAll}
                        />
                      )
                    : null}
                </div>
              </>
            )
          : (
              <>
                <IconButton
                  icon={<SquarePen className="size-4" />}
                  label={t("activityRail.newChatShort")}
                  active={false}
                  onClick={() => onNewChat()}
                />
                <IconButton
                  icon={<Sparkles className="size-4" />}
                  label={t("activityRail.models")}
                  active={false}
                  onClick={onOpenModels}
                />
                {featureItems.map((item) => {
                  const Icon = item.icon;
                  return (
                    <IconButton
                      key={item.id}
                      icon={<Icon className="size-4" />}
                      label={item.label}
                      active={active === item.id}
                      onClick={() => onChange(item.id)}
                    />
                  );
                })}
                <IconButton
                  icon={<ListChecks className="size-4" />}
                  label={t("activityRail.tasks")}
                  active={active === "tasks"}
                  onClick={() => onChange("tasks")}
                />
                {/* Always shown, and the same for both directions of remote:
                    the client half needs no sign-in, so gating this on one hid
                    a feature that was usable. */}
                <IconButton
                  icon={(
                    <span className="relative inline-flex">
                      <MonitorSmartphone className="size-4" />
                      {remoteDot}
                    </span>
                  )}
                  label={t("activityRail.remote")}
                  active={active === "remote"}
                  onClick={() => onChange("remote")}
                />
                <IconButton
                  icon={<Folder className="size-4" />}
                  label={t("activityRail.workspace")}
                  active={active === "workspace"}
                  onClick={() => onChange("workspace")}
                />
                <IconButton
                  icon={<MessageSquare className="size-4" />}
                  label={t("activityRail.chat")}
                  active={active === "chat"}
                  onClick={() => onChange("chat")}
                />
              </>
            )}
      </div>
      <ActivityRailAccountFooter
        active={active}
        balance={futureBalance ?? null}
        communityEdition={communityEdition}
        expanded={expanded}
        hasUpdate={hasUpdate}
        onChange={onChange}
        onOpenUpdate={onOpenUpdate}
        onRecharge={onRecharge}
        userEmail={userEmail}
      />
      {!floating ? <div className="pointer-events-none absolute inset-y-0 right-0 z-30 w-6 shadow-sidebar-divider" /> : null}
    </nav>
  );
}

function sortThreads(items: StoredThread[]) {
  return [...items].sort((a, b) => {
    if (a.status !== b.status)
      return a.status === "active" ? -1 : 1;
    if (a.pinned !== b.pinned)
      return a.pinned ? -1 : 1;
    return threadSortTime(b) - threadSortTime(a);
  });
}

function threadSortTime(thread: StoredThread) {
  return thread.lastMessageAt ?? thread.updatedAt ?? thread.createdAt;
}

/**
 * Collapse/expand chevron for a top-level list section header (Workspace / Chat),
 * sized to sit next to that header's `+` button.
 */
function SectionToggle({
  collapsed,
  label,
  onToggle,
}: {
  collapsed: boolean;
  label: string;
  onToggle: () => void;
}) {
  return (
    <button
      aria-expanded={!collapsed}
      aria-label={label}
      className="inline-flex size-5 items-center justify-center rounded text-ink-muted transition-colors hover:bg-surface-subtle hover:text-ink-soft"
      onClick={onToggle}
      title={label}
      type="button"
    >
      {collapsed ? <ChevronRight className="size-3.5" /> : <ChevronDown className="size-3.5" />}
    </button>
  );
}

/**
 * A full-width expanded-rail nav button (New Chat, Models, feature entries). The
 * Settings entry keeps its own accent active style and isn't built on this.
 */
function NavButton({
  icon: Icon,
  label,
  onClick,
  active = false,
  primary = false,
  indicator = null,
  attention = false,
  badge = null,
  dot = false,
}: {
  icon: LucideIcon;
  label: string;
  onClick: () => void;
  active?: boolean;
  /** New Chat: solid ink label with no hover recolor, muted icon. */
  primary?: boolean;
  /** Optional dot overlaid on the icon's top-right (e.g. remote connection). */
  indicator?: ReactNode;
  /** Short accent pulse drawing the eye to this entry (skill-guide dismissal). */
  attention?: boolean;
  /** Count capsule at the row's right edge (Skills installed count). */
  badge?: number | null;
  /** Blue "unread intro" dot at the row's right edge. */
  dot?: boolean;
}) {
  return (
    <button
      className={cn(
        "flex h-8 w-full items-center gap-2 rounded-md border border-transparent px-2 text-sm font-medium transition-colors hover:bg-surface-subtle",
        primary ? "text-ink" : "text-ink-soft hover:text-ink",
        active && "bg-surface-subtle text-ink",
        attention && "animate-skill-entry-pulse border-accent/50 bg-accent-soft text-accent",
      )}
      onClick={onClick}
      type="button"
    >
      <span className="relative inline-flex shrink-0">
        <Icon className={cn("size-4", primary && "text-ink-soft")} />
        {indicator}
      </span>
      <span className="truncate">{label}</span>
      {(badge != null && badge > 0) || dot
        ? (
            <span className="ml-auto flex shrink-0 items-center gap-1.5">
              {badge != null && badge > 0
                ? (
                    <span className="rounded-full bg-accent-soft px-1.5 py-0.5 text-2xs font-semibold leading-none text-accent">
                      {badge}
                    </span>
                  )
                : null}
              {dot ? <span className="size-2 rounded-full bg-accent" /> : null}
            </span>
          )
        : null}
    </button>
  );
}
