import type { StepSegment } from "./replyBlocks";
import { Brain, ChevronDown, ChevronUp, TriangleAlert, Wrench } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { AgentActivityLine } from "./AgentActivityList";
import { ThinkingBlock } from "./ThinkingBlock";

/** A single quiet summary on the right; revealed steps return to the reading column. */
export function ReplySteps({ segments, showThinking, workspaceId, workspacePath, runId }: {
  segments: StepSegment[];
  showThinking?: boolean;
  workspaceId?: string | null;
  workspacePath?: string | null;
  runId?: string | null;
}) {
  const { t } = useTranslation("agent");
  const [expanded, setExpanded] = useState(false);
  // Mobile counts projected step rows, not the children of an already grouped
  // same-kind burst. That burst retains its own count and detail when opened.
  const tools = segments.filter(segment => segment.kind === "activity").length;
  const thoughts = segments.length - tools;
  const failed = segments.filter(segment => segment.kind === "activity" && segment.item.status === "failed").length;
  const summary = [
    tools ? t("activity.stepTools", { count: tools }) : null,
    thoughts ? t("activity.stepThoughts", { count: thoughts }) : null,
    failed ? t("activity.stepsFailed", { count: failed }) : null,
  ].filter(Boolean).join(" · ");
  const Chevron = expanded ? ChevronUp : ChevronDown;

  return (
    <div className="flex min-w-0 flex-col gap-1 text-[13px] leading-6 text-ink-muted">
      <button
        aria-label={summary}
        aria-expanded={expanded}
        className="flex max-w-full cursor-pointer flex-wrap items-center justify-end gap-1 self-end text-left hover:text-ink"
        onClick={() => setExpanded(value => !value)}
        type="button"
      >
        {tools > 0
          ? (
              <span aria-hidden="true" className="inline-flex items-center">
                <Wrench className="size-3.5" />
                {t("activity.stepCount", { count: tools })}
              </span>
            )
          : null}
        {tools > 0 && thoughts > 0 ? <span aria-hidden="true">·</span> : null}
        {thoughts > 0
          ? (
              <span aria-hidden="true" className="inline-flex items-center">
                <Brain className="size-3.5" />
                {t("activity.stepCount", { count: thoughts })}
              </span>
            )
          : null}
        {failed > 0 ? <TriangleAlert aria-hidden="true" className="size-3.5 shrink-0" /> : null}
        <Chevron aria-hidden="true" className="size-3 shrink-0" />
      </button>
      {expanded
        ? (
            <div className="min-w-0 space-y-1 pl-6">
              {segments.map(segment => segment.kind === "thinking"
                ? <ThinkingBlock key={segment.id} text={segment.text} workspaceId={workspaceId} showContent={showThinking} inSteps />
                : <AgentActivityLine key={segment.id} item={segment.item} workspacePath={workspacePath} runId={runId} inSteps />)}
            </div>
          )
        : null}
    </div>
  );
}
