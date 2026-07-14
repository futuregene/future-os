import type { LucideIcon } from "lucide-react";
import type { StoredThread, StoredWorkspace } from "../../integrations/storage/threadStore";
import type { ThreadRunInfo } from "./hooks/useThreadStore";
import {
  Blocks,
  ChevronDown,
  ChevronRight,
  Folder,
  MessageSquare,
  PanelLeftClose,
  PanelLeftOpen,
  Plus,
  Settings,
  Smartphone,
  Sparkles,
  SquarePen,
} from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useBuildInfo } from "../../integrations/tauri/useBuildInfo";
import { cn } from "../../lib/cn";
import { isMacOS } from "../../lib/platform";
import { useFloatingScrollbar } from "../../lib/useFloatingScrollbar";
import { useIsFullscreen } from "../../lib/useIsFullscreen";
import { startWindowDrag } from "../../lib/windowDrag";
import { FloatingScrollbar } from "../ui/FloatingScrollbar";
import { IconButton } from "../ui/IconButton";
import { WorkspaceHeaderMenu } from "./ActivityRailMenus";
import { ThreadListItem } from "./ThreadListItem";

export type ActivitySection = "chat" | "workspace" | "data" | "skill" | "remote" | "settings";

interface ActivityRailProps {
  active: ActivitySection;
  expanded: boolean;
  floating?: boolean;
  activeThreadId: string | null;
  threads: StoredThread[];
  threadRunStatuses: Record<string, ThreadRunInfo | undefined>;
  unreadThreadIds: Set<string>;
  workspaces: StoredWorkspace[];
  onChange: (section: ActivitySection) => void;
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
  threads,
  threadRunStatuses,
  unreadThreadIds,
  workspaces,
  onChange,
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
}: ActivityRailProps) {
  const { t } = useTranslation("layout");
  // Shared overlay scrollbar for the conversation list, matching the chat view.
  const listScrollbar = useFloatingScrollbar();
  // The Remote (phone) feature is still under development — show its nav entry
  // only in dev builds. Hidden while build info is loading so it never flashes
  // into a release build.
  const build = useBuildInfo();
  const showRemote = build.data ? !build.data.isRelease : false;
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
  const visibleThreads = sortThreads(
    threads.filter(thread => thread.status === "active"),
  );
  // Pinned threads are hoisted into a single global section (regardless of
  // workspace/chat); the per-group lists show only the unpinned rest.
  const pinnedThreads = visibleThreads.filter(thread => thread.pinned);
  const chatThreads = visibleThreads.filter(thread => thread.mode === "chat" && !thread.pinned);
  const workspaceThreads = visibleThreads.filter(thread => thread.mode === "workspace" && !thread.pinned);
  const workspaceGroups = workspaces
    .filter(workspace => workspace.kind === "user" || workspaceThreads.some(thread => thread.workspaceId === workspace.id))
    .map(workspace => ({
      workspace,
      threads: workspaceThreads.filter(thread => thread.workspaceId === workspace.id),
    }));
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
          {expanded && !floating
            ? (
                <PanelLeftClose className="size-3.5" />
              )
            : (
                <PanelLeftOpen className="size-3.5" />
              )}
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
                    ? <NavButton icon={Smartphone} label={t("activityRail.remote")} active={active === "remote"} onClick={() => onChange("remote")} />
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
                                runStatus={threadRunStatuses[thread.id]}
                                thread={thread}
                                unread={unreadThreadIds.has(thread.id)}
                                onDeleteThread={onDeleteThread}
                                onMenuOpenChange={open => setOpenThreadMenuId(open ? thread.id : null)}
                                onRenameThread={onRenameThread}
                                onRestoreThread={onRestoreThread}
                                onSelectThread={onSelectThread}
                                onTogglePinThread={onTogglePinThread}
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
                                        runStatus={threadRunStatuses[thread.id]}
                                        thread={thread}
                                        unread={unreadThreadIds.has(thread.id)}
                                        compact
                                        onDeleteThread={onDeleteThread}
                                        onMenuOpenChange={open => setOpenThreadMenuId(open ? thread.id : null)}
                                        onRenameThread={onRenameThread}
                                        onRestoreThread={onRestoreThread}
                                        onSelectThread={onSelectThread}
                                        onTogglePinThread={onTogglePinThread}
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
                          runStatus={threadRunStatuses[thread.id]}
                          thread={thread}
                          unread={unreadThreadIds.has(thread.id)}
                          onDeleteThread={onDeleteThread}
                          onMenuOpenChange={open => setOpenThreadMenuId(open ? thread.id : null)}
                          onRenameThread={onRenameThread}
                          onRestoreThread={onRestoreThread}
                          onSelectThread={onSelectThread}
                          onTogglePinThread={onTogglePinThread}
                        />
                      ))}
                    </div>
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
                        icon={<Smartphone className="size-4" />}
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
      <div className={cn("border-t border-line-soft/40 p-2", expanded ? "w-full" : "")}>
        {expanded
          ? (
              <button
                className={cn(
                  "flex h-8 w-full items-center gap-2 rounded-md border border-transparent px-2 text-sm font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink",
                  active === settingsItem.id && "border-accent bg-accent-soft text-accent",
                )}
                onClick={() => onChange(settingsItem.id)}
                type="button"
              >
                <Settings className="size-4 shrink-0" />
                <span className="truncate">{t("activityRail.settings")}</span>
              </button>
            )
          : (
              <IconButton
                icon={<Settings className="size-4" />}
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
}: {
  icon: LucideIcon;
  label: string;
  onClick: () => void;
  active?: boolean;
  /** New Chat: solid ink label with no hover recolor, muted icon. */
  primary?: boolean;
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
      <Icon className={cn("size-4 shrink-0", primary && "text-ink-soft")} />
      <span className="truncate">{label}</span>
    </button>
  );
}
