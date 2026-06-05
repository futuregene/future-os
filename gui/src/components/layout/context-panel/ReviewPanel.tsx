import type {
  StoredReviewChangeset,
  StoredReviewFileChange,
} from "../../../integrations/storage/threadStore";
import { ChevronDown, ChevronRight, FileDiff } from "lucide-react";
import { useState } from "react";
import { Badge } from "../../ui/Badge";
import { EmptyState } from "./ContextEmptyState";

export function ReviewPanel({
  changesets,
  filesByChangeset,
}: {
  changesets: StoredReviewChangeset[];
  filesByChangeset: Record<string, StoredReviewFileChange[]>;
}) {
  if (changesets.length === 0) {
    return <EmptyState title="No review changes" detail="File and artifact changes will appear here." />;
  }

  return (
    <div className="space-y-3">
      {changesets.map(changeset => (
        <ReviewChangesetCard
          changeset={changeset}
          files={filesByChangeset[changeset.id] ?? []}
          key={changeset.id}
        />
      ))}
    </div>
  );
}

function ReviewChangesetCard({
  changeset,
  files,
}: {
  changeset: StoredReviewChangeset;
  files: StoredReviewFileChange[];
}) {
  const [open, setOpen] = useState(true);

  return (
    <div className="rounded-md border border-line-soft bg-surface">
      <button
        className="flex w-full items-center justify-between gap-3 px-3 py-2 text-left"
        onClick={() => setOpen(value => !value)}
        type="button"
      >
        <span className="flex min-w-0 items-center gap-2">
          <FileDiff className="size-4 shrink-0 text-ink-soft" />
          <span className="min-w-0 truncate text-sm font-semibold text-ink">{changeset.title}</span>
        </span>
        {open
          ? (
              <ChevronDown className="size-4 text-ink-muted" />
            )
          : (
              <ChevronRight className="size-4 text-ink-muted" />
            )}
      </button>
      {open
        ? (
            <div className="border-t border-line-soft p-3">
              <div className="mb-2 flex items-center gap-2 text-sm">
                <Badge>
                  {changeset.filesChanged}
                  {" "}
                  files
                </Badge>
                <span className="font-medium text-green-700">
                  +
                  {changeset.additions}
                </span>
                <span className="font-medium text-red-600">
                  -
                  {changeset.deletions}
                </span>
              </div>
              {changeset.summary ? <p className="mb-3 text-sm leading-5 text-ink-soft">{changeset.summary}</p> : null}
              <div className="space-y-2">
                {files.length === 0 ? <div className="text-xs text-ink-muted">No file changes recorded.</div> : null}
                {files.map(file => (
                  <div key={file.id} className="rounded-md border border-line-soft bg-white p-2">
                    <div className="flex items-center justify-between gap-2">
                      <span className="min-w-0 truncate text-xs font-medium text-ink">
                        {file.path ?? file.targetId ?? file.targetType}
                      </span>
                      <span className="shrink-0 text-xs">
                        <span className="text-green-700">
                          +
                          {file.additions}
                        </span>
                        {" "}
                        <span className="text-red-600">
                          -
                          {file.deletions}
                        </span>
                      </span>
                    </div>
                    {file.summary ? <div className="mt-1 text-xs text-ink-muted">{file.summary}</div> : null}
                    {file.diff
                      ? (
                          <pre className="mt-2 max-h-40 overflow-auto rounded-md bg-surface-subtle p-2 text-[11px] leading-4 text-ink-soft">
                            <code>{file.diff}</code>
                          </pre>
                        )
                      : null}
                  </div>
                ))}
              </div>
            </div>
          )
        : null}
    </div>
  );
}
