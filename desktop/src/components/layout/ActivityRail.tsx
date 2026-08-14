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
  Settings,
  Smartphone,
  Sparkles,
  SquarePen,
  Trash2,
  Wallet,
  X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useBuildInfo } from "../../integrations/tauri/useBuildInfo";
import { cn } from "../../lib/cn";
import { isMacOS } from "../../lib/platform";
import { useDismissableLayer } from "../../lib/useDismissableLayer";
import { useFloatingScrollbar } from "../../lib/useFloatingScrollbar";
import { useIsFullscreen } from "../../lib/useIsFullscreen";
import { startWindowDrag } from "../../lib/windowDrag";
import { Button } from "../ui/Button";
import { FloatingScrollbar } from "../ui/FloatingScrollbar";
import { IconButton } from "../ui/IconButton";
import { MenuPanel } from "../ui/MenuPanel";
import { ChatSectionMenu, WorkspaceHeaderMenu } from "./ActivityRailMenus";
import { usePendingApprovalCounts } from "./hooks/usePendingApprovalCounts";
import { ThreadListItem } from "./ThreadListItem";

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
  /** Remote bridge connection state for the nav indicator dot (dev-only). */
  remoteIndicator?: RemoteIndicator;
  /** FutureOS credit balance (null when signed out). */
  futureBalance?: number | null;
  /** Signed-in FutureOS email (null when signed out). Drives the account menu. */
  userEmail?: string | null;
  /** Opens the recharge page in the system browser. */
  onRecharge?: () => void;
  /** Opens the Settings dialog on the "Check for updates" tab. */
  onOpenUpdate?: () => void;
}

// Data / Skill entries are temporarily hidden from the navigation:
// these modules have been deprioritised (see PLAN.md "Next Priorities").
// Section handling logic is preserved; only the left-nav items are removed —
// add them back to restore. (Research was removed entirely; see PRODUCT.md §4.9.)
const featureItems: Array<{ id: ActivitySection; label: string; icon: LucideIcon }> = [];

