import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import type { StoredRun, StoredThread, StoredWorkspace } from "../../integrations/storage/threadStore";
import {
  Archive,
  Database,
  Folder,

  MessageSquare,
  Microscope,
  MoreHorizontal,
  PanelLeftClose,
  PanelLeftOpen,
  Pencil,
  Pin,
  Plus,
  Settings,
  Sparkles,
  SquarePen,
  Trash2,
} from "lucide-react";
import { useState } from "react";
import { cn } from "../../lib/cn";
import { useDismissableLayer } from "../../lib/useDismissableLayer";
import { startWindowDrag } from "../../lib/windowDrag";
import { IconButton } from "../ui/IconButton";

export type ActivitySection = "chat" | "workspace" | "research" | "data" | "skill" | "settings";

interface ActivityRailProps {
  active: ActivitySection;
  expanded: boolean;
  floating?: boolean;
  activeThreadId: string | null;
  threads: StoredThread[];
  threadRunStatuses: Record<string, StoredRun["status"] | undefined>;
  workspaces: StoredWorkspace[];
  onChange: (section: ActivitySection) => void;
  onDeleteThread: (thread: StoredThread) => void;
  onNewChat: (workspaceId?: string) => void;
  onRenameThread: (thread: StoredThread) => void;
  onRestoreThread: (thread: StoredThread) => void;
  onSelectWorkspace: (workspace: StoredWorkspace, threads: StoredThread[]) => void;
  onSelectThread: (thread: StoredThread) => void;
  onTogglePinThread: (thread: StoredThread) => void;
  onToggleExpanded: () => void;
}

