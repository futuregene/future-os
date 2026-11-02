import type { FusionStep } from "@future-os/thread-projection";
import type { FusionResolver } from "../../integrations/fusion/queries";
import type { FusionSelection } from "./types";
import { fusionTranscript } from "@future-os/thread-projection";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { CopyablePre } from "../../components/ui/CopyablePre";
import { readFusion } from "../../integrations/fusion/queries";
import { isRecord } from "../../lib/objects";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { MarkdownContent } from "../markdown/MarkdownContent";

export function FusionStepBody({ step, runId, toolCallId, resolveTarget }: { step: FusionStep } & FusionSelection & { resolveTarget?: FusionResolver }) {
  const { t } = useTranslation("agent");
  const [offset, setOffset] = useState(0);
  const [pages, setPages] = useState<{ offset: number; step: FusionStep }[]>([]);
  const load = useCallback(async () => ({ offset, step: await readFusion<FusionStep>({ runId, toolCallId, stepId: step.id, offset }, resolveTarget) }), [runId, toolCallId, step.id, offset, resolveTarget]);
  const resource = useAsyncResource(load, [load], null);
  useEffect(() => {
    const page = resource.data;
    if (!page || page.offset !== offset)
      return;
    setPages(previous => page.offset === 0 ? [page] : previous.some(entry => entry.offset === page.offset) ? previous : [...previous, page]);
  }, [resource.data, offset]);
  const lastPage = pages[pages.length - 1];
  const last = lastPage?.step;
  const raw = pages.map(page => page.step.transcript ?? "").join("");
  const more = !!last && (last.next_offset ?? 0) > lastPage.offset && (last.next_offset ?? 0) < (last.total_bytes ?? 0);
  const text = !more ? publicAnswer(raw) || step.output : step.output;
  return (
    <div className="space-y-3 border-t border-line-soft px-3 py-3 text-13 leading-6 text-ink-soft">
      {step.error && <p className="whitespace-pre-wrap wrap-break-word text-xs text-danger">{step.error}</p>}
      {text ? <div data-fusion-public-output><StepResponse text={text} phase={step.phase} /></div> : <p className="text-xs text-ink-muted">{t("fusion.detail.noOutput")}</p>}
      {resource.loading && <p role="status" className="text-xs text-ink-muted">{t("fusion.loading")}</p>}
      {resource.error && (
        <div role="alert" className="space-y-2 text-xs text-danger">
          <p>{t("fusion.detail.readFailed")}</p>
          <Button size="xs" variant="ghost" onClick={resource.reload}>{t("fusion.detail.refresh")}</Button>
        </div>
      )}
      {more && <Button size="xs" disabled={resource.loading} onClick={() => setOffset(last.next_offset!)}>{t("fusion.more")}</Button>}
      {raw && !more && (
        <details className="text-xs text-ink-muted">
          <summary className="cursor-pointer py-1">{t("fusion.original")}</summary>
          <CopyablePre text={fusionTranscript(raw)} maxHeightClassName="max-h-80" />
        </details>
      )}
    </div>
  );
}

/** Moderator JSON is presented as readable discussion material, with raw records below. */
function StepResponse({ text, phase }: { text: string; phase: string }) {
  const { t } = useTranslation("agent");
  let content = text;
  try {
    const trimmed = text.trim();
    const source = trimmed.startsWith("```") && trimmed.endsWith("```") ? trimmed.slice(trimmed.indexOf("\n") + 1, -3).trim() : trimmed;
    const value: unknown = JSON.parse(source);
    if (isRecord(value)) {
      const list = (field: string, label: string) => Array.isArray(value[field])
        ? `### ${label}\n${value[field].filter((item): item is string => typeof item === "string").map(item => `- ${item}`).join("\n")}`
        : "";
      if (phase === "synthesizing" && typeof value.conclusion === "string") {
        content = [value.conclusion, ...["rationale", "conditions", "unresolved", "next_steps"].map(field => list(field, t(`fusion.result.${field}`)))].filter(Boolean).join("\n\n");
      }
      if (phase === "issue_mapping" && Array.isArray(value.issues)) {
        const role = (participant: string) => t("fusion.participantRole", { number: participant.replace(/^P/u, "") });
        const source = (item: Record<string, unknown>) => typeof item.participant === "string" && typeof item.quote === "string" && item.quote
          ? `> ${role(item.participant)}: ${item.quote.replace(/\n/gu, "\n> ")}`
          : "";
        const consensus = Array.isArray(value.consensus)
          ? value.consensus.map((item) => {
              if (typeof item === "string")
                return `- ${item}`;
              if (!isRecord(item) || typeof item.point !== "string")
                return "";
              return [`- ${item.point}`, Array.isArray(item.sources) ? item.sources.filter(isRecord).map(source).join("\n\n") : ""].filter(Boolean).join("\n\n");
            }).filter(Boolean).join("\n\n")
          : "";
        const issues = value.issues.filter(isRecord).map(issue => [
          typeof issue.topic === "string" ? `#### ${issue.topic}` : "",
          Array.isArray(issue.stances)
            ? issue.stances.filter(isRecord).map((stance) => {
                if (typeof stance.participant !== "string")
                  return "";
                if (stance.status === "not_mentioned")
                  return `- ${role(stance.participant)}: ${t("fusion.detail.notMentioned")}`;
                return typeof stance.position === "string" ? [`- ${role(stance.participant)}: ${stance.position}`, source(stance)].filter(Boolean).join("\n\n") : "";
              }).filter(Boolean).join("\n\n")
            : "",
        ].filter(Boolean).join("\n\n")).join("\n\n");
        const excluded = Array.isArray(value.excluded)
          ? value.excluded.filter(isRecord).map(item => [
              source(item),
              typeof item.reason === "string" ? item.reason : "",
              typeof item.basis === "string" ? `${t("fusion.detail.boundary")}: ${item.basis}` : "",
            ].filter(Boolean).join("\n\n")).join("\n\n")
          : "";
        content = [consensus ? `### ${t("fusion.detail.consensus")}\n${consensus}` : "", issues ? `### ${t("fusion.detail.disagreements")}\n${issues}` : "", excluded ? `### ${t("fusion.detail.excluded")}\n${excluded}` : ""].filter(Boolean).join("\n\n") || t("fusion.detail.noMappedViews");
      }
    }
  }
  catch {
    // Failed/partial structured output remains visible as its original public text.
  }
  return <MarkdownContent content={content} />;
}

/** Render public assistant text; tool records stay in the optional transcript. */
function publicAnswer(raw: string): string {
  try {
    const messages: unknown = JSON.parse(raw);
    if (!Array.isArray(messages))
      return "";
    return messages.filter(isRecord).filter(message => message.role === "assistant").flatMap(message => Array.isArray(message.content) ? message.content.filter(isRecord) : []).filter(block => block.type === "text" && typeof block.text === "string").map(block => block.text).join("\n\n");
  }
  catch {
    return "";
  }
}
