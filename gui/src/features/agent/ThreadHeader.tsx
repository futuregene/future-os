import type { ReactNode } from "react";
import type { StoredThread } from "../../integrations/storage/threadStore";
import { Archive, Bell, Command, MoreHorizontal, Pencil, Pin, Trash2 } from "lucide-react";
import { LeftPanelTitlebarToggle } from "../../components/layout/LeftPanelTitlebarToggle";
import { IconButton } from "../../components/ui/IconButton";
import { useDismissableLayer } from "../../lib/useDismissableLayer";
import { startWindowDrag } from "../../lib/windowDrag";

interface ThreadHeaderProps {
  thread: StoredThread | null;
  leftPanelExpanded: boolean;
  menuOpen: boolean;
  onArchiveThread: () => void;
  onDeleteThread: () => void;
  onMenuOpenChange: (open: boolean) => void;
  onRenameThread: () => void;
  onToggleLeftPanel: () => void;
  onTogglePinThread: () => void;
}

export function ThreadHeader({
  thread,
  leftPanelExpanded,
  menuOpen,
  onArchiveThread,
  onDeleteThread,
  onMenuOpenChange,
  onRenameThread,
  onToggleLeftPanel,
  onTogglePinThread,
}: ThreadHeaderProps) {
  const menuRef = useDismissableLayer<HTMLDivElement>({
    enabled: menuOpen,
    onDismiss: () => onMenuOpenChange(false),
  });

  return (
    <header
      className="flex h-12 shrink-0 select-none items-center justify-between border-b border-line-soft px-4"
      onMouseDown={startWindowDrag}
    >
      <div className="flex min-w-0 flex-1 items-center" data-tauri-drag-region>
        <LeftPanelTitlebarToggle
          expanded={leftPanelExpanded}
          onToggle={onToggleLeftPanel}
        />
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-semibold text-ink">{thread?.title ?? "FutureOS"}</div>
          <div className="truncate text-xs text-ink-muted">
            {thread?.mode === "workspace" ? "Workspace Copilot" : "Research Copilot"}
          </div>
        </div>
      </div>
      <div className="flex items-center gap-2">
        <IconButton icon={<Command className="size-4" />} label="Command palette" />
        <IconButton icon={<Bell className="size-4" />} label="Notifications" />
        <div className="relative" ref={menuRef}>
          <IconButton
            icon={<MoreHorizontal className="size-4" />}
            label="Thread actions"
            onClick={() => onMenuOpenChange(!menuOpen)}
          />
          {menuOpen
            ? (
                <ThreadActionMenu
                  pinned={Boolean(thread?.pinned)}
                  disabled={!thread}
                  onArchiveThread={onArchiveThread}
                  onDeleteThread={onDeleteThread}
                  onClose={() => onMenuOpenChange(false)}
                  onRenameThread={onRenameThread}
                  onTogglePinThread={onTogglePinThread}
                />
              )
            : null}
        </div>
      </div>
    </header>
  );
}

function ThreadActionMenu({
  disabled,
  pinned,
  onArchiveThread,
  onClose,
  onDeleteThread,
  onRenameThread,
  onTogglePinThread,
}: {
  disabled: boolean;
  pinned: boolean;
  onArchiveThread: () => void;
  onClose: () => void;
  onDeleteThread: () => void;
  onRenameThread: () => void;
  onTogglePinThread: () => void;
}) {
  return (
    <div className="absolute right-0 top-10 z-30 w-44 rounded-lg border border-line-soft bg-white p-1 shadow-panel">
      <ThreadAction icon={<Pencil className="size-3.5" />} disabled={disabled} onClick={onRenameThread} onClose={onClose}>
        Rename
      </ThreadAction>
      <ThreadAction icon={<Pin className="size-3.5" />} disabled={disabled} onClick={onTogglePinThread} onClose={onClose}>
        {pinned ? "Unpin" : "Pin"}
      </ThreadAction>
      <ThreadAction
        danger
        icon={<Archive className="size-3.5" />}
        disabled={disabled}
        onClick={onArchiveThread}
        onClose={onClose}
      >
        Archive
      </ThreadAction>
      <ThreadAction
        danger
        icon={<Trash2 className="size-3.5" />}
        disabled={disabled}
        onClick={onDeleteThread}
        onClose={onClose}
      >
        Delete
      </ThreadAction>
    </div>
  );
}

function ThreadAction({
  children,
  danger,
  disabled,
  icon,
  onClick,
  onClose,
}: {
  children: string;
  danger?: boolean;
  disabled: boolean;
  icon: ReactNode;
  onClick: () => void;
  onClose: () => void;
}) {
  return (
    <button
      className={
        danger
          ? "flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-sm font-medium text-red-600 transition-colors hover:bg-red-50"
          : "flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-sm font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
      }
      disabled={disabled}
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