const featureItems = [
  { id: "research", label: "Research", icon: Microscope },
  { id: "data", label: "Data", icon: Database },
  { id: "skill", label: "Skill", icon: Sparkles },
] satisfies Array<{ id: ActivitySection; label: string; icon: LucideIcon }>;

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
  workspaces,
  onChange,
  onDeleteThread,
  onNewChat,
  onRenameThread,
  onRestoreThread,
  onSelectWorkspace,
  onSelectThread,
  onTogglePinThread,
  onToggleExpanded,
}: ActivityRailProps) {
  const [showArchived, setShowArchived] = useState(false);
  const [openThreadMenuId, setOpenThreadMenuId] = useState<string | null>(null);
  const visibleThreads = sortThreads(
    threads.filter(thread => thread.status === "active" || (showArchived && thread.status === "archived")),
  );
  const chatThreads = visibleThreads.filter(thread => thread.mode === "chat");
  const workspaceThreads = visibleThreads.filter(thread => thread.mode === "workspace");
  const workspaceGroups = workspaces
    .filter(workspace => workspace.kind === "user" || workspaceThreads.some(thread => thread.workspaceId === workspace.id))
    .map(workspace => ({
      workspace,
      threads: workspaceThreads.filter(thread => thread.workspaceId === workspace.id),
    }));
  const toggleLabel = floating ? "Pin sidebar" : expanded ? "Collapse sidebar" : "Expand sidebar";

  return (
    <nav
      className={cn(
        "flex h-full flex-col bg-surface-subtle transition-[width] duration-200",
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
            expanded && "absolute left-[80px] top-1.5",
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
      <div className={cn("flex flex-1 flex-col p-2", expanded ? "w-full" : "items-center gap-2")}>
        {expanded
          ? (
              <>
                <button
                  className="flex h-8 w-full items-center gap-2 rounded-md border border-transparent px-2 text-sm font-medium text-ink transition-colors hover:bg-surface"
                  onClick={() => onNewChat()}
                  type="button"
                >
                  <SquarePen className="size-4 shrink-0 text-ink-soft" />
                  <span className="truncate">New Chat</span>
                </button>
                <div className="mb-3 space-y-0.5">
                  {featureItems.map((item) => {
                    const Icon = item.icon;
                    return (
                      <button
                        key={item.id}
                        className={cn(
                          "flex h-8 w-full items-center gap-2 rounded-md border border-transparent px-2 text-sm font-medium text-ink-soft transition-colors hover:bg-surface hover:text-ink",
                          active === item.id && "bg-surface text-ink",
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
                <div className="space-y-0.5">
                  <div className="flex h-6 items-center justify-between px-2 text-xs font-medium text-ink-muted">
                    <span>Workspace</span>
                    <div className="flex items-center gap-1">
                      <button
                        aria-label={showArchived ? "Hide archived" : "Show archived"}
                        className={cn(
                          "inline-flex size-5 items-center justify-center rounded transition-colors hover:bg-surface hover:text-ink-soft",
                          showArchived ? "text-ink-soft" : "text-ink-muted",
                        )}
                        onClick={() => setShowArchived(value => !value)}
                        title={showArchived ? "Hide archived" : "Show archived"}
                        type="button"
                      >
                        <Archive className="size-3.5" />
                      </button>
                      <button
                        aria-label="New workspace chat"
                        className="inline-flex size-5 items-center justify-center rounded text-ink-muted transition-colors hover:bg-surface hover:text-ink-soft"
                        onClick={() => onNewChat()}
                        title="New workspace chat"
                        type="button"
                      >
                        <Plus className="size-3.5" />
                      </button>
                    </div>
                  </div>
                  {workspaceGroups.length === 0
                    ? (
                        <div className="px-2 py-1 text-xs text-ink-muted">No workspace threads</div>
                      )
                    : null}
                  {workspaceGroups.map(({ workspace, threads: groupThreads }) => (
                    <div key={workspace.id} className="space-y-0.5">
                      <button
                        className={cn(
                          "group flex h-7 w-full items-center gap-2 rounded-md px-2 text-left transition-colors hover:bg-surface",
                          active === "workspace"
                          && groupThreads.some(thread => thread.id === activeThreadId)
                          && "bg-surface text-ink",
                        )}
                        onClick={() => onSelectWorkspace(workspace, groupThreads)}
                        type="button"
                      >
                        <Folder className="size-4 shrink-0 text-ink-soft" />
                        <span className="min-w-0 flex-1 truncate text-sm font-medium text-ink-soft">
                          {workspace.name}
                        </span>
                        {showArchived && groupThreads.some(thread => thread.status === "archived")
                          ? <span className="text-[11px] font-medium text-ink-muted">Archived</span>
                          : null}
                        <span
                          aria-label={`New chat in ${workspace.name}`}
                          className="inline-flex size-5 shrink-0 items-center justify-center rounded text-ink-muted opacity-0 transition hover:bg-surface hover:text-ink-soft group-hover:opacity-100"
                          onClick={(event) => {
                            event.stopPropagation();
                            onNewChat(workspace.id);
                          }}
                          role="button"
                          title={`New chat in ${workspace.name}`}
                        >
                          <Plus className="size-3.5" />
                        </span>
                      </button>
                      {groupThreads.length > 0
                        ? (
                            <div className="ml-6 space-y-0.5 border-l border-line-soft/60 pl-2">
                              {groupThreads.map(thread => (
                                <ThreadListItem
                                  active={thread.id === activeThreadId}
                                  archived={thread.status === "archived"}
                                  key={thread.id}
                                  menuOpen={openThreadMenuId === thread.id}
                                  runStatus={threadRunStatuses[thread.id]}
                                  thread={thread}
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
                  ))}
                </div>
                <div className="mt-3 space-y-0.5">
                  <div className="flex h-6 items-center justify-between px-2 text-xs font-medium text-ink-muted">
                    <span>Chat</span>
                    <button
                      aria-label={showArchived ? "Hide archived" : "Show archived"}
                      className={cn(
                        "inline-flex size-5 items-center justify-center rounded transition-colors hover:bg-surface hover:text-ink-soft",
                        showArchived ? "text-ink-soft" : "text-ink-muted",
                      )}
                      onClick={() => setShowArchived(value => !value)}
                      title={showArchived ? "Hide archived" : "Show archived"}
                      type="button"
                    >
                      <Archive className="size-3.5" />
                    </button>
                  </div>
                  {chatThreads.length === 0 ? <div className="px-2 py-1 text-xs text-ink-muted">No chats</div> : null}
                  {chatThreads.map(thread => (
                    <ThreadListItem
                      active={thread.id === activeThreadId && active === "chat"}
                      archived={thread.status === "archived"}
                      key={thread.id}
                      menuOpen={openThreadMenuId === thread.id}
                      runStatus={threadRunStatuses[thread.id]}
                      thread={thread}
                      onDeleteThread={onDeleteThread}
                      onMenuOpenChange={open => setOpenThreadMenuId(open ? thread.id : null)}
                      onRenameThread={onRenameThread}
                      onRestoreThread={onRestoreThread}
                      onSelectThread={onSelectThread}
                      onTogglePinThread={onTogglePinThread}
                    />
                  ))}
                </div>
              </>
            )
          : (
              <>
                <IconButton
                  icon={<SquarePen className="size-4" />}
                  label="New chat"
                  active={false}
                  onClick={() => onNewChat()}
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
                  label="Workspace"
                  active={active === "workspace"}
                  onClick={() => onChange("workspace")}
                />
                <IconButton
                  icon={<MessageSquare className="size-4" />}
                  label="Chat"
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
                <span className="truncate">{settingsItem.label}</span>
              </button>
            )
          : (
              <IconButton
                icon={<Settings className="size-4" />}
                label={settingsItem.label}
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
  runStatus?: StoredRun["status"];
  thread: StoredThread;
  onDeleteThread: (thread: StoredThread) => void;
  onMenuOpenChange: (open: boolean) => void;
  onRenameThread: (thread: StoredThread) => void;
  onRestoreThread: (thread: StoredThread) => void;
  onSelectThread: (thread: StoredThread) => void;
  onTogglePinThread: (thread: StoredThread) => void;
}) {
  const menuRef = useDismissableLayer<HTMLDivElement>({
    enabled: menuOpen,
    onDismiss: () => onMenuOpenChange(false),
  });

  return (
    <div
      ref={menuRef}
      className={cn(
        "group/thread relative flex w-full items-center gap-1 rounded-md px-2 text-left transition-colors hover:bg-surface",
        compact ? "h-7" : "h-8 gap-2",
        active && "bg-surface text-ink shadow-sm",
      )}
    >
      {!compact ? <MessageSquare className="size-4 shrink-0 text-ink-soft" /> : null}
      <button
        className={cn(
          "min-w-0 flex-1 truncate text-left text-sm font-medium",
          archived ? "text-ink-muted" : "text-ink-soft",
        )}
        onClick={() => onSelectThread(thread)}
        type="button"
      >
        {thread.title}
      </button>
      {archived ? <span className="shrink-0 text-[11px] text-ink-muted group-hover/thread:hidden">Archived</span> : null}
      <ThreadRunIndicator status={runStatus} />
      <button
        aria-label={`Thread actions for ${thread.title}`}
        className={cn(
          "hidden size-5 shrink-0 items-center justify-center rounded text-ink-muted transition-colors hover:bg-surface hover:text-ink-soft group-hover/thread:inline-flex",
          menuOpen && "inline-flex",
        )}
        onClick={(event) => {
          event.stopPropagation();
          onMenuOpenChange(!menuOpen);
        }}
        title={`Thread actions for ${thread.title}`}
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

function ThreadRunIndicator({ status }: { status?: StoredRun["status"] }) {
  if (!status)
    return <span className="size-5 shrink-0 group-hover/thread:hidden" />;

  if (status === "queued" || status === "running" || status === "waiting_approval") {
    return (
      <span
        aria-label="Running"
        className="inline-flex size-5 shrink-0 items-center justify-center group-hover/thread:hidden"
        title="Running"
      >
        <span className="size-3 animate-spin rounded-full border-2 border-blue-200 border-t-blue-600" />
      </span>
    );
  }

  if (status === "completed" || status === "failed" || status === "cancelled") {
    const label = status === "completed" ? "Completed" : status === "failed" ? "Failed" : "Cancelled";
    return (
      <span
        aria-label={label}
        className="inline-flex size-5 shrink-0 items-center justify-center group-hover/thread:hidden"
        title={label}
      >
        <span className="size-2 rounded-full bg-ink-muted/70" />
      </span>
    );
  }

  return <span className="size-5 shrink-0 group-hover/thread:hidden" />;
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
  return (
    <div className="absolute right-1 top-7 z-40 w-36 rounded-lg border border-line-soft bg-surface p-1 shadow-panel">
      {archived
        ? (
            <ThreadMenuItem icon={<Archive className="size-3.5" />} onClick={onRestore} onClose={onClose}>
              Restore
            </ThreadMenuItem>
          )
        : (
            <>
              <ThreadMenuItem icon={<Pencil className="size-3.5" />} onClick={onRename} onClose={onClose}>
                Rename
              </ThreadMenuItem>
              <ThreadMenuItem icon={<Pin className="size-3.5" />} onClick={onTogglePin} onClose={onClose}>
                {pinned ? "Unpin" : "Pin"}
              </ThreadMenuItem>
            </>
          )}
      <ThreadMenuItem danger icon={<Trash2 className="size-3.5" />} onClick={onDelete} onClose={onClose}>
        Delete
      </ThreadMenuItem>
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
        danger ? "text-red-600 hover:bg-red-50" : "text-ink-soft hover:bg-surface-subtle hover:text-ink",
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
