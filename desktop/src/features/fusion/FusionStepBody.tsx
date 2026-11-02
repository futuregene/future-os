import type { FusionStep } from "@future-os/thread-projection";
import type { FusionResolver } from "../../integrations/fusion/queries";
import type { FusionSelection } from "./types";
import { useCallback, useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { readFusionStep } from "../../integrations/fusion/queries";
import { isRecord } from "../../lib/objects";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { MarkdownContent } from "../markdown/MarkdownContent";

export function FusionStepBody({ step, runId, toolCallId, resolveTarget }: { step: FusionStep } & FusionSelection & { resolveTarget?: FusionResolver }) {
  const { t } = useTranslation("agent");
  const controllerRef = useRef<AbortController | null>(null);
  const scope = `${runId}:${toolCallId}:${step.id}`;
  const load = useCallback(async () => {
    controllerRef.current?.abort();
    const controller = new AbortController();
    controllerRef.current = controller;
    const full = await readFusionStep({ runId, toolCallId, stepId: step.id }, resolveTarget, controller.signal);
    return { scope, resolveTarget, step: full };
  }, [runId, toolCallId, step.id, scope, resolveTarget]);
  useEffect(() => () => controllerRef.current?.abort(), [load]);
  const resource = useAsyncResource(load, [load], null);
  const full = resource.data?.scope === scope && resource.data.resolveTarget === resolveTarget ? resource.data.step : null;
  const text = full ? publicAnswer(full.transcript ?? "") || full.output || step.output : "";
  return (
    <div className="space-y-3 border-t border-line-soft px-3 py-3 text-13 leading-6 text-ink-soft">
      {text ? <div data-fusion-public-output><StepResponse text={text} phase={step.phase} /></div> : full && <p className="text-xs text-ink-muted">{t("fusion.detail.noOutput")}</p>}
      {!full && !resource.error && <p role="status" className="text-xs text-ink-muted">{t("fusion.loading")}</p>}
      {resource.error && (
        <div role="alert" className="space-y-2 text-xs text-danger">
          <p>{t("fusion.detail.readFailed")}</p>
          <Button size="xs" variant="ghost" onClick={resource.reload}>{t("fusion.detail.refresh")}</Button>
        </div>
      )}
    </div>
  );
}

/** Moderator JSON is presented as readable discussion material. */
function StepResponse({ text, phase }: { text: string; phase: string }) {
  const { t } = useTranslation("agent");
  let content = text;
  try {
    const trimmed = text.trim();
    const source = trimmed.startsWith("```") && trimmed.endsWith("```") ? trimmed.slice(trimmed.indexOf("\n") + 1, -3).trim() : trimmed;
    const value: unknown = JSON.parse(source);
    if (isRecord(value)) {
      const list = (field: string, label: string) => {
        const items = Array.isArray(value[field]) ? value[field].filter((item): item is string => typeof item === "string" && !!item.trim()) : [];
        return items.length ? `### ${label}\n${items.map(item => `- ${item}`).join("\n")}` : "";
      };
      if (phase === "synthesizing" && typeof value.conclusion === "string") {
        content = [value.conclusion, ...["rationale", "unresolved"].map(field => list(field, t(`fusion.result.${field}`)))].filter(Boolean).join("\n\n");
      }
      if (phase === "reviewing" && Array.isArray(value.self_review)) {
        const section = (label: string, entries: unknown, render: (entry: Record<string, unknown>) => string) => Array.isArray(entries) && entries.length
          ? `### ${label}\n${entries.filter(isRecord).map(render).filter(Boolean).join("\n\n")}`
          : "";
        const textField = (entry: Record<string, unknown>, field: string) => typeof entry[field] === "string" ? entry[field] as string : "";
        const quoted = (text: string) => text ? `> ${text.replace(/\n/gu, "\n> ")}` : "";
        const decision = (entry: Record<string, unknown>) => t(`fusion.detail.review.decisions.${textField(entry, "decision")}`);
        content = [
          typeof value.current_answer === "string" ? value.current_answer : "",
          section(t("fusion.detail.review.responses"), value.responses, entry => [
            `**${t("fusion.participantRole", { number: textField(entry, "participant").replace(/^P/u, "") })} · ${decision(entry)}**`,
            quoted(textField(entry, "quote")),
            textField(entry, "reason"),
          ].filter(Boolean).join("\n\n")),
          section(t("fusion.detail.review.self"), value.self_review, entry => [
            `**${decision(entry)}**`,
            quoted(textField(entry, "original_quote")),
            textField(entry, "updated_position"),
            textField(entry, "reason"),
          ].filter(Boolean).join("\n\n")),
          section(t("fusion.detail.review.corrections"), value.mapping_corrections, entry => [
            quoted(textField(entry, "summary_quote")),
            textField(entry, "correction"),
            quoted(textField(entry, "source_quote")),
            textField(entry, "reason"),
          ].filter(Boolean).join("\n\n")),
          section(t("fusion.detail.review.additions"), value.additions, entry => [
            `**${t(`fusion.detail.review.kinds.${textField(entry, "kind")}`)}**`,
            textField(entry, "content"),
            Array.isArray(entry.evidence) ? entry.evidence.filter((v): v is string => typeof v === "string").map(v => `- ${v}`).join("\n") : "",
          ].filter(Boolean).join("\n\n")),
        ].filter(Boolean).join("\n\n");
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
    // A truncated structured preview is not a readable report.
    if (["issue_mapping", "reviewing"].includes(phase) && (text.trimStart().startsWith("{") || text.trimStart().startsWith("```")))
      return <p className="text-xs text-ink-muted">{t("fusion.detail.noOutput")}</p>;
  }
  return <MarkdownContent content={content} />;
}

/** Render only public assistant text from the stored transcript. */
function publicAnswer(raw: string): string {
  try {
    const messages: unknown = JSON.parse(raw);
    if (!Array.isArray(messages))
      return "";
    const replies = messages.filter(isRecord).filter(message => message.role === "assistant");
    const last = replies[replies.length - 1];
    return (Array.isArray(last?.content) ? last.content.filter(isRecord) : []).filter(block => block.type === "text" && typeof block.text === "string").map(block => block.text).join("\n\n");
  }
  catch {
    return "";
  }
}
