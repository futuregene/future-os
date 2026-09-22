import type { AgentStatus } from "../../integrations/agent/agentStatus";
import { Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";

export interface AgentStatusGateProps {
  status: AgentStatus;
  showWait?: boolean;
}

export function AgentStatusGate({ status, showWait = false }: AgentStatusGateProps) {
  const { t } = useTranslation("layout");
  const pending = status.phase === "checking" || status.phase === "starting" || status.phase === "recovering";
  const statusKey = status.phase === "incompatible"
    ? "agentStatus.incompatible"
    : "agentStatus.failed";
  const actionKey = status.phase === "incompatible"
    ? "agentStatus.fixIncompatible"
    : pending
      ? "agentStatus.wait"
      : "agentStatus.fixFailed";

  return (
    <div className="flex h-full items-center justify-center bg-canvas px-6 text-center text-ink">
      <div className="flex max-w-lg flex-col items-center gap-3">
        {pending ? <Loader2 aria-hidden="true" className="size-6 animate-spin text-accent" /> : null}
        {!pending ? <p className="text-lg font-medium">{t(statusKey)}</p> : null}
        {!pending || showWait ? <p className="text-sm text-ink-muted">{t(actionKey)}</p> : null}
      </div>
    </div>
  );
}
