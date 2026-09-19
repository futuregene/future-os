import type { ReactNode } from "react";
import type { AgentSessionUsage } from "../../integrations/agent/agentStateCache";
import type { StoredThread } from "../../integrations/storage/threadStore";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { LeftPanelTitlebarToggle } from "../../components/layout/LeftPanelTitlebarToggle";
import { formatCostCny } from "../../lib/format";
import { startWindowDrag } from "../../lib/windowDrag";
import { SessionUsageDialog } from "./SessionUsageDialog";

interface ThreadHeaderProps {
  thread: StoredThread | null;
  leftPanelExpanded: boolean;
  onToggleLeftPanel: () => void;
  /**
   * Session token usage and amount. The header always offers the details
   * button; a session without usage yet shows it without a figure.
   */
  usage?: AgentSessionUsage;
  /**
   * Optional trailing affordance owned by the shell (the terminal toggle).
   * A node rather than a callback keeps this header unaware of the feature.
   */
  action?: ReactNode;
}

export function ThreadHeader({
  thread,
  leftPanelExpanded,
  onToggleLeftPanel,
  usage,
  action,
}: ThreadHeaderProps) {
  const { t } = useTranslation("agent");
  const [usageOpen, setUsageOpen] = useState(false);
  return (
    <header
      className="flex h-12 shrink-0 select-none items-center border-b border-line-soft/40 pl-4 pr-14"
      onMouseDown={startWindowDrag}
    >
      <div className="mr-3 flex min-w-0 flex-1 items-center" data-tauri-drag-region>
        <LeftPanelTitlebarToggle
          expanded={leftPanelExpanded}
          onToggle={onToggleLeftPanel}
        />
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-semibold text-ink">{thread?.title ?? t("thread.defaultTitle")}</div>
        </div>
      </div>
      {/* The conversation's own account book: the running amount is visible by
          default, and clicking opens the token/cost breakdown. Hidden on a
          thread with no agent session — there is nothing to account for. */}
      {thread?.agentSessionId
        ? (
            <button
              aria-label={t("usage.open")}
              className="mr-2 inline-flex h-7 shrink-0 items-center gap-1 rounded-md px-2 text-xs font-medium tabular-nums text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
              data-testid="thread-usage"
              onClick={() => setUsageOpen(true)}
              title={t("usage.open")}
              type="button"
            >
              {formatCostCny(usage?.costCny ?? 0)}
            </button>
          )
        : null}
      {action
        ? (
            <div className="flex shrink-0 items-center gap-1">
              {action}
            </div>
          )
        : null}
      <SessionUsageDialog
        onClose={() => setUsageOpen(false)}
        open={usageOpen}
        title={thread?.title ?? t("thread.defaultTitle")}
        usage={usage}
      />
    </header>
  );
}
