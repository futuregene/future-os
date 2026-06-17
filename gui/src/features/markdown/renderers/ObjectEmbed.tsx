import type { ReactNode } from "react";
import type {
  StoredApprovalRequest,
  StoredResearchResource,
  StoredReviewChangeset,
  StoredToolCall,
} from "../../../integrations/storage/types";
import type { FutureReference } from "../futureMarkdownTypes";
import { AlertTriangle, Beaker, FileDiff, Microscope } from "lucide-react";
import { Badge } from "../../../components/ui/Badge";

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
  return (
    <ObjectFrame
      icon={<FileDiff className="mt-0.5 size-4 shrink-0 text-accent" />}
      meta={`${review.filesChanged} files, +${review.additions} -${review.deletions}`}
      status={review.status}
      title={review.title || reference.label || review.id}
    >
      {review.summary ? <p className="text-sm leading-5 text-ink-soft">{review.summary}</p> : null}
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
  return (
    <ObjectFrame
      icon={<Microscope className="mt-0.5 size-4 shrink-0 text-accent" />}
      meta={resource.sourceUri ?? resource.resourceType}
      title={resource.title || reference.label || resource.id}
    >
      {resource.summary ? <p className="text-sm leading-5 text-ink-soft">{resource.summary}</p> : null}
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
              <code>{tool.input}</code>
            </pre>
          )
        : null}
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
