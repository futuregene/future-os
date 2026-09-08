import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import type { StoredThread, StoredWorkspace } from "../../integrations/storage/threadStore";
import type { RemoteIndicator } from "./hooks/useRemoteStatus";
import type { ThreadRunInfo } from "./hooks/useThreadStore";
import {
  Blocks,
  ChevronDown,
  ChevronRight,
  Folder,
  MessageSquare,
  PanelLeft,
  Plus,
  Smartphone,
  Sparkles,
  SquarePen,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
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
import { usePendingApprovalCounts } from "./hooks/usePendingApprovalCounts";
import { useRailSelection } from "./hooks/useRailSelection";
import { ThreadListItem } from "./ThreadListItem";
import { buildThreadTree, visibleThreadRows } from "./threadTree";

export type ActivitySection = "chat" | "workspace" | "skill" | "remote" | "settings";

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
  onDeleteThread: (thread: StoredThread) => void;
  onNewChat: (workspaceId?: string) => void;
  onOpenModels: () => void;
  onNewWorkspace: () => void;
  onRenameThread: (thread: StoredThread) => void;
  onRenameWorkspace: (workspace: StoredWorkspace) => void;
  onDeleteWorkspace: (workspace: StoredWorkspace) => void;
  onRestoreThread: (thread: StoredThread) => void;
  onSelectWorkspace: (workspace: StoredWorkspace, threads: StoredThread[]) => void;
  onSelectThread: (thread: StoredThread) => void;
  onTogglePinThread: (thread: StoredThread) => void;
  onToggleExpanded: () => void;
  /** Remote bridge connection state for the nav indicator dot. */
  remoteIndicator?: RemoteIndicator;
  /** FutureOS credit balance (null when signed out). */
  futureBalance?: number | null;
  /** Signed-in FutureOS email (null when signed out). Drives the account menu. */
  userEmail?: string | null;
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
  onDeleteThread,
  onNewChat,
  onOpenModels,
  onNewWorkspace,
  onRenameThread,
  onRenameWorkspace,
  onDeleteWorkspace,
  onRestoreThread,
  onSelectWorkspace,
  onSelectThread,
  onTogglePinThread,
  onToggleExpanded,
  remoteIndicator,
  futureBalance,
  userEmail,
  communityEdition,
  onRecharge,
  onOpenUpdate,
  skillIntroDismissed,
  onDismissSkillIntro,
}: ActivityRailProps) {
  const { t } = useTranslation("layout");
  // Pending approvals across all threads — badged on rail items so a background
  // conversation's approval is visible without opening it.
  const pendingApprovalCounts = usePendingApprovalCounts();
  // Shared overlay scrollbar for the conversation list, matching the chat view.
  const listScrollbar = useFloatingScrollbar();
  // Remote pairing issues its code through the FutureOS service, so it needs a
  // sign-in. Hide the nav entry while signed out.
  const showRemote = userEmail != null;
  // Connection indicator overlaid on the Remote nav icon: blue when connected,
  // amber while recovery is in progress, red for an actionable bridge error,
  // and nothing when disconnected.
  const remoteDot = remoteIndicator
    ? (
        <span
          className={cn(
            "absolute -right-1 -top-1 size-2 rounded-full",
            remoteIndicator === "connected"
              ? "bg-accent"
              : remoteIndicator === "reconnecting"
                ? "bg-warning"
                : "bg-danger",
            remoteIndicator === "reconnecting" && "animate-pulse",
          )}
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
  const [collapsedWorkspaces, setCollapsedWorkspaces] = useState<Set<string>>(() => new Set());
  // Collapse state for the two top-level list sections (Workspace / Chat),
  // independent of the per-workspace group collapse above.
  const [workspaceSectionCollapsed, setWorkspaceSectionCollapsed] = useState(false);
  const [chatSectionCollapsed, setChatSectionCollapsed] = useState(false);
  // Chat section header menu.
  const [chatSectionMenuOpen, setChatSectionMenuOpen] = useState(false);
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

  function toggleWorkspaceCollapsed(workspaceId: string) {
    setCollapsedWorkspaces((current) => {
      const next = new Set(current);
      if (next.has(workspaceId))
        next.delete(workspaceId);
      else
        next.add(workspaceId);
      return next;
    });
  }

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
      const scope = root.thread.mode === "chat" ? "chat" : root.thread.workspaceId;
      for (const { thread } of visibleThreadRows([root], allExpanded)) scopes.set(thread.id, scope);
    }
    return scopes;
  }, [threadTree, visibleThreads]);
  const {
    deleteSelected,
    deselectAll,
    enterSelectionMode,
    exitSelectionMode,
    handleRowSelect,
    isThreadInScope,
    scopedThreads,
    selectAll,
    selectedThreadIds,
    selectionMode,
    toggleThreadSelection,
  } = useRailSelection({ onBatchDeleteThreads, onSelectThread, threadScopes, visibleThreads });

  /** Whether this thread should show a selection checkbox. */
  function threadSelectionMode(thread: StoredThread): boolean {
    return selectionMode && isThreadInScope(thread);
  }

  // Pinned threads are hoisted into a single global section (regardless of
  // workspace/chat); the per-group lists show only the unpinned rest.
  const pinnedThreads = useMemo(() => visibleThreadRows(threadTree.filter(node => node.thread.pinned), expandedThreads), [threadTree, expandedThreads]);
  const chatThreads = useMemo(() => visibleThreadRows(threadTree.filter(node => node.thread.mode === "chat" && !node.thread.pinned), expandedThreads), [threadTree, expandedThreads]);
  const workspaceRoots = useMemo(() => threadTree.filter(node => node.thread.mode === "workspace" && !node.thread.pinned), [threadTree]);
  const workspaceGroups = useMemo(() => workspaces
    .filter(workspace => workspace.kind === "user" || workspaceRoots.some(node => node.thread.workspaceId === workspace.id))
    .map((workspace) => {
      const roots = workspaceRoots.filter(node => node.thread.workspaceId === workspace.id);
      return {
        workspace,
        rows: visibleThreadRows(roots, expandedThreads),
        threads: visibleThreadRows(roots, new Set(visibleThreads.map(thread => thread.id))).map(row => row.thread),
      };
    }), [workspaceRoots, workspaces, expandedThreads, visibleThreads]);
  const visibleWorkspaceGroups = workspaceSectionCollapsed ? [] : workspaceGroups;
  const visibleChatThreads = chatSectionCollapsed ? [] : chatThreads;
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
                  {showRemote
                    ? <NavButton icon={Smartphone} indicator={remoteDot} label={t("activityRail.remote")} active={active === "remote"} onClick={() => onChange("remote")} />
                    : null}
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
                <div className="flex min-h-0 flex-1 flex-col">
                  <div className="group relative -mx-2 flex min-h-0 flex-1">
                    <div
                      ref={listScrollbar.scrollRef}
                      className="floating-scrollbar min-h-0 flex-1 overflow-y-auto px-2"
                      data-activity-rail-scroll="true"
                      onScroll={listScrollbar.handleScroll}
                    >
                      <div ref={listScrollbar.contentRef} className="flex min-h-full flex-col" data-activity-rail-content="true">
                        {pinnedThreads.length > 0
                          ? (
                              <div className="mb-3 space-y-0.5">
                                <div className="sticky top-0 z-20 flex h-6 items-center bg-surface px-2 text-xs font-medium text-ink-muted">
                                  <span>{t("activityRail.pinnedHeader")}</span>
                                </div>
                                {pinnedThreads.map(({ thread, depth, hasChildren }) => (
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
                          {!workspaceSectionCollapsed && workspaceGroups.length === 0
                            ? (
                                <div className="px-2 py-1 text-xs text-ink-muted">{t("activityRail.noWorkspaceThreads")}</div>
                              )
                            : null}
                          {visibleWorkspaceGroups.map(({ workspace, threads: groupThreads, rows }) => {
                            const collapsed = collapsedWorkspaces.has(workspace.id);
                            return (
                              <div key={workspace.id} className="space-y-0.5">
                                {/* Group header: hover only, no selected state (req 4).
                                Right-click anywhere on the row opens the same
                                actions menu as the `...` button. */}
                                <div
                                  className="group flex h-7 w-full items-center gap-1 rounded-md px-2 text-left transition-colors hover:bg-surface-subtle"
                                  onContextMenu={(event) => {
                                    event.preventDefault();
                                    setOpenWorkspaceMenuId(workspace.id);
                                  }}
                                >
                                  <button
                                    aria-label={collapsed ? t("activityRail.expandWorkspace") : t("activityRail.collapseWorkspace")}
                                    className="inline-flex size-4 shrink-0 items-center justify-center text-ink-muted transition-colors hover:text-ink-soft"
                                    onClick={() => toggleWorkspaceCollapsed(workspace.id)}
                                    type="button"
                                  >
                                    {collapsed ? <ChevronRight className="size-3.5" /> : <ChevronDown className="size-3.5" />}
                                  </button>
                                  <button
                                    className="flex min-w-0 flex-1 items-center gap-2 text-left"
                                    onClick={() => onSelectWorkspace(workspace, groupThreads)}
                                    type="button"
                                  >
                                    <Folder className="size-4 shrink-0 text-ink-soft" />
                                    <span className="min-w-0 flex-1 truncate text-sm font-medium text-ink-soft" title={workspace.name}>
                                      {workspace.name}
                                    </span>
                                  </button>
                                  <WorkspaceHeaderMenu
                                    open={openWorkspaceMenuId === workspace.id}
                                    workspace={workspace}
                                    onDelete={onDeleteWorkspace}
                                    onOpenChange={open => setOpenWorkspaceMenuId(open ? workspace.id : null)}
                                    onRename={onRenameWorkspace}
                                    onSelect={selectionMode ? undefined : () => enterSelectionMode(workspace.id)}
                                  />
                                  <button
                                    aria-label={t("activityRail.newChatInWorkspace", { name: workspace.name })}
                                    className="inline-flex size-5 shrink-0 items-center justify-center rounded text-ink-muted opacity-0 transition hover:bg-surface hover:text-ink-soft group-hover:opacity-100"
                                    onClick={() => onNewChat(workspace.id)}
                                    title={t("activityRail.newChatInWorkspace", { name: workspace.name })}
                                    type="button"
                                  >
                                    <Plus className="size-3.5" />
                                  </button>
                                </div>
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
                          {!chatSectionCollapsed && chatThreads.length === 0
                            ? <div className="px-2 py-1 text-xs text-ink-muted">{t("activityRail.noChats")}</div>
                            : null}
                          <div className="shrink-0 space-y-0.5">
                            {visibleChatThreads.map(({ thread, depth, hasChildren }) => (
                              <ThreadListItem
                                active={thread.id === activeThreadId && active === "chat"}
                                archived={thread.status === "archived"}
                                key={thread.id}
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
                            ))}
                          </div>
                        </div>
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
                          selectedCount={selectedThreadIds.size}
                          totalCount={scopedThreads.length}
                          onCancel={exitSelectionMode}
                          onDelete={deleteSelected}
                          onToggleAll={selectedThreadIds.size === scopedThreads.length ? deselectAll : selectAll}
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
                {showRemote
                  ? (
                      <IconButton
                        icon={(
                          <span className="relative inline-flex">
                            <Smartphone className="size-4" />
                            {remoteDot}
                          </span>
                        )}
                        label={t("activityRail.remote")}
                        active={active === "remote"}
                        onClick={() => onChange("remote")}
                      />
                    )
                  : null}
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
                    <span className="rounded-full bg-accent-soft px-1.5 py-0.5 text-[11px] font-semibold leading-none text-accent">
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
