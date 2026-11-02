import type { FusionDiscussion, FusionStep } from "@future-os/thread-projection";
import type { ReactNode } from "react";
import type { FusionResolver } from "../../integrations/fusion/queries";
import type { FusionSelection } from "./FusionPanel";
import { FUSION_PHASES, fusionProgress } from "@future-os/thread-projection";
import { ArrowLeft, ChevronDown, RefreshCw, UserRoundGroup } from "lucide-react";
import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { Badge } from "../../components/ui/Badge";
import { IconButton } from "../../components/ui/IconButton";
import { readFusion } from "../../integrations/fusion/queries";
import { cn } from "../../lib/cn";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { usePolling } from "../../lib/usePolling";
import { MarkdownContent } from "../markdown/MarkdownContent";
import { FusionStepBody } from "./FusionStepBody";

export function FusionDiscussionPanel({ runId, toolCallId, status = "running", resolveTarget, onBack }: FusionSelection & {
  resolveTarget?: FusionResolver;
  onBack: () => void;
}) {
  const { t } = useTranslation("agent");
  const scope = `${runId}:${toolCallId}`;
  const load = useCallback(async () => ({ scope, discussion: await readFusion<FusionDiscussion>({ runId, toolCallId }, resolveTarget) }), [runId, toolCallId, scope, resolveTarget]);
  const resource = useAsyncResource(load, [load], null);
  const data = resource.data?.scope === scope ? resource.data.discussion : null;
  const progress = fusionProgress(data, status === "interrupted" ? "failed" : status);
  usePolling(resource.reload, 1500, { enabled: !resource.loading && !progress.terminal, deps: [scope] });
  const [selectedPhase, setSelectedPhase] = useState<string | null>(null);
  const phase = selectedPhase ?? (FUSION_PHASES.includes(progress.phase as typeof FUSION_PHASES[number]) && !progress.terminal ? progress.phase : "analyzing");
  const run = data?.run;
  const steps = [...(data?.steps ?? [])].sort((a, b) => a.started_at - b.started_at || a.attempt - b.attempt);
  const result = ["completed", "delivering"].includes(run?.status ?? "") ? run?.result : null;
  return (
    <div className="flex h-full min-h-0 flex-col" data-fusion-panel="detail">
      <div className="flex shrink-0 items-center gap-2 border-b border-line-soft px-3 py-2">
        <IconButton icon={<ArrowLeft className="size-3.5" />} label={t("fusion.detail.back")} onClick={onBack} />
        <UserRoundGroup className="size-4 shrink-0 text-ink-muted" />
        <span className="min-w-0 flex-1 text-sm font-medium text-ink">{t("fusion.detail.title")}</span>
        <IconButton icon={<RefreshCw className={cn("size-3.5", resource.loading && "animate-spin")} />} label={t("fusion.detail.refresh")} disabled={resource.loading} onClick={resource.reload} />
      </div>
      <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-4 py-4">
        {resource.error && <p role="alert" className="text-xs text-danger">{t("fusion.statusUnavailable")}</p>}
        {!run && <p role="status" className="text-sm text-ink-muted">{t(resource.loading ? "fusion.loading" : "fusion.detail.pending")}</p>}
        {run && (
          <>
            <div className="space-y-3">
              <h2 className="wrap-break-word text-sm font-medium leading-6 text-ink">{run.input.question}</h2>
              <div className="flex flex-wrap gap-2">
                <Badge tone={progress.status === "completed" ? "success" : ["failed", "interrupted"].includes(progress.status) ? "danger" : "neutral"}>{t(`fusion.states.${progress.status}`)}</Badge>
                {result && <Badge>{t(`fusion.completeness.${run.completeness}`)}</Badge>}
              </div>
              {progress.notices.map(notice => <p key={notice} role="status" className="text-xs leading-5 text-ink-muted">{t(`fusion.notices.${notice}`)}</p>)}
              {run.error && <p className="whitespace-pre-wrap wrap-break-word text-xs leading-5 text-danger">{run.error}</p>}
              <details className="rounded-lg border border-line-soft px-3 py-2 text-xs text-ink-soft">
                <summary className="cursor-pointer py-1">{t("fusion.detail.context")}</summary>
                <div className="space-y-3 py-2">
                  <TextSection title={t("fusion.detail.goal")}>{run.input.goal}</TextSection>
                  <TextSection title={t("fusion.detail.background")}>{run.input.background}</TextSection>
                  <ListSection title={t("fusion.detail.constraints")} items={run.input.constraints} />
                </div>
              </details>
            </div>
            {result
              ? (
                  <section className="space-y-4 rounded-lg border border-line-soft bg-surface/50 p-3">
                    <h3 className="text-sm font-medium text-ink">{t("fusion.detail.conclusion")}</h3>
                    <div className="text-13 leading-6 text-ink-soft"><MarkdownContent content={result.conclusion} /></div>
                    <details className="space-y-3 text-xs text-ink-soft">
                      <summary className="cursor-pointer">{t("fusion.detail.resultDetails")}</summary>
                      {(["rationale", "conditions", "unresolved", "next_steps"] as const).map(field => <ListSection key={field} title={t(`fusion.result.${field}`)} items={result[field]} />)}
                    </details>
                  </section>
                )
              : progress.terminal && <p className="rounded-lg border border-line-soft p-3 text-xs leading-5 text-ink-muted">{t("fusion.detail.noConclusion")}</p>}
            {steps.some(step => step.phase === "preflight") && (
              <details className="text-xs text-ink-muted">
                <summary className="cursor-pointer">{t("fusion.phases.preflight")}</summary>
                {steps.filter(step => step.phase === "preflight").map(step => <StepCard key={step.id} step={step} runId={runId} toolCallId={toolCallId} resolveTarget={resolveTarget} terminal={progress.terminal} />)}
              </details>
            )}
            <section className="space-y-3">
              <h3 className="text-sm font-medium text-ink">{t("fusion.discussion")}</h3>
              <div className="grid grid-cols-2 gap-2" role="group" aria-label={t("fusion.progress")}>
                {FUSION_PHASES.map((value, index) => (
                  <button key={value} type="button" aria-pressed={phase === value} onClick={() => setSelectedPhase(value)} className={cn("space-y-2 rounded-md border px-2 py-2 text-left text-xs", phase === value ? "border-info-line bg-accent-soft text-accent" : "border-line-soft text-ink-muted hover:bg-surface-subtle")}>
                    <span className={cn("block h-1 rounded-full", progress.stages[index] === "completed" ? "bg-accent" : progress.stages[index] === "thinking" ? "animate-pulse bg-accent" : progress.stages[index] === "failed" ? "bg-danger" : "bg-line")} />
                    <span className="block">
                      {index + 1}
                      .
                      {" "}
                      {t(`fusion.phases.${value}`)}
                    </span>
                    {progress.stages[index] === "skipped" && <span className="block">{t("fusion.modelStates.skipped")}</span>}
                  </button>
                ))}
              </div>
              {!steps.some(step => step.phase === phase) && <p className="py-3 text-xs text-ink-muted">{t("fusion.detail.noMessages")}</p>}
              {steps.filter(step => step.phase === phase).map(step => <StepCard key={step.id} step={step} runId={runId} toolCallId={toolCallId} resolveTarget={resolveTarget} terminal={progress.terminal} onOpen={() => setSelectedPhase(phase)} />)}
            </section>
          </>
        )}
      </div>
    </div>
  );
}

