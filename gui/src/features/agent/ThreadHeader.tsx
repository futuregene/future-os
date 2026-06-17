import type { StoredThread } from "../../integrations/storage/threadStore";
import { Bell, Command } from "lucide-react";
import { LeftPanelTitlebarToggle } from "../../components/layout/LeftPanelTitlebarToggle";
import { IconButton } from "../../components/ui/IconButton";
import { startWindowDrag } from "../../lib/windowDrag";

interface ThreadHeaderProps {
  thread: StoredThread | null;
  leftPanelExpanded: boolean;
  onToggleLeftPanel: () => void;
}

export function ThreadHeader({
  thread,
  leftPanelExpanded,
  onToggleLeftPanel,
}: ThreadHeaderProps) {
  return (
    <header
      className="flex h-12 shrink-0 select-none items-center justify-between border-b border-line-soft pl-4 pr-14"
      onMouseDown={startWindowDrag}
    >
      <div className="mr-3 flex min-w-0 flex-1 items-center" data-tauri-drag-region>
        <LeftPanelTitlebarToggle
          expanded={leftPanelExpanded}
          onToggle={onToggleLeftPanel}
        />
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-semibold text-ink">{thread?.title ?? "FutureOS"}</div>
        </div>
      </div>
      <div className="flex shrink-0 items-center gap-2">
        <IconButton icon={<Command className="size-4" />} label="Command palette" />
        <IconButton icon={<Bell className="size-4" />} label="Notifications" />
      </div>
    </header>
  );
}
