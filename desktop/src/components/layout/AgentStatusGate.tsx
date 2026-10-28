import type { AgentStatus } from "../../integrations/agent/agentStatus";
import { Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { UpdateRecoveryPanel } from "../../features/settings/UpdateRecoveryPanel";

export interface AgentStatusGateProps {
  status: AgentStatus;
  showWait?: boolean;
}

export function AgentStatusGate({ status, showWait = false }: AgentStatusGateProps) {
  const { t } = useTranslation("layout");
  if (status.phase === "stopping") {
    const failed = status.shutdown?.state === "failed";
    const complete = status.shutdown?.state === "stopped";
    if (status.shutdown?.reason === "update") {
      return (
        <div className="flex h-full items-center justify-center bg-canvas px-6 text-center text-ink">
          <div className="max-w-lg"><UpdateRecoveryPanel shutdownFailed={failed} /></div>
        </div>
      );
    }
    return (
      <div className="flex h-full items-center justify-center bg-canvas px-6 text-center text-ink">
        <div aria-live="polite" className="flex max-w-lg flex-col items-center gap-3" role="status">
          {!failed && !complete ? <Loader2 aria-hidden="true" className="size-6 animate-spin text-accent" /> : null}
          <p className="text-lg font-medium">{t(failed ? "agentStatus.cleanupFailed" : complete ? "agentStatus.cleanupComplete" : "agentStatus.stopping")}</p>
          {status.shutdown ? <p className="text-sm text-ink-muted">{t(`agentStatus.shutdownStages.${status.shutdown.stage}`)}</p> : null}
          {failed && status.error ? <p role="alert" className="text-sm text-ink-muted">{status.error}</p> : null}
          {failed ? <p className="text-sm text-ink-muted">{t("agentStatus.cleanupFailedHint")}</p> : null}
        </div>
      </div>
    );
  }
  const pending = status.phase === "checking" || status.phase === "starting" || status.phase === "recovering";
  const noResponse = status.phase === "startup_timeout";
  const statusKey = noResponse
    ? "agentStatus.noResponse"
    : status.phase === "incompatible"
      ? "agentStatus.incompatible"
      : "agentStatus.failed";
  const actionKey = noResponse
    ? "agentStatus.noResponseHint"
    : status.phase === "incompatible"
      ? "agentStatus.fixIncompatible"
      : pending
        ? "agentStatus.wait"
        : "agentStatus.fixFailed";

  const hintKey = pending && status.startup
    ? `agentStatus.startupStages.${status.startup.stage}` as const
    : actionKey;
  return (
    <div className="flex h-full items-center justify-center bg-canvas px-6 text-center text-ink">
      <div className="flex max-w-lg flex-col items-center gap-3">
        {pending ? <Loader2 aria-hidden="true" className="size-6 animate-spin text-accent" /> : null}
        {!pending ? <p className="text-lg font-medium">{t(statusKey)}</p> : null}
        {!pending && status.error ? <p role="alert" className="text-sm text-ink-muted">{status.error}</p> : null}
        {!pending || showWait ? <p className="text-sm text-ink-muted">{t(hintKey)}</p> : null}
        {pending && showWait && (status.startup?.elapsedMs ?? 0) >= 15000 ? <p className="text-sm text-ink-muted">{t("agentStatus.takingLonger")}</p> : null}
      </div>
    </div>
  );
}
