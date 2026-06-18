import type { StoredResearchResource } from "../../integrations/storage/threadStore";
import { BookOpen, FileText, LocateFixed } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Badge } from "../../components/ui/Badge";
import { EmptyState } from "../../components/ui/EmptyState";
import { listResearchResources, storedTimeToIso } from "../../integrations/storage/threadStore";
import { formatTime } from "../../lib/date";
import { startWindowDrag } from "../../lib/windowDrag";

interface ResearchViewProps {
  selectedResourceId?: string | null;
  workspaceId: string | null;
  workspaceName: string;
}

export function ResearchView({ selectedResourceId, workspaceId, workspaceName }: ResearchViewProps) {
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [resources, setResources] = useState<StoredResearchResource[]>([]);
  const selectedResource = useMemo(
    () => selectedResourceId ? resources.find(resource => resource.id === selectedResourceId) ?? null : null,
    [resources, selectedResourceId],
  );

  useEffect(() => {
    let cancelled = false;

    async function loadResources() {
      if (!workspaceId) {
        setResources([]);
        return;
      }

      setLoading(true);
      setError(null);
      try {
        const nextResources = await listResearchResources(workspaceId);
        if (!cancelled) {
          setResources(nextResources);
        }
      }
      catch (reason) {
        if (!cancelled) {
          setError(reason instanceof Error ? reason.message : String(reason));
        }
      }
      finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    }

    void loadResources();

    return () => {
      cancelled = true;
    };
  }, [workspaceId]);

  return (
    <section className="flex h-full min-h-0 flex-col bg-surface">
      <header
        className="flex h-12 shrink-0 select-none items-center border-b border-line-soft px-8"
        onMouseDown={startWindowDrag}
      >
        <div className="min-w-0" data-tauri-drag-region>
          <h1 className="truncate text-base font-semibold text-ink">Research</h1>
          <p className="truncate text-xs text-ink-muted">{workspaceName}</p>
        </div>
      </header>
      <div className="min-h-0 flex-1 overflow-auto px-8 py-6">
        <div className="mx-auto max-w-4xl">
          <div className="mb-5 flex items-center justify-between gap-4">
            <div>
              <h2 className="text-lg font-semibold text-ink">Resources</h2>
              <p className="mt-1 text-sm text-ink-muted">
                Artifacts promoted from chats and workspace runs become reusable research material here.
              </p>
            </div>
            <div className="rounded-md border border-line-soft bg-surface-subtle px-3 py-2 text-sm text-ink-soft">
              {resources.length}
              {" "}
              resources
            </div>
          </div>

          {loading ? <div className="py-4 text-sm text-ink-muted">Loading research resources...</div> : null}
          {error ? <div className="rounded-md border border-red-200 bg-red-50 p-3 text-sm text-red-700">{error}</div> : null}
          {!loading && !error && !workspaceId
            ? (
                <EmptyState title="No workspace selected" detail="Open a workspace or chat before collecting research resources." />
              )
            : null}
          {!loading && !error && workspaceId && resources.length === 0
            ? (
                <EmptyState title="No resources yet" detail="Use the Artifacts panel to add generated work into Research." />
              )
            : null}
          {resources.length > 0
            ? (
                <div className="grid gap-3">
                  {selectedResource
                    ? (
                        <section className="rounded-md border border-blue-200 bg-blue-50 p-3">
                          <div className="mb-2 flex items-center gap-1.5 text-xs font-medium text-blue-700">
                            <LocateFixed className="size-3.5" />
                            Selected from chat
                          </div>
                          <ResearchResourceCard highlighted resource={selectedResource} />
                        </section>
                      )
                    : null}
                  {resources.filter(resource => resource.id !== selectedResourceId).map(resource => (
                    <ResearchResourceCard
                      key={resource.id}
                      resource={resource}
                    />
                  ))}
                </div>
              )
            : null}
        </div>
      </div>
    </section>
  );
}

function ResearchResourceCard({
  highlighted,
  resource,
}: {
  highlighted?: boolean;
  resource: StoredResearchResource;
}) {
  return (
    <article className={highlighted ? "rounded-lg border border-blue-200 bg-white p-4 shadow-sm" : "rounded-lg border border-line-soft bg-white p-4"}>
      <div className="flex items-start gap-3">
        {resource.sourceArtifactId
          ? <FileText className="mt-0.5 size-4 shrink-0 text-accent" />
          : <BookOpen className="mt-0.5 size-4 shrink-0 text-accent" />}
        <div className="min-w-0 flex-1">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <h3 className="truncate text-sm font-semibold text-ink">{resource.title}</h3>
              <div className="mt-1 flex items-center gap-2">
                <Badge>{resource.resourceType}</Badge>
                {resource.sourceArtifactId ? <Badge tone="accent">Artifact</Badge> : null}
                <span className="text-xs text-ink-muted">{formatTime(storedTimeToIso(resource.createdAt))}</span>
              </div>
            </div>
          </div>
          {resource.summary ? <p className="mt-3 text-sm leading-5 text-ink-soft">{resource.summary}</p> : null}
          {resource.sourceUri
            ? (
                <div className="mt-3 truncate rounded-md bg-surface-subtle px-2 py-1.5 text-xs text-ink-muted" title={resource.sourceUri}>
                  {resource.sourceUri}
                </div>
              )
            : null}
          {resource.content
            ? (
                <pre className="mt-3 max-h-56 overflow-auto whitespace-pre-wrap rounded-md bg-surface-subtle p-2 text-xs leading-5 text-ink-soft">
                  <code>{resource.content}</code>
                </pre>
              )
            : null}
        </div>
      </div>
    </article>
  );
}
