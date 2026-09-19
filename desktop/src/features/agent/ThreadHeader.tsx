import type { ReactNode } from "react";
import type { AgentSessionUsage } from "../../integrations/agent/agentStateCache";
import type { StoredThread } from "../../integrations/storage/threadStore";
import { ReceiptText } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { LeftPanelTitlebarToggle } from "../../components/layout/LeftPanelTitlebarToggle";
import { IconButton } from "../../components/ui/IconButton";
import { startWindowDrag } from "../../lib/windowDrag";
import { SessionUsageDialog } from "./SessionUsageDialog";

interface ThreadHeaderProps {
  thread: StoredThread | null;
  leftPanelExpanded: boolean;
  onToggleLeftPanel: () => void;
  /**
   * Session token usage and amount, opened by the header's spend button.
   * Absent for an agent too old to report usage; the dialog then says so
   * rather than showing a fabricated zero.
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
      {/* The conversation's own account book. An icon rather than the amount:
          the header is for the conversation, and a number that changes every
          turn draws the eye away from it. A receipt (the bill for this
          conversation) rather than a coin or a wallet — those read as "money"
          in general, and the wallet already means the account balance in the
          activity rail. Hidden on a thread with no agent session — there is
          nothing to account for. */}
      {thread?.agentSessionId
        ? (
            <IconButton
              aria-haspopup="dialog"
              className="mr-2 size-8"
              data-testid="thread-usage"
              icon={<ReceiptText className="size-4" />}
              label={t("usage.open")}
              onClick={() => setUsageOpen(true)}
            />
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
