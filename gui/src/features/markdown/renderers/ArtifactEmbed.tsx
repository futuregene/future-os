import type { StoredArtifact } from "../../../integrations/storage/types";
import type { FutureReference } from "../futureMarkdownTypes";
import { BookMarked, FileText } from "lucide-react";
import { storedTimeToIso } from "../../../integrations/storage/threadStore";
import { formatTime } from "../../../lib/date";

export function ArtifactEmbed({
  artifact,
  reference,
}: {
  artifact: StoredArtifact;
  reference: FutureReference;
}) {
  return (
    <article className="rounded-md border border-line-soft bg-surface p-3">
      <div className="flex items-start gap-2">
        <FileText className="mt-0.5 size-4 shrink-0 text-accent" />
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-2">
            <h4 className="truncate text-sm font-semibold text-ink">{artifact.title || reference.label || artifact.id}</h4>
            <span className="shrink-0 rounded bg-surface-subtle px-1.5 py-0.5 text-[11px] text-ink-muted">
              {artifact.artifactType}
            </span>
          </div>
          <div className="mt-1 text-xs text-ink-muted">{formatTime(storedTimeToIso(artifact.createdAt))}</div>
          {artifact.summary ? <p className="mt-2 text-sm leading-5 text-ink-soft">{artifact.summary}</p> : null}
          {artifact.path
            ? (
                <div className="mt-2 truncate rounded-md bg-surface-subtle px-2 py-1.5 text-xs text-ink-muted" title={artifact.path}>
                  {artifact.path}
                </div>
              )
            : null}
          {!artifact.path && artifact.content
            ? (
                <pre className="mt-2 max-h-32 overflow-auto rounded-md bg-surface-subtle p-2 text-[11px] leading-4 text-ink-soft">
                  <code>{artifact.content}</code>
                </pre>
              )
            : null}
        </div>
        <BookMarked className="mt-0.5 size-4 shrink-0 text-ink-muted" />
      </div>
    </article>
  );
}
