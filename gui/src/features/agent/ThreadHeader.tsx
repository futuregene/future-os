import type { AgentConnectionState } from "../../components/layout/AppShell";
import type { StoredThread } from "../../integrations/storage/threadStore";
import { Bell, Command, RefreshCw, Wifi, WifiOff } from "lucide-react";
import { LeftPanelTitlebarToggle } from "../../components/layout/LeftPanelTitlebarToggle";
import { IconButton } from "../../components/ui/IconButton";
import { cn } from "../../lib/cn";
import { startWindowDrag } from "../../lib/windowDrag";

interface ThreadHeaderProps {
  thread: StoredThread | null;
  agentConnection: AgentConnectionState;
  leftPanelExpanded: boolean;
  onRetryAgentConnection: () => void;
  onToggleLeftPanel: () => void;
}

export function ThreadHeader({
  thread,
  agentConnection,
  leftPanelExpanded,
  onRetryAgentConnection,
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
        <AgentConnectionBadge
          connection={agentConnection}
          onRetry={onRetryAgentConnection}
        />
        <IconButton icon={<Command className="size-4" />} label="Command palette" />
        <IconButton icon={<Bell className="size-4" />} label="Notifications" />
      </div>
    </header>
  );
}

function AgentConnectionBadge({
  connection,
  onRetry,
}: {
  connection: AgentConnectionState;
  onRetry: () => void;
}) {
  const connected = connection.status === "connected";
  const checking = connection.status === "checking";
  const label = agentConnectionLabel(connection);
  const title = connected
    ? "Future Agent connected"
    : checking
      ? "Checking Future Agent connection"
      : `Future Agent disconnected${connection.error ? `: ${connection.error}` : ""}`;

  return (
    <div
      className={cn(
        "inline-flex h-8 items-center gap-1.5 rounded-md border px-2 text-xs font-medium",
        connected && "border-success-line bg-success-soft text-success",
        checking && "border-info-line bg-info-soft text-info",
        !connected && !checking && "border-danger-line bg-danger-soft text-danger",
      )}
      title={title}
    >
      {connected
        ? <Wifi className="size-3.5" />
        : <WifiOff className={cn("size-3.5", checking && "animate-pulse")} />}
      <span className="hidden xl:inline">
        {label}
      </span>
      {!connected && !checking
        ? (
            <button
              aria-label="Retry Future Agent connection"
              className="ml-0.5 inline-flex size-5 items-center justify-center rounded text-danger transition-colors hover:bg-surface"
              onClick={onRetry}
              title="Retry connection"
              type="button"
            >
              <RefreshCw className="size-3" />
            </button>
          )
        : null}
    </div>
  );
}

function agentConnectionLabel(connection: AgentConnectionState) {
  if (connection.status === "connected")
    return "Agent";
  if (connection.status === "checking")
    return "Checking";
  if (connection.kind === "agent_unavailable")
    return "Agent not running";
  if (connection.kind === "model_error")
    return "Models unavailable";
  return "Agent offline";
}
