import type { AgentActivityItem, FusionDiscussion, FusionReadRequest } from "@future-os/thread-projection";
import { FUSION_PHASES, fusionProgress } from "@future-os/thread-projection";
import { Check, ChevronDown, ChevronUp, Circle, CircleMinus, CirclePause, CircleStop, CircleX, Clock, LoaderCircle, TriangleAlert, UserRoundGroup } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invokeCommand } from "../../integrations/tauri/invoke";
import { emitFutureEvent } from "../../lib/futureEvents";
import { useAsyncResource } from "../../lib/useAsyncResource";

const RING_CIRCUMFERENCE = 2 * Math.PI * 20;

export function FusionActivity({ item, runId, resolveTarget }: {
  item: AgentActivityItem;
  runId?: string | null;
  resolveTarget?: (input: FusionReadRequest) => Promise<string | null>;
}) {
  const { t } = useTranslation("agent");
  const [open, setOpen] = useState(false);
  const parentRun = item.runId ?? runId;
  const tool = item.toolCallId ?? item.id;
  const scope = `${parentRun}:${tool}`;
  const load = useCallback(async () => {
    if (!parentRun)
      throw new Error(t("fusion.missingParent"));
    const request = { runId: parentRun, toolCallId: tool, fusion: true };
    if (resolveTarget) {
      const result = await resolveTarget(request);
      if (!result)
        throw new Error(t("fusion.unavailable"));
      return { scope, discussion: JSON.parse(result) as FusionDiscussion };
    }
    return { scope, discussion: await invokeCommand<FusionDiscussion>("get_fusion", request) };
  }, [parentRun, tool, resolveTarget, t, scope]);
  const { data: resource, error, reload, loading } = useAsyncResource(load, [load], null);
  const data = resource?.scope === scope ? resource.discussion : null;
  const progress = fusionProgress(data, item.status);
  const currentStage = progress.status === "completed" ? 4 : Math.max(0, progress.stageIndex + 1);
  const stageLabel = t(progress.terminal ? `fusion.states.${progress.status}` : `fusion.phases.${progress.phase}`);
  const moderatorOnly = progress.phase === "issue_mapping" || progress.phase === "synthesizing";
  const models = moderatorOnly
    ? progress.models.filter(model => model.role === "moderator" || (!model.role && data?.steps?.some(step => step.participant === "moderator" && step.model === model.model)))
    : progress.models;
  useEffect(() => {
    if (!parentRun || progress.terminal || loading || (error && item.status !== "running"))
      return;
    const timer = window.setTimeout(reload, 1500);
    return () => window.clearTimeout(timer);
  }, [parentRun, progress.terminal, loading, data, error, reload, item.status]);
  return (
    <div className="my-3 min-w-0 space-y-2 text-13 text-ink-soft" data-testid="fusion-activity">
      <button
        type="button"
        className="flex cursor-pointer items-center gap-2 hover:text-ink"
        onClick={() => {
          if (!open && error)
            reload();
          setOpen(value => !value);
        }}
        aria-expanded={open}
      >
        <UserRoundGroup className={`size-3.5 ${["failed", "interrupted"].includes(progress.status) ? "text-danger" : ""}`} />
        <span>{t("fusion.title")}</span>
        <span className="relative inline-flex size-3.5 shrink-0 items-center justify-center" role="progressbar" aria-label={t("fusion.progress")} aria-valuemin={0} aria-valuemax={4} aria-valuenow={currentStage} aria-valuetext={stageLabel} title={stageLabel}>
          <svg className="absolute inset-0 size-full" viewBox="0 0 48 48" fill="none" aria-hidden="true">
            {FUSION_PHASES.map((phase, index) => <circle key={phase} cx="24" cy="24" r="20" stroke="currentColor" strokeWidth="6" strokeLinecap="round" strokeDasharray={`${RING_CIRCUMFERENCE / 4 - 9} ${RING_CIRCUMFERENCE * 3 / 4 + 9}`} strokeDashoffset={-(index * RING_CIRCUMFERENCE / 4 + 4.5)} transform="rotate(-90 24 24)" opacity={progress.stages[index] === "waiting" ? 0.18 : 1} className={progress.stages[index] === "failed" ? "text-danger" : progress.stages[index] === "skipped" ? "text-line" : "text-accent"} />)}
          </svg>
        </span>
        {open ? <ChevronUp className="size-3.5" /> : <ChevronDown className="size-3.5" />}
      </button>
      {open && (
        <div className="space-y-3 pl-5">
          {data?.run?.input.question && <p className="whitespace-pre-wrap wrap-break-word leading-5 text-ink-soft" data-testid="fusion-topic">{data.run.input.question}</p>}
          {progress.terminal && progress.partial && progress.notices.length === 0 && <p className="text-xs text-ink-muted">{t("fusion.partialNotice")}</p>}
          {progress.notices.length > 0 && <p className="text-xs text-ink-muted" role="status">{progress.notices.map(notice => t(`fusion.notices.${notice}`)).join(" ")}</p>}
          <div className="space-y-2" aria-live="polite" data-testid="fusion-models">
            {models.map((model) => {
              const role = moderatorOnly ? "moderator" : model.role;
              const name = `${role ? `${t(role === "moderator" ? "fusion.moderatorRole" : "fusion.participantRole", { number: model.number })} · ` : ""}${model.label}`;
              const failed = ["failed", "timed_out", "output_limited"].includes(model.status);
              const Icon = model.status === "thinking" ? LoaderCircle : model.status === "completed" ? Check : model.status === "timed_out" ? Clock : model.status === "failed" ? CircleX : model.status === "interrupted" ? CirclePause : model.status === "cancelled" ? CircleStop : model.status === "skipped" ? CircleMinus : failed || model.status === "partial" ? TriangleAlert : Circle;
              return (
                <div key={model.id} className="flex min-w-0 items-center gap-2">
                  <span className="inline-flex shrink-0" role="img" aria-label={t(`fusion.modelStates.${model.status}`)} title={t(`fusion.modelStates.${model.status}`)}>
                    <Icon aria-hidden="true" className={`size-3.5 ${model.status === "thinking" ? "animate-spin text-accent" : model.status === "completed" ? "text-success" : failed ? "text-danger" : model.status === "partial" || model.status === "interrupted" ? "text-warning" : "text-ink-muted"}`} />
                  </span>
                  <span className="min-w-0 flex-1 wrap-break-word">
                    {name}
                  </span>
                </div>
              );
            })}
          </div>
          {error && <p className="text-danger" role="alert">{t("fusion.statusUnavailable")}</p>}
          {parentRun && <button type="button" className="text-xs text-accent hover:underline" onClick={() => emitFutureEvent("inspect-fusion", { runId: parentRun, toolCallId: tool, status: item.status, resolveTarget })}>{t("fusion.detail.open")}</button>}
        </div>
      )}
    </div>
  );
}
