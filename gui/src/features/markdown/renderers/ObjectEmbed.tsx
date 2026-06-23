import type { ReactNode } from "react";
import type {
  StoredApprovalRequest,
  StoredResearchResource,
  StoredReviewChangeset,
  StoredToolCall,
} from "../../../integrations/storage/types";
import type { FutureReference } from "../futureMarkdownTypes";
import { AlertTriangle, Beaker, FileDiff, Maximize2, Microscope } from "lucide-react";
import { Badge } from "../../../components/ui/Badge";
import { emitFutureEvent } from "../../../lib/futureEvents";

export function ApprovalEmbed({
  approval,
  reference,
}: {
  approval: StoredApprovalRequest;
  reference: FutureReference;
}) {
  return (
    <ObjectFrame
      icon={<AlertTriangle className="mt-0.5 size-4 shrink-0 text-amber-600" />}
      meta={approval.kind}
      status={approval.status}
      title={approval.title || reference.label || approval.id}
    >
      {approval.summary ? <p className="text-sm leading-5 text-ink-soft">{approval.summary}</p> : null}
      {approval.requestedAction
        ? (
            <pre className="mt-2 max-h-32 overflow-auto rounded-md bg-surface-subtle p-2 text-xs leading-5 text-ink-soft">
              <code>{approval.requestedAction}</code>
            </pre>
          )
        : null}
    </ObjectFrame>
  );
}

export function ReviewEmbed({
  reference,
  review,
}: {
  reference: FutureReference;
  review: StoredReviewChangeset;
}) {
  function openReview() {
    emitFutureEvent("open-review", { reviewId: review.id });
  }

  return (
    <ObjectFrame
      icon={<FileDiff className="mt-0.5 size-4 shrink-0 text-accent" />}
      meta={`${review.filesChanged} files, +${review.additions} -${review.deletions}`}
      status={review.status}
      title={review.title || reference.label || review.id}
    >
      {review.summary ? <p className="text-sm leading-5 text-ink-soft">{review.summary}</p> : null}
      <button
        className="mt-2 inline-flex h-7 items-center gap-1.5 rounded-md border border-line bg-surface px-2 text-xs font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
        onClick={openReview}
        type="button"
      >
        <Maximize2 className="size-3.5" />
        Open Review
      </button>
    </ObjectFrame>
  );
}

export function ResearchEmbed({
  reference,
  resource,
}: {
  reference: FutureReference;
  resource: StoredResearchResource;
}) {
  function openResearch() {
    emitFutureEvent("open-research-resource", { resourceId: resource.id });
  }

  return (
    <ObjectFrame
      icon={<Microscope className="mt-0.5 size-4 shrink-0 text-accent" />}
      meta={resource.sourceUri ?? resource.resourceType}
      title={resource.title || reference.label || resource.id}
    >
      {resource.summary ? <p className="text-sm leading-5 text-ink-soft">{resource.summary}</p> : null}
      <button
        className="mt-2 inline-flex h-7 items-center gap-1.5 rounded-md border border-line bg-surface px-2 text-xs font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
        onClick={openResearch}
        type="button"
      >
        <Maximize2 className="size-3.5" />
        Open Research
      </button>
    </ObjectFrame>
  );
}

export function ToolEmbed({
  reference,
  tool,
}: {
  reference: FutureReference;
  tool: StoredToolCall;
}) {
  function inspectRun() {
    emitFutureEvent("inspect-run", { runId: tool.runId });
  }

  const command = toolCommand(tool.input);

  return (
    <ObjectFrame
      icon={<Beaker className="mt-0.5 size-4 shrink-0 text-accent" />}
      meta={tool.kind}
      status={tool.status}
      title={tool.name || reference.label || tool.id}
    >
      {tool.input
        ? (
            <pre className="max-h-32 overflow-auto rounded-md bg-surface-subtle p-2 text-xs leading-5 text-ink-soft">
              <code>{command ?? tool.input}</code>
            </pre>
          )
        : null}
      <button
        className="mt-2 inline-flex h-7 items-center gap-1.5 rounded-md border border-line bg-surface px-2 text-xs font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
        onClick={inspectRun}
        type="button"
      >
        <Maximize2 className="size-3.5" />
        Inspect run
      </button>
    </ObjectFrame>
  );
}

function ObjectFrame({
  children,
  icon,
  meta,
  status,
  title,
}: {
  children?: ReactNode;
  icon: ReactNode;
  meta?: string | null;
  status?: string | null;
  title: string;
}) {
  return (
    <article className="rounded-md border border-line-soft bg-surface p-3">
      <div className="flex items-start gap-2">
        {icon}
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-2">
            <h4 className="truncate text-sm font-semibold text-ink">{title}</h4>
            {status ? <Badge tone={status === "pending" ? "warning" : "neutral"}>{status}</Badge> : null}
          </div>
          {meta ? <div className="mt-1 truncate text-xs text-ink-muted">{meta}</div> : null}
          {children ? <div className="mt-2">{children}</div> : null}
        </div>
      </div>
    </article>
  );
}

function toolCommand(input: string | null | undefined) {
  if (!input)
    return null;

  let current: unknown = input;
  for (let index = 0; index < 3; index += 1) {
    if (isRecord(current)) {
      const value = current.command;
      return typeof value === "string" && value.trim() ? value : null;
    }
    if (typeof current !== "string")
      return null;

    try {
      current = JSON.parse(current) as unknown;
    }
    catch {
      return null;
    }
  }

  return null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
