import type { StoredArtifact } from "../../../integrations/storage/threadStore";
import { BookMarked, FileText, Trash2 } from "lucide-react";
import { deleteArtifact, promoteArtifactToResearch, storedTimeToIso } from "../../../integrations/storage/threadStore";
import { formatTime } from "../../../lib/date";
import { Badge } from "../../ui/Badge";
import { EmptyState } from "./ContextEmptyState";

export function ArtifactsPanel({
  artifacts,
  onChanged,
}: {
  artifacts: StoredArtifact[];
  onChanged: () => void;
}) {
  if (artifacts.length === 0) {
    return <EmptyState title="No artifacts yet" detail="Generated reports, summaries, tables, and files will appear here." />;
  }

  return (
    <div className="space-y-3">
      {artifacts.map(artifact => (
        <ArtifactCard artifact={artifact} key={artifact.id} onChanged={onChanged} />
      ))}
    </div>
  );
}

function ArtifactCard({
  artifact,
  onChanged,
}: {
  artifact: StoredArtifact;
  onChanged: () => void;
}) {
  async function handlePromote() {
    await promoteArtifactToResearch(artifact.id);
    onChanged();
  }

  async function handleDelete() {
    await deleteArtifact(artifact.id);
    onChanged();
  }

  return (
    <div className="rounded-md border border-line-soft bg-surface p-3">
      <div className="flex items-start gap-2">
        <FileText className="mt-0.5 size-4 shrink-0 text-accent" />
        <div className="min-w-0 flex-1">
          <div className="flex items-start justify-between gap-2">
            <div className="min-w-0">
              <div className="truncate text-sm font-semibold text-ink">{artifact.title}</div>
              <div className="mt-1 flex items-center gap-2">
                <Badge>{artifact.artifactType}</Badge>
                <span className="text-xs text-ink-muted">{formatTime(storedTimeToIso(artifact.createdAt))}</span>
              </div>
            </div>
            <button
              aria-label={`Add artifact ${artifact.title} to Research`}
              className="inline-flex size-7 shrink-0 items-center justify-center rounded-md text-ink-muted transition-colors hover:bg-accent-soft hover:text-accent"
              onClick={() => void handlePromote()}
              title="Add to Research"
              type="button"
            >
              <BookMarked className="size-3.5" />
            </button>
            <button
              aria-label={`Delete artifact ${artifact.title}`}
              className="inline-flex size-7 shrink-0 items-center justify-center rounded-md text-ink-muted transition-colors hover:bg-red-50 hover:text-red-600"
              onClick={() => void handleDelete()}
              title="Delete artifact"
              type="button"
            >
              <Trash2 className="size-3.5" />
            </button>
          </div>
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
      </div>
    </div>
  );
}
