import type { StoredRun } from "../../../integrations/storage/types";
import type { FutureReference } from "../futureMarkdownTypes";
import { Maximize2, PlayCircle } from "lucide-react";
import { formatRunStatus, runTone, shortId } from "../../../components/layout/context-panel/contextPanelFormatters";
import { Badge } from "../../../components/ui/Badge";
import { storedTimeToIso } from "../../../integrations/storage/threadStore";
import { formatTime } from "../../../lib/date";

export function RunEmbed({
  reference,
  run,
}: {
  reference: FutureReference;
  run: StoredRun;
}) {
  function inspectRun() {
    window.dispatchEvent(new CustomEvent("futureos:inspect-run", {
      detail: { runId: run.id },
    }));
  }

  return (
    <article className="rounded-md border border-line-soft bg-surface p-3">
      <div className="flex items-start gap-2">
        <PlayCircle className="mt-0.5 size-4 shrink-0 text-accent" />
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-2">
            <h4 className="truncate text-sm font-semibold text-ink">{reference.label ?? shortId(run.id)}</h4>
            <Badge tone={runTone(run.status)}>{formatRunStatus(run.status)}</Badge>
          </div>
          <div className="mt-1 text-xs text-ink-muted">
            {run.startedAt ? formatTime(storedTimeToIso(run.startedAt)) : formatTime(storedTimeToIso(run.createdAt))}
          </div>
          <div className="mt-2 grid grid-cols-2 gap-2 text-xs text-ink-soft">
            <div>
              <span className="text-ink-muted">Model</span>
              <div className="truncate">{run.modelId ?? "-"}</div>
            </div>
            <div>
              <span className="text-ink-muted">Run</span>
              <div className="truncate">{shortId(run.id)}</div>
            </div>
          </div>
          {run.errorMessage
            ? <p className="mt-2 rounded-md bg-red-50 p-2 text-xs leading-5 text-red-700">{run.errorMessage}</p>
            : null}
          <button
            className="mt-3 inline-flex h-7 items-center gap-1.5 rounded-md border border-line bg-surface px-2 text-xs font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
            onClick={inspectRun}
            type="button"
          >
            <Maximize2 className="size-3.5" />
            Inspect
          </button>
        </div>
      </div>
    </article>
  );
}
