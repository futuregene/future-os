import type { AgentActivityItem, FusionDiscussion, FusionReadRequest } from "@future-os/thread-projection";
import { FUSION_PHASES, fusionProgress } from "@future-os/thread-projection";
import { Check, ChevronDown, ChevronUp, Circle, Clock, LoaderCircle, TriangleAlert, UserRoundGroup } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invokeCommand } from "../../integrations/tauri/invoke";
import { useAsyncResource } from "../../lib/useAsyncResource";

export function FusionActivity({ item, runId, resolveTarget }: {
  item: AgentActivityItem;
  runId?: string | null;
  resolveTarget?: (input: FusionReadRequest) => Promise<string | null>;
}) {
  const { t } = useTranslation("agent");
  const [open, setOpen] = useState(false);
  const parentRun = item.runId ?? runId;
  const tool = item.toolCallId ?? item.id;
  const load = useCallback(async () => {
    if (!open)
      return null;
    if (!parentRun)
      throw new Error(t("fusion.missingParent"));
    const request = { runId: parentRun, toolCallId: tool, fusion: true };
    if (resolveTarget) {
      const result = await resolveTarget(request);
      if (!result)
        throw new Error(t("fusion.unavailable"));
      return JSON.parse(result) as FusionDiscussion;
    }
    return invokeCommand<FusionDiscussion>("get_fusion", request);
  }, [parentRun, tool, resolveTarget, t, open]);
  const { data, error, reload, loading } = useAsyncResource(load, [load], null);
  const progress = fusionProgress(data, item.status);
  useEffect(() => {
    if (!open || progress.terminal || loading)
      return;
    const timer = window.setTimeout(reload, 1500);
    return () => window.clearTimeout(timer);
  }, [open, progress.terminal, loading, data, error, reload]);
  return (
    <div className="my-3 min-w-0 space-y-2 text-13 text-ink-soft">
      <button type="button" className="flex cursor-pointer items-center gap-2 hover:text-ink" onClick={() => setOpen(value => !value)} aria-expanded={open}>
        <UserRoundGroup className="size-3.5" />
        <span>{t("fusion.title")}</span>
        {open ? <ChevronUp className="size-3.5" /> : <ChevronDown className="size-3.5" />}
      </button>
      {open && (
        <div className="space-y-3 pl-5">
          <div className="space-y-2">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <span>{t(progress.terminal ? `fusion.states.${progress.status}` : `fusion.phases.${progress.phase}`)}</span>
              <span className="text-xs text-ink-muted">{t("fusion.stageCount", { current: progress.status === "completed" ? 4 : Math.max(1, progress.stageIndex + 1), total: 4 })}</span>
            </div>
            <div className="flex gap-1" role="progressbar" aria-label={t("fusion.progress")} aria-valuemin={0} aria-valuemax={4} aria-valuenow={progress.completedStages} aria-valuetext={t(`fusion.phases.${progress.phase}`)}>
              {FUSION_PHASES.map((phase, index) => <span key={phase} title={t(`fusion.phases.${phase}`)} className={`h-1 flex-1 rounded-full ${index < progress.completedStages ? "bg-accent" : index === progress.stageIndex ? progress.terminal ? "bg-danger" : "animate-pulse bg-accent" : "bg-line"}`} />)}
            </div>
            {progress.terminal && progress.partial && <p className="text-xs text-ink-muted">{t("fusion.partialNotice")}</p>}
          </div>
          <div className="space-y-2" aria-live="polite">
            {progress.models.map((model) => {
              const failed = ["failed", "timed_out", "output_limited"].includes(model.status);
              const Icon = model.status === "thinking" ? LoaderCircle : model.status === "completed" ? Check : model.status === "timed_out" ? Clock : failed || model.status === "partial" ? TriangleAlert : Circle;
              return (
                <div key={model.id} className="flex min-w-0 items-center gap-2">
                  <Icon aria-hidden="true" className={`size-3.5 shrink-0 ${model.status === "thinking" ? "animate-spin text-accent" : failed ? "text-danger" : "text-ink-muted"}`} />
                  <span className="min-w-0 flex-1 wrap-break-word">
                    {model.role ? `${t(model.role === "moderator" ? "fusion.moderatorRole" : "fusion.participantRole", { number: model.number })} · ` : ""}
                    {model.label}
                  </span>
                  <span className={`shrink-0 ${failed ? "text-danger" : "text-ink-muted"}`}>{t(`fusion.modelStates.${model.status}`)}</span>
                </div>
              );
            })}
          </div>
          {error && <p className="text-danger" role="alert">{t("fusion.statusUnavailable")}</p>}
        </div>
      )}
    </div>
  );
}
