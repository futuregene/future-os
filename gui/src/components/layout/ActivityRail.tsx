import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import type { StoredThread, StoredWorkspace } from "../../integrations/storage/threadStore";
import type { ThreadRunInfo } from "./hooks/useThreadStore";
import {
  Archive,
  Blocks,
  ChevronDown,
  ChevronRight,
  Folder,
  MessageSquare,
  MoreHorizontal,
  PanelLeftClose,
  PanelLeftOpen,
  Pencil,
  Pin,
  Plus,
  Settings,
  Smartphone,
  Sparkles,
  SquarePen,
  Trash2,
} from "lucide-react";
import { useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "../../lib/cn";
import { isMacOS } from "../../lib/platform";
import { useDismissableLayer } from "../../lib/useDismissableLayer";
import { startWindowDrag } from "../../lib/windowDrag";
import { IconButton } from "../ui/IconButton";

export type ActivitySection = "chat" | "workspace" | "research" | "data" | "skill" | "remote" | "settings";

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

// Research / Data / Skill 入口暂时从导航隐藏：这些模块优先级下调（见 PLAN.md
// 「Next Priorities」）。section 处理逻辑保留（markdown research embed 仍可跳转），
// 仅移除左侧导航项；恢复时把对应条目加回即可。
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
  const [openThreadMenuId, setOpenThreadMenuId] = useState<string | null>(null);
  const [collapsedWorkspaces, setCollapsedWorkspaces] = useState<Set<string>>(() => new Set());

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
  const toggleLabel = floating
    ? t("activityRail.pinSidebar")
    : expanded
      ? t("activityRail.collapseSidebar")
      : t("activityRail.expandSidebar");

  return (
    <nav
      className={cn(
        "flex h-full flex-col bg-surface transition-[width] duration-200",
        floating
          ? "w-full rounded-r-lg border-r border-line-soft/70 shadow-[10px_0_28px_rgba(15,23,42,0.12)]"
          : "shrink-0 border-r border-line-soft",
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
            // don't, so the toggle sits near the edge.
            expanded && (isMacOS ? "absolute left-20 top-2" : "absolute left-2 top-2"),
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
                  <button
                    className="flex h-8 w-full items-center gap-2 rounded-md border border-transparent px-2 text-sm font-medium text-ink transition-colors hover:bg-surface-subtle"
                    onClick={() => onNewChat()}
                    type="button"
                  >
                    <SquarePen className="size-4 shrink-0 text-ink-soft" />
                    <span className="truncate">{t("activityRail.newChat")}</span>
                  </button>
                  <button
                    className="flex h-8 w-full items-center gap-2 rounded-md border border-transparent px-2 text-sm font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
                    onClick={onOpenModels}
                    type="button"
                  >
                    <Sparkles className="size-4 shrink-0" />
                    <span className="truncate">{t("activityRail.models")}</span>
                  </button>
                  <button
                    className={cn(
                      "flex h-8 w-full items-center gap-2 rounded-md border border-transparent px-2 text-sm font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink",
                      active === "skill" && "bg-surface-subtle text-ink",
                    )}
                    onClick={() => onChange("skill")}
                    type="button"
                  >
                    <Blocks className="size-4 shrink-0" />
                    <span className="truncate">{t("activityRail.skills")}</span>
                  </button>
                  <button
                    className={cn(
                      "flex h-8 w-full items-center gap-2 rounded-md border border-transparent px-2 text-sm font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink",
                      active === "remote" && "bg-surface-subtle text-ink",
                    )}
                    onClick={() => onChange("remote")}
                    type="button"
                  >
                    <Smartphone className="size-4 shrink-0" />
                    <span className="truncate">{t("activityRail.remote")}</span>
                  </button>
                </div>
                {featureItems.length > 0
                  ? (
                      <div className="mb-3 shrink-0 space-y-0.5">
                        {featureItems.map((item) => {
                          const Icon = item.icon;
                          return (
                            <button
                              key={item.id}
                              className={cn(
                                "flex h-8 w-full items-center gap-2 rounded-md border border-transparent px-2 text-sm font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink",
                                active === item.id && "bg-surface-subtle text-ink",
                              )}
                              onClick={() => onChange(item.id)}
                              type="button"
                            >
                              <Icon className="size-4 shrink-0" />
                              <span className="truncate">{item.label}</span>
                            </button>
                          );
                        })}
                      </div>
                    )
                  : null}
                <div className="scrollbar-hover -mx-2 flex min-h-0 flex-1 flex-col px-2">
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
                    {workspaceGroups.length === 0
                      ? (
                          <div className="px-2 py-1 text-xs text-ink-muted">{t("activityRail.noWorkspaceThreads")}</div>
                        )
                      : null}
                    {workspaceGroups.map(({ workspace, threads: groupThreads }) => {
                      const collapsed = collapsedWorkspaces.has(workspace.id);
                      return (
                        <div key={workspace.id} className="space-y-0.5">
                          {/* Group header: hover only, no selected state (req 4). */}
                          <div className="group flex h-7 w-full items-center gap-1 rounded-md px-2 text-left transition-colors hover:bg-surface-subtle">
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
                              <span className="min-w-0 flex-1 truncate text-sm font-medium text-ink-soft">
                                {workspace.name}
                              </span>
                            </button>
                            <WorkspaceHeaderMenu
                              workspace={workspace}
                              onDelete={onDeleteWorkspace}
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
                    {chatThreads.length === 0 ? <div className="px-2 py-1 text-xs text-ink-muted">{t("activityRail.noChats")}</div> : null}
                    {chatThreads.map(thread => (
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
                <IconButton
                  icon={<Smartphone className="size-4" />}
                  label={t("activityRail.remote")}
                  active={active === "remote"}
                  onClick={() => onChange("remote")}
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
      <div className={cn("p-2", expanded ? "w-full" : "")}>
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
    </nav>
  );
}

function ThreadListItem({
  active,
  archived,
  compact,
  menuOpen,
  runStatus,
  thread,
  unread,
  onDeleteThread,
  onMenuOpenChange,
  onRenameThread,
  onRestoreThread,
  onSelectThread,
  onTogglePinThread,
}: {
  active: boolean;
  archived?: boolean;
  compact?: boolean;
  menuOpen: boolean;
  runStatus?: ThreadRunInfo;
  thread: StoredThread;
  unread?: boolean;
  onDeleteThread: (thread: StoredThread) => void;
  onMenuOpenChange: (open: boolean) => void;
  onRenameThread: (thread: StoredThread) => void;
  onRestoreThread: (thread: StoredThread) => void;
  onSelectThread: (thread: StoredThread) => void;
  onTogglePinThread: (thread: StoredThread) => void;
}) {
  const { t } = useTranslation("layout");
  const menuRef = useDismissableLayer<HTMLDivElement>({
    enabled: menuOpen,
    onDismiss: () => onMenuOpenChange(false),
  });

  return (
    <div
      ref={menuRef}
      className={cn(
        // Full-width row; workspace threads (compact) indent their content via
        // padding so the highlight still spans the full width (req 2).
        "group/thread relative flex w-full items-center gap-1 rounded-md pr-2 text-left transition-colors hover:bg-surface-subtle",
        compact ? "h-7 pl-7" : "h-8 gap-2 pl-2",
        active && "bg-surface-subtle text-ink",
      )}
    >
      {/* Full-row click target so the whole (highlighted) row selects the
          thread. Content below sits on top but is pointer-events-none so clicks
          fall through to this button; the actions trigger and menu keep a higher
          stacking (z-10 / z-40) so they stay clickable and never mis-fire. */}
      <button
        aria-label={thread.title}
        className="absolute inset-0 rounded-md"
        onClick={() => onSelectThread(thread)}
        type="button"
      />
      {/* Spacer keeps the non-compact title indent after dropping the (uniform,
          meaningless) chat-bubble icon. */}
      {!compact ? <span className="pointer-events-none size-4 shrink-0" /> : null}
      <span
        className={cn(
          "pointer-events-none min-w-0 flex-1 truncate text-sm font-medium",
          archived ? "text-ink-muted" : "text-ink-soft",
        )}
      >
        {thread.title}
      </span>
      {archived ? <span className="pointer-events-none shrink-0 text-[11px] text-ink-muted group-hover/thread:hidden">{t("activityRail.archived")}</span> : null}
      <span className="pointer-events-none flex shrink-0">
        <ThreadRunIndicator status={runStatus?.status} unread={unread} />
      </span>
      <button
        aria-label={t("activityRail.threadActions", { title: thread.title })}
        className={cn(
          "relative z-10 hidden size-5 shrink-0 items-center justify-center rounded text-ink-muted transition-colors hover:bg-surface-subtle hover:text-ink-soft group-hover/thread:inline-flex",
          menuOpen && "inline-flex",
        )}
        onClick={(event) => {
          event.stopPropagation();
          onMenuOpenChange(!menuOpen);
        }}
        title={t("activityRail.threadActions", { title: thread.title })}
        type="button"
      >
        <MoreHorizontal className="size-3.5" />
      </button>
      {menuOpen
        ? (
            <ThreadItemMenu
              archived={archived}
              pinned={thread.pinned}
              onClose={() => onMenuOpenChange(false)}
              onDelete={() => onDeleteThread(thread)}
              onRename={() => onRenameThread(thread)}
              onRestore={() => onRestoreThread(thread)}
              onTogglePin={() => onTogglePinThread(thread)}
            />
          )
        : null}
    </div>
  );
}

/**
 * Bottom edge (viewport px) of the nearest scroll/clip ancestor, or the
 * viewport height when none clips — used to decide if a menu must flip up.
 */
function clippingBottom(element: HTMLElement): number {
  let node = element.parentElement;
  while (node) {
    const overflowY = getComputedStyle(node).overflowY;
    if (overflowY === "auto" || overflowY === "scroll" || overflowY === "hidden")
      return node.getBoundingClientRect().bottom;
    node = node.parentElement;
  }
  return window.innerHeight;
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

function ThreadRunIndicator({ status, unread }: { status?: ThreadRunInfo["status"]; unread?: boolean }) {
  const { t } = useTranslation("layout");
  // Reserved-width placeholder so idle rows (and the hover state) stay aligned.
  const placeholder = <span className="size-5 shrink-0 group-hover/thread:hidden" />;

  if (status === "queued" || status === "running" || status === "waiting_approval") {
    return (
      <span
        aria-label={t("activityRail.running")}
        className="inline-flex size-5 shrink-0 items-center justify-center group-hover/thread:hidden"
        title={t("activityRail.running")}
      >
        <span className="size-3 animate-spin rounded-full border-2 border-accent-soft border-t-accent" />
      </span>
    );
  }

  // A finished run is "unread" until the thread is opened: green when it
  // completed, red when it failed. Once read no dot shows. `cancelled` is a
  // deliberate user action (they aborted the run), so it never needs surfacing
  // as unread — it falls through to the empty placeholder below.
  if (unread && (status === "completed" || status === "failed")) {
    const failed = status === "failed";
    const label = failed ? t("activityRail.failed") : t("activityRail.completed");
    return (
      <span
        aria-label={label}
        className="inline-flex size-5 shrink-0 items-center justify-center group-hover/thread:hidden"
        title={label}
      >
        <span className={cn("size-2 rounded-full", failed ? "bg-danger" : "bg-success")} />
      </span>
    );
  }

  return placeholder;
}

function ThreadItemMenu({
  archived,
  pinned,
  onClose,
  onDelete,
  onRename,
  onRestore,
  onTogglePin,
}: {
  archived?: boolean;
  pinned: boolean;
  onClose: () => void;
  onDelete: () => void;
  onRename: () => void;
  onRestore: () => void;
  onTogglePin: () => void;
}) {
  const { t } = useTranslation("layout");
  const menuRef = useRef<HTMLDivElement>(null);
  // Open downward by default, but flip above the trigger when the menu would
  // spill past its scrolling container (e.g. the last thread near the sidebar
  // bottom) so the full menu — including Delete — stays visible.
  const [dropUp, setDropUp] = useState(false);
  useLayoutEffect(() => {
    const element = menuRef.current;
    if (!element)
      return;
    const rect = element.getBoundingClientRect();
    const boundary = Math.min(clippingBottom(element), window.innerHeight) - 8;
    setDropUp(rect.bottom > boundary);
  }, []);

  return (
    <div
      ref={menuRef}
      className={cn(
        "absolute right-1 z-40 w-36 rounded-lg border border-line-soft bg-surface p-1 shadow-panel",
        dropUp ? "bottom-7" : "top-7",
      )}
    >
      {archived
        ? (
            <ThreadMenuItem icon={<Archive className="size-3.5" />} onClick={onRestore} onClose={onClose}>
              {t("activityRail.restore")}
            </ThreadMenuItem>
          )
        : (
            <>
              <ThreadMenuItem icon={<Pencil className="size-3.5" />} onClick={onRename} onClose={onClose}>
                {t("activityRail.rename")}
              </ThreadMenuItem>
              <ThreadMenuItem icon={<Pin className="size-3.5" />} onClick={onTogglePin} onClose={onClose}>
                {pinned ? t("activityRail.unpin") : t("activityRail.pin")}
              </ThreadMenuItem>
            </>
          )}
      <ThreadMenuItem danger icon={<Trash2 className="size-3.5" />} onClick={onDelete} onClose={onClose}>
        {t("activityRail.delete")}
      </ThreadMenuItem>
    </div>
  );
}

function WorkspaceHeaderMenu({
  workspace,
  onDelete,
  onRename,
}: {
  workspace: StoredWorkspace;
  onDelete: (workspace: StoredWorkspace) => void;
  onRename: (workspace: StoredWorkspace) => void;
}) {
  const { t } = useTranslation("layout");
  const [open, setOpen] = useState(false);
  const layerRef = useDismissableLayer<HTMLDivElement>({ enabled: open, onDismiss: () => setOpen(false) });
  const menuRef = useRef<HTMLDivElement>(null);
  const [dropUp, setDropUp] = useState(false);
  useLayoutEffect(() => {
    if (!open)
      return;
    const element = menuRef.current;
    if (!element)
      return;
    const rect = element.getBoundingClientRect();
    const boundary = Math.min(clippingBottom(element), window.innerHeight) - 8;
    setDropUp(rect.bottom > boundary);
  }, [open]);

  return (
    <div className="relative" ref={layerRef}>
      <button
        aria-label={t("activityRail.workspaceActions", { name: workspace.name })}
        className={cn(
          "inline-flex size-5 shrink-0 items-center justify-center rounded text-ink-muted opacity-0 transition hover:bg-surface hover:text-ink-soft group-hover:opacity-100",
          open && "opacity-100",
        )}
        onClick={(event) => {
          event.stopPropagation();
          setOpen(value => !value);
        }}
        title={t("activityRail.workspaceActions", { name: workspace.name })}
        type="button"
      >
        <MoreHorizontal className="size-3.5" />
      </button>
      {open
        ? (
            <div
              ref={menuRef}
              className={cn(
                "absolute right-0 z-40 w-36 rounded-lg border border-line-soft bg-surface p-1 shadow-panel",
                dropUp ? "bottom-7" : "top-7",
              )}
            >
              <ThreadMenuItem icon={<Pencil className="size-3.5" />} onClick={() => onRename(workspace)} onClose={() => setOpen(false)}>
                {t("activityRail.rename")}
              </ThreadMenuItem>
              <ThreadMenuItem danger icon={<Trash2 className="size-3.5" />} onClick={() => onDelete(workspace)} onClose={() => setOpen(false)}>
                {t("activityRail.delete")}
              </ThreadMenuItem>
            </div>
          )
        : null}
    </div>
  );
}

function ThreadMenuItem({
  children,
  danger,
  icon,
  onClick,
  onClose,
}: {
  children: string;
  danger?: boolean;
  icon: ReactNode;
  onClick: () => void;
  onClose: () => void;
}) {
  return (
    <button
      className={cn(
        "flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-sm font-medium transition-colors",
        danger ? "text-danger hover:bg-danger-soft" : "text-ink-soft hover:bg-surface-subtle hover:text-ink",
      )}
      onClick={() => {
        onClose();
        onClick();
      }}
      type="button"
    >
      {icon}
      <span>{children}</span>
    </button>
  );
}
