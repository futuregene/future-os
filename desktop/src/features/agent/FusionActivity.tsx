import type { AgentActivityItem, FusionDiscussion, FusionReadRequest, FusionStep } from "@future-os/thread-projection";
import { fusionIssues, fusionTranscript } from "@future-os/thread-projection";
import { Brain, ChevronDown, ChevronUp } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
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
  const [tab, setTab] = useState("discussion");
  const parentRun = item.runId ?? runId;
  const tool = item.toolCallId ?? item.id;
  const read = useCallback(async (stepId?: string, offset?: number) => {
    if (!parentRun)
      throw new Error(t("fusion.missingParent"));
    const request = { runId: parentRun, toolCallId: tool, stepId, offset, fusion: true };
    if (resolveTarget) {
      const result = await resolveTarget(request);
      if (!result)
        throw new Error(t("fusion.unavailable"));
      return JSON.parse(result) as FusionDiscussion & FusionStep;
    }
    return invokeCommand<FusionDiscussion & FusionStep>("get_fusion", request);
  }, [parentRun, tool, resolveTarget, t]);
  const load = useCallback(() => open ? read() : Promise.resolve(null), [read, open]);
  const { data, error, reload, loading } = useAsyncResource(load, [load], null);
  const active = !data?.run || ["preparing", "queued", "running", "delivering"].includes(data.run.status);
  useEffect(() => {
    if (!open || !active || loading)
      return;
    const timer = window.setTimeout(reload, 1500);
    return () => window.clearTimeout(timer);
  }, [open, active, loading, data, error, reload]);
  const [step, setStep] = useState<FusionStep | null>(null);
  const [transcript, setTranscript] = useState("");
  const [stepError, setStepError] = useState("");
  const stepRequestRef = useRef(0);
  const showStep = async (id: string, offset = 0) => {
    const request = ++stepRequestRef.current;
    try {
      const value = await read(id, offset);
      if (request !== stepRequestRef.current)
        return;
      setStep(value);
      setTranscript(previous => offset ? previous + (value.transcript ?? "") : value.transcript ?? "");
      setStepError("");
    }
    catch (failure) { setStepError(String(failure)); }
  };
  const issues = fusionIssues(data);
  return (
    <div className="my-3 min-w-0 space-y-2 text-13 text-ink-soft">
      <button type="button" className="flex cursor-pointer items-center gap-2 hover:text-ink" onClick={() => setOpen(value => !value)} aria-expanded={open}>
        <Brain className="size-3.5" />
        <span>{t(item.status === "running" ? "fusion.runningTitle" : "fusion.title")}</span>
        {open ? <ChevronUp className="size-3.5" /> : <ChevronDown className="size-3.5" />}
      </button>
      {item.target && <p className="wrap-break-word">{item.target}</p>}
      {open && (
        <div className="space-y-3 border-l border-line pl-3">
          <p>{data?.run ? `${t(`fusion.phases.${data.run.phase}`)} · ${t(`fusion.states.${data.run.status}`)} · ${t(`fusion.completeness.${data.run.completeness}`)}` : t("fusion.loading")}</p>
          {error && <p className="text-danger" role="alert">{String(error)}</p>}
          {data?.run?.error && <p className="text-danger">{data.run.error}</p>}
          <div className="flex flex-wrap gap-3" role="tablist" aria-label="Fusion">
            {["discussion", "issues", "diagnostics"].map(name => <button type="button" key={name} role="tab" aria-selected={tab === name} className="cursor-pointer hover:text-ink" onClick={() => setTab(name)}>{t(`fusion.${name}`)}</button>)}
          </div>
          {tab === "discussion" && data?.steps?.map(value => (
            <div key={value.id} className="space-y-1">
              <button type="button" className="cursor-pointer text-left hover:text-ink" onClick={() => void showStep(value.id)}>
                {t(`fusion.phases.${value.phase}`)}
                {" "}
                ·
                {" "}
                {value.participant}
                {" "}
                ·
                {" "}
                {value.model}
                {" "}
                ·
                {" "}
                {t(`fusion.states.${value.status}`)}
                {value.attempt ? ` · ${t("fusion.retry", { count: value.attempt })}` : ""}
              </button>
              <p className="whitespace-pre-wrap wrap-break-word">{value.error ?? value.output}</p>
            </div>
          ))}
          {tab === "issues" && <pre className="whitespace-pre-wrap wrap-break-word font-sans">{issues.length ? issues.map(issue => `${issue.id} · ${issue.topic}\n${issue.stances.map(stance => `${stance.participant}: ${stance.position}`).join("\n")}`).join("\n\n") : t("fusion.noIssues")}</pre>}
          {tab === "diagnostics" && <pre className="whitespace-pre-wrap wrap-break-word text-xs">{JSON.stringify(data, null, 2)}</pre>}
          {data?.run?.result && (
            <div className="space-y-2 text-ink">
              <p className="whitespace-pre-wrap wrap-break-word">{data.run.result.conclusion}</p>
              {["rationale", "conditions", "unresolved", "next_steps"].map((key) => {
                const items = data.run?.result?.[key as "rationale" | "conditions" | "unresolved" | "next_steps"] ?? [];
                return items.length
                  ? (
                      <div key={key}>
                        <p>{t(`fusion.result.${key}`)}</p>
                        <ul className="list-disc space-y-1 pl-4">{items.map(text => <li key={`${key}-${text}`} className="whitespace-pre-wrap wrap-break-word">{text}</li>)}</ul>
                      </div>
                    )
                  : null;
              })}
            </div>
          )}
          {step && (
            <details open className="space-y-2">
              <summary>
                {step.model}
                {" "}
                ·
                {" "}
                {t("fusion.original")}
              </summary>
              <pre className="whitespace-pre-wrap wrap-break-word text-xs">{fusionTranscript(transcript)}</pre>
              {(step.next_offset ?? 0) < (step.total_bytes ?? 0) && <button type="button" className="cursor-pointer hover:text-ink" onClick={() => void showStep(step.id, step.next_offset)}>{t("fusion.more")}</button>}
              <details>
                <summary>{t("fusion.input")}</summary>
                <pre className="whitespace-pre-wrap wrap-break-word text-xs">{step.input}</pre>
              </details>
            </details>
          )}
          {stepError && <p className="text-danger" role="alert">{stepError}</p>}
        </div>
      )}
    </div>
  );
}