const settingsItem = { id: "settings", label: "Settings", icon: Settings } satisfies {
  id: ActivitySection;
  label: string;
  icon: LucideIcon;
};

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
  onRecharge,
  onOpenUpdate,
}: ActivityRailProps) {
  const { t } = useTranslation("layout");
  // Pending approvals across all threads — badged on rail items so a background
  // conversation's approval is visible without opening it.
  const pendingApprovalCounts = usePendingApprovalCounts();
  // Shared overlay scrollbar for the conversation list, matching the chat view.
  const listScrollbar = useFloatingScrollbar();
  // The Remote (phone) feature is still under development — show its nav entry
  // only in dev builds. Hidden while build info is loading so it never flashes
  // into a release build.
  const build = useBuildInfo();
  const showRemote = build.data ? !build.data.isRelease : false;
  // Connection indicator overlaid on the Remote nav icon: blue when connected,
  // amber when the bridge reports an error, nothing when disconnected.
  const remoteDot = showRemote && remoteIndicator
    ? (
        <span
          className={cn(
            "absolute -right-1 -top-1 size-2 rounded-full",
            remoteIndicator === "connected" ? "bg-accent" : "bg-warning",
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
  const [collapsedWorkspaces, setCollapsedWorkspaces] = useState<Set<string>>(() => new Set());
  // Collapse state for the two top-level list sections (Workspace / Chat),
  // independent of the per-workspace group collapse above.
  const [workspaceSectionCollapsed, setWorkspaceSectionCollapsed] = useState(false);
  const [chatSectionCollapsed, setChatSectionCollapsed] = useState(false);
  // Batch selection mode.
  const [selectionMode, setSelectionMode] = useState(false);
  const [selectionScope, setSelectionScope] = useState<"chat" | string>("chat");
  const [selectedThreadIds, setSelectedThreadIds] = useState<Set<string>>(() => new Set());
  // Chat section header menu.
  const [chatSectionMenuOpen, setChatSectionMenuOpen] = useState(false);

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

  // Stable row callbacks for the memoized ThreadListItem: one instance shared
  // by every row (each row passes its own thread back), instead of fresh
  // per-row closures on every render.
  const toggleThreadSelection = useCallback((thread: StoredThread) => {
    setSelectedThreadIds((current) => {
      const next = new Set(current);
      if (next.has(thread.id)) {
        next.delete(thread.id);
      }
      else {
        next.add(thread.id);
      }
      return next;
    });
  }, []);

  const handleThreadMenuOpenChange = useCallback((thread: StoredThread, open: boolean) => {
    setOpenThreadMenuId(open ? thread.id : null);
  }, []);

  function exitSelectionMode() {
    setSelectionMode(false);
    setSelectedThreadIds(new Set());
  }

  // Esc key exits selection mode.
  useEffect(() => {
    if (!selectionMode)
      return;
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        exitSelectionMode();
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [selectionMode]);

  // Derived thread lists are memoized: the rail re-renders whenever any
  // run/streaming status changes, but these depend only on the thread list
  // and selection scope — recomputing the sort + filters (O(W×T) with the
  // workspace grouping) on every render was wasted churn (M3).
  const visibleThreads = useMemo(
    () => sortThreads(threads.filter(thread => thread.status === "active")),
    [threads],
  );

  // Scope-filtered threads for the current selection mode.
  const scopedThreads = useMemo(() => visibleThreads.filter((thread) => {
    if (selectionScope === "chat")
      return thread.mode === "chat";
    return thread.mode === "workspace" && thread.workspaceId === selectionScope;
  }), [selectionScope, visibleThreads]);

  function enterChatSelectionMode() {
    setSelectionScope("chat");
    setSelectedThreadIds(new Set());
    setSelectionMode(true);
  }

  function enterWorkspaceSelectionMode(workspaceId: string) {
    setSelectionScope(workspaceId);
    setSelectedThreadIds(new Set());
    setSelectionMode(true);
  }

  function selectAll() {
    setSelectedThreadIds(new Set(scopedThreads.map(t => t.id)));
  }

  function deselectAll() {
    setSelectedThreadIds(new Set());
  }

  /** Whether a thread is selectable in the current selection scope. */
  const isThreadInScope = useCallback((thread: StoredThread) => {
    if (selectionScope === "chat")
      return thread.mode === "chat";
    return thread.mode === "workspace" && thread.workspaceId === selectionScope;
  }, [selectionScope]);

  // One stable row-click handler for all memoized ThreadListItems: in
  // selection mode it toggles in-scope threads (out-of-scope rows are a
  // deliberate no-op), otherwise it opens the thread.
  const handleRowSelect = useCallback((thread: StoredThread) => {
    if (selectionMode) {
      if (isThreadInScope(thread))
        toggleThreadSelection(thread);
      return;
    }
    onSelectThread(thread);
  }, [isThreadInScope, onSelectThread, selectionMode, toggleThreadSelection]);

  /** Whether this thread should show a selection checkbox. */
  function threadSelectionMode(thread: StoredThread): boolean {
    return selectionMode && isThreadInScope(thread);
  }

  function handleStartBatchDelete() {
    const selectedThreads = visibleThreads.filter(t => selectedThreadIds.has(t.id));
    if (selectedThreads.length === 0)
      return;
    onBatchDeleteThreads(selectedThreads);
    exitSelectionMode();
  }
  // Pinned threads are hoisted into a single global section (regardless of
  // workspace/chat); the per-group lists show only the unpinned rest.
  const pinnedThreads = useMemo(() => visibleThreads.filter(thread => thread.pinned), [visibleThreads]);
  const chatThreads = useMemo(() => visibleThreads.filter(thread => thread.mode === "chat" && !thread.pinned), [visibleThreads]);
  const workspaceThreads = useMemo(() => visibleThreads.filter(thread => thread.mode === "workspace" && !thread.pinned), [visibleThreads]);
  const workspaceGroups = useMemo(() => workspaces
    .filter(workspace => workspace.kind === "user" || workspaceThreads.some(thread => thread.workspaceId === workspace.id))
    .map(workspace => ({
      workspace,
      threads: workspaceThreads.filter(thread => thread.workspaceId === workspace.id),
    })), [workspaceThreads, workspaces]);
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
        expanded ? (floating ? "" : "w-56 md:w-64 xl:w-72") : "w-14 items-center",
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
                  <NavButton icon={Blocks} label={t("activityRail.skills")} active={active === "skill"} onClick={() => onChange("skill")} />
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
                <div className="group relative -mx-2 flex min-h-0 flex-1">
                  <div
                    ref={listScrollbar.scrollRef}
                    className="floating-scrollbar flex min-h-0 flex-1 flex-col overflow-y-auto px-2"
                    onScroll={listScrollbar.handleScroll}
                  >
                    {pinnedThreads.length > 0
                      ? (
                          <div className="mb-3 space-y-0.5">
                            <div className="sticky top-0 z-10 flex h-6 items-center bg-surface px-2 text-xs font-medium text-ink-muted">
                              <span>{t("activityRail.pinnedHeader")}</span>
                            </div>
                            {pinnedThreads.map(thread => (
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
                      <div className="sticky top-0 z-10 flex h-6 items-center justify-between bg-surface px-2 text-xs font-medium text-ink-muted">
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
                      {visibleWorkspaceGroups.map(({ workspace, threads: groupThreads }) => {
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
                                onSelect={selectionMode ? undefined : () => enterWorkspaceSelectionMode(workspace.id)}
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
                                    {groupThreads.map(thread => (
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
                    <div className="mt-3 space-y-0.5">
                      <div className="sticky top-0 z-10 flex h-6 items-center justify-between bg-surface px-2 text-xs font-medium text-ink-muted">
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
                                  onSelect={enterChatSelectionMode}
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
                      {visibleChatThreads.map(thread => (
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
                    {/* Batch selection action bar. */}
                    {selectionMode
                      ? (
                          <div className="sticky bottom-0 z-10 -mx-2 -mb-0.5 border-t border-line-soft bg-surface px-2 py-2">
                            <div className="flex items-center gap-2">
                              <SelectAllCheckbox
                                checked={scopedThreads.length > 0 && selectedThreadIds.size === scopedThreads.length}
                                indeterminate={selectedThreadIds.size > 0 && selectedThreadIds.size < scopedThreads.length}
                                onChange={() => {
                                  if (selectedThreadIds.size === scopedThreads.length) {
                                    deselectAll();
                                  }
                                  else {
                                    selectAll();
                                  }
                                }}
                              />
                              <span className="flex-1 text-xs text-ink-soft">
                                {selectedThreadIds.size > 0
                                  ? t("activityRail.threadsSelected", { count: selectedThreadIds.size })
                                  : t("activityRail.selectAll")}
                              </span>
                              <Button
                                disabled={selectedThreadIds.size === 0}
                                leftIcon={<Trash2 className="size-3.5" />}
                                onClick={handleStartBatchDelete}
                                size="sm"
                                type="button"
                                variant="danger"
                              >
                                {t("activityRail.deleteSelected")}
                              </Button>
                              <Button
                                leftIcon={<X className="size-3.5" />}
                                onClick={exitSelectionMode}
                                size="sm"
                                type="button"
                                variant="secondary"
                              >
                                {t("common:cancel")}
                              </Button>
                            </div>
                          </div>
                        )
                      : null}
                  </div>
                  <FloatingScrollbar
                    scrollbar={listScrollbar.scrollbar}
                    onPointerDown={listScrollbar.handleThumbPointerDown}
                  />
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
      <div className="border-t border-line-soft/40 p-2">
        {expanded
          ? (
              userEmail
                ? (
                    <AccountMenuButton
                      balance={futureBalance ?? null}
                      email={userEmail}
                      hasUpdate={hasUpdate}
                      onOpenSettings={() => onChange(settingsItem.id)}
                      onOpenUpdate={onOpenUpdate}
                      onRecharge={onRecharge}
                    />
                  )
                : (
                    <button
                      className={cn(
                        "flex h-8 w-full items-center gap-2 rounded-md border border-transparent px-2 text-sm font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink",
                        active === settingsItem.id && "border-accent bg-accent-soft text-accent",
                      )}
                      onClick={() => onChange(settingsItem.id)}
                      type="button"
                    >
                      <span className="relative inline-flex shrink-0">
                        <Settings className="size-4" />
                        {hasUpdate ? <span className="absolute -right-1 -top-1 size-2 rounded-full bg-danger" /> : null}
                      </span>
                      <span className="truncate">{t("activityRail.settings")}</span>
                    </button>
                  )
            )
          : (
              <IconButton
                icon={(
                  <span className="relative inline-flex">
                    <Settings className="size-4" />
                    {hasUpdate ? <span className="absolute -right-1 -top-1 size-2 rounded-full bg-danger" /> : null}
                  </span>
                )}
                label={t("activityRail.settings")}
                active={active === settingsItem.id}
                onClick={() => onChange(settingsItem.id)}
              />
            )}
      </div>
      {!floating ? <div className="pointer-events-none absolute inset-y-0 right-0 z-30 w-6 shadow-sidebar-divider" /> : null}
    </nav>
  );
}

/**
 * Signed-in account chip (avatar initial + email prefix) that opens a flat
 * popover with Settings / Balance (+Recharge). An Upgrade button sits to the
 * right of the avatar when a new app version is available (opening the Settings
 * update tab). Replaces the plain Settings button at the bottom of the rail.
 *
 * The chip is a flex row of two sibling buttons — the trigger (avatar + name)
 * and the upgrade action — because nesting a `<button>` inside the trigger
 * `<button>` is invalid HTML.
 */
function AccountMenuButton({
  balance,
  email,
  hasUpdate,
  onOpenSettings,
  onOpenUpdate,
  onRecharge,
}: {
  balance: number | null;
  email: string;
  hasUpdate?: boolean;
  onOpenSettings: () => void;
  onOpenUpdate?: () => void;
  onRecharge?: () => void;
}) {
  const { t } = useTranslation("layout");
  const [open, setOpen] = useState(false);
  const layerRef = useDismissableLayer<HTMLDivElement>({ enabled: open, onDismiss: () => setOpen(false) });

  const prefix = email.split("@")[0] || email;
  const initial = (prefix[0] ?? "?").toUpperCase();
  const close = () => setOpen(false);

  return (
    <div className="relative" ref={layerRef}>
      <div
        className="flex w-full cursor-pointer items-center gap-2 rounded-md border border-transparent px-2 py-1.5 text-left transition-colors hover:bg-surface-subtle focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
        onClick={() => setOpen(value => !value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            setOpen(value => !value);
          }
        }}
        role="button"
        tabIndex={0}
      >
        <span className="flex size-7 shrink-0 items-center justify-center rounded-full bg-accent-soft text-sm font-semibold uppercase leading-none text-accent">
          {initial}
        </span>
        <span className="min-w-0 flex-1 truncate text-sm font-medium text-ink">{prefix}</span>
        {hasUpdate
          ? (
              <button
                className="ml-auto shrink-0 rounded bg-accent px-1.5 py-0.5 text-[11px] font-medium leading-none text-white transition-colors hover:bg-accent-hover"
                onClick={(event) => {
                  event.stopPropagation();
                  onOpenUpdate?.();
                  close();
                }}
                onKeyDown={event => event.stopPropagation()}
                type="button"
              >
                {t("userMenu.upgrade")}
              </button>
            )
          : null}
      </div>
      {open
        ? (
            <MenuPanel className="absolute bottom-full left-0 right-0 z-40 mb-2 overflow-hidden p-0">
              <MenuRow
                icon={Settings}
                label={t("activityRail.settings")}
                onClick={() => {
                  onOpenSettings();
                  close();
                }}
              />
              <MenuRow
                action={<ActionBadge>{t("userMenu.recharge")}</ActionBadge>}
                icon={Wallet}
                label={t("userMenu.balance", { credits: balance != null ? Math.trunc(balance) : "—" })}
                onClick={() => {
                  onRecharge?.();
                  close();
                }}
              />
            </MenuPanel>
          )
        : null}
    </div>
  );
}

/** A borderless row inside the account popover; the whole row is clickable. */
function MenuRow({ action, icon: Icon, label, onClick }: { action?: ReactNode; icon?: LucideIcon; label: string; onClick: () => void }) {
  return (
    <button
      className="flex w-full items-center justify-between gap-2 px-3 py-2 text-left text-sm text-ink transition-colors hover:bg-surface-subtle"
      onClick={onClick}
      type="button"
    >
      {Icon ? <Icon className="size-4 shrink-0 text-ink-soft" /> : null}
      <span className="min-w-0 flex-1 truncate">{label}</span>
      {action ?? null}
    </button>
  );
}

/** Solid accent pill used as the right-hand action label (Recharge / Upgrade). */
function ActionBadge({ children }: { children: ReactNode }) {
  return (
    <span className="shrink-0 rounded bg-accent px-1.5 py-0.5 text-[11px] font-medium leading-none text-white">
      {children}
    </span>
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
  return thread.lastMessageAt ?? thread.lastOpenedAt ?? thread.updatedAt ?? thread.createdAt;
}

/** Tri-state checkbox for select-all / deselect-all in the selection action bar. */
function SelectAllCheckbox({
  checked,
  indeterminate,
  onChange,
}: {
  checked: boolean;
  indeterminate: boolean;
  onChange: () => void;
}) {
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (ref.current)
      ref.current.indeterminate = indeterminate;
  }, [indeterminate]);
  return (
    <input
      ref={ref}
      checked={checked}
      className="size-4 shrink-0 rounded border-line accent-accent"
      onChange={onChange}
      type="checkbox"
    />
  );
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
}: {
  icon: LucideIcon;
  label: string;
  onClick: () => void;
  active?: boolean;
  /** New Chat: solid ink label with no hover recolor, muted icon. */
  primary?: boolean;
  /** Optional dot overlaid on the icon's top-right (e.g. remote connection). */
  indicator?: ReactNode;
}) {
  return (
    <button
      className={cn(
        "flex h-8 w-full items-center gap-2 rounded-md border border-transparent px-2 text-sm font-medium transition-colors hover:bg-surface-subtle",
        primary ? "text-ink" : "text-ink-soft hover:text-ink",
        active && "bg-surface-subtle text-ink",
      )}
      onClick={onClick}
      type="button"
    >
      <span className="relative inline-flex shrink-0">
        <Icon className={cn("size-4", primary && "text-ink-soft")} />
        {indicator}
      </span>
      <span className="truncate">{label}</span>
    </button>
  );
}