function StepCard({ step, runId, toolCallId, resolveTarget, terminal, onOpen }: { step: FusionStep; terminal: boolean; onOpen?: () => void } & FusionSelection & { resolveTarget?: FusionResolver }) {
  const { t } = useTranslation("agent");
  const [open, setOpen] = useState(false);
  const failed = ["failed", "error", "interrupted"].includes(step.status);
  const status = step.status === "running" || step.status === "starting" ? terminal ? "interrupted" : "thinking" : step.failure_kind === "timeout" ? "timed_out" : step.failure_kind === "output_limit" ? "output_limited" : step.failure_kind === "budget_reserved" ? "skipped" : step.status;
  return (
    <div className="overflow-hidden rounded-lg border border-line-soft">
      <button
        type="button"
        className="flex w-full items-start gap-2 px-3 py-3 text-left hover:bg-surface-subtle"
        aria-expanded={open}
        onClick={() => {
          if (!open)
            onOpen?.();
          setOpen(value => !value);
        }}
      >
        <span className="min-w-0 flex-1 space-y-1">
          <span className="block text-xs font-medium text-ink">{step.participant === "moderator" ? t("fusion.moderatorRole") : t("fusion.participantRole", { number: step.participant.replace(/^P/u, "") })}</span>
          <span className="block wrap-break-word text-xs text-ink-muted">{step.model}</span>
          {step.attempt > 0 && <span className="block text-xs text-ink-muted">{t("fusion.retry", { count: step.attempt })}</span>}
        </span>
        <span className={cn("text-xs", failed ? "text-danger" : "text-ink-muted")}>{t(`fusion.modelStates.${status}`)}</span>
        <ChevronDown className={cn("size-3.5 shrink-0 text-ink-muted", open && "rotate-180")} />
      </button>
      {open && <FusionStepBody key={`${step.id}:${step.status}`} step={step} runId={runId} toolCallId={toolCallId} resolveTarget={resolveTarget} />}
    </div>
  );
}

function TextSection({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="space-y-1">
      <h4 className="text-xs font-medium text-ink">{title}</h4>
      <div className="whitespace-pre-wrap wrap-break-word text-xs leading-5 text-ink-soft">{children}</div>
    </div>
  );
}

function ListSection({ title, items }: { title: string; items: string[] }) {
  if (items.length === 0)
    return null;
  return <TextSection title={title}><ul className="list-disc space-y-1 pl-4">{[...new Set(items)].map(text => <li key={text} className="wrap-break-word">{text}</li>)}</ul></TextSection>;
}
