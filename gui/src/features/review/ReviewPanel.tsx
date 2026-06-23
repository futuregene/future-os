import type {
  GitReview,
  GitReviewFile,
  StoredReviewChangeset,
  StoredReviewFileChange,
} from "../../integrations/storage/types";
import { Check, ChevronDown, ChevronRight, FileDiff, GitBranch, ListTree, Search } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { DiffView } from "../../components/ui/DiffView";
import { EmptyState } from "../../components/ui/EmptyState";
import {
  listReviewChangesets,
  listReviewFileChanges,
  storedTimeToIso,
  updateReviewChangesetStatus,
} from "../../integrations/storage/threadStore";
import { formatTime } from "../../lib/date";

export type ReviewBase = "custom" | "head" | "merge-base" | "upstream";

type ReviewChangesetStatus = "applied" | "discarded" | "pending";

export function ReviewPanel({
  customBase,
  onCustomBaseChange,
  onReviewBaseChange,
  review,
  reviewBase,
  selectedReviewId,
  threadId,
}: {
  customBase: string;
  onCustomBaseChange: (value: string) => void;
  onReviewBaseChange: (value: ReviewBase) => void;
  review: GitReview | null;
  reviewBase: ReviewBase;
  selectedReviewId?: string | null;
  threadId: string;
}) {
  const [query, setQuery] = useState("");
  const [reviewMode, setReviewMode] = useState<"changesets" | "working-tree">("working-tree");
  const [changesets, setChangesets] = useState<StoredReviewChangeset[]>([]);
  const [changesetFiles, setChangesetFiles] = useState<StoredReviewFileChange[]>([]);
  const [changesetsError, setChangesetsError] = useState<string | null>(null);
  const [loadingChangesets, setLoadingChangesets] = useState(false);
  const [selectedChangesetId, setSelectedChangesetId] = useState<string | null>(selectedReviewId ?? null);
  const [updatingChangesetId, setUpdatingChangesetId] = useState<string | null>(null);
  const [viewedFiles, setViewedFiles] = useState<Record<string, boolean>>({});
  const reviewFiles = review?.files;
  const filteredFiles = useMemo(
    () => (reviewFiles ?? []).filter(file => reviewFileMatches(file, query)),
    [query, reviewFiles],
  );
  const selectedChangeset = selectedChangesetId
    ? changesets.find(changeset => changeset.id === selectedChangesetId) ?? null
    : changesets[0] ?? null;

  useEffect(() => {
    let cancelled = false;

    async function loadChangesets() {
      setLoadingChangesets(true);
      setChangesetsError(null);
      try {
        const nextChangesets = await listReviewChangesets(threadId);
        if (!cancelled) {
          setChangesets(nextChangesets);
          setSelectedChangesetId(current =>
            current && nextChangesets.some(changeset => changeset.id === current)
              ? current
              : selectedReviewId ?? nextChangesets[0]?.id ?? null,
          );
        }
      }
      catch (error) {
        if (!cancelled) {
          setChangesetsError(error instanceof Error ? error.message : String(error));
        }
      }
      finally {
        if (!cancelled) {
          setLoadingChangesets(false);
        }
      }
    }

    void loadChangesets();
    return () => {
      cancelled = true;
    };
  }, [selectedReviewId, threadId]);

  useEffect(() => {
    if (!selectedReviewId)
      return;

    setSelectedChangesetId(selectedReviewId);
    setReviewMode("changesets");
  }, [selectedReviewId]);

  useEffect(() => {
    let cancelled = false;

    async function loadChangesetFiles() {
      if (!selectedChangeset?.id) {
        setChangesetFiles([]);
        return;
      }

      try {
        const nextFiles = await listReviewFileChanges(selectedChangeset.id);
        if (!cancelled) {
          setChangesetFiles(nextFiles);
        }
      }
      catch (error) {
        if (!cancelled) {
          setChangesetsError(error instanceof Error ? error.message : String(error));
          setChangesetFiles([]);
        }
      }
    }

    void loadChangesetFiles();
    return () => {
      cancelled = true;
    };
  }, [selectedChangeset?.id]);

  async function handleChangesetStatusChange(changesetId: string, status: ReviewChangesetStatus) {
    setUpdatingChangesetId(changesetId);
    setChangesetsError(null);
    try {
      const updated = await updateReviewChangesetStatus({ changesetId, status });
      setChangesets(current =>
        current.map(changeset => changeset.id === updated.id ? updated : changeset),
      );
    }
    catch (error) {
      setChangesetsError(error instanceof Error ? error.message : String(error));
    }
    finally {
      setUpdatingChangesetId(null);
    }
  }

  if (!review?.isGitWorkspace) {
    return <EmptyState title="No Git workspace" detail="Review is available for Git-backed workspaces." />;
  }

  const viewedCount = review.files.filter(file => viewedFiles[file.path]).length;

  return (
    <div className="space-y-3">
      <ReviewHeader
        customBase={customBase}
        review={review}
        reviewBase={reviewBase}
        viewedCount={viewedCount}
        onCustomBaseChange={onCustomBaseChange}
        onReviewBaseChange={onReviewBaseChange}
      />
      <div className="grid grid-cols-2 gap-1 rounded-md bg-surface p-1">
        <button
          className={reviewMode === "working-tree"
            ? "h-8 rounded bg-surface-subtle text-sm font-medium text-ink"
            : "h-8 rounded text-sm font-medium text-ink-muted transition-colors hover:text-ink"}
          onClick={() => setReviewMode("working-tree")}
          type="button"
        >
          Working tree
        </button>
        <button
          className={reviewMode === "changesets"
            ? "h-8 rounded bg-surface-subtle text-sm font-medium text-ink"
            : "h-8 rounded text-sm font-medium text-ink-muted transition-colors hover:text-ink"}
          onClick={() => setReviewMode("changesets")}
          type="button"
        >
          Run changes
        </button>
      </div>
      {reviewMode === "changesets"
        ? (
            <RunChangesetsReview
              changesetFiles={changesetFiles}
              changesets={changesets}
              error={changesetsError}
              loading={loadingChangesets}
              selectedChangeset={selectedChangeset}
              updatingChangesetId={updatingChangesetId}
              onSelectChangeset={setSelectedChangesetId}
              onStatusChange={handleChangesetStatusChange}
            />
          )
        : (
            <WorkingTreeReview
              filteredFiles={filteredFiles}
              query={query}
              viewedFiles={viewedFiles}
              onQueryChange={setQuery}
              onViewedChange={(filePath, viewed) => {
                setViewedFiles(current => ({ ...current, [filePath]: viewed }));
              }}
            />
          )}
    </div>
  );
}

function WorkingTreeReview({
  filteredFiles,
  onQueryChange,
  onViewedChange,
  query,
  viewedFiles,
}: {
  filteredFiles: GitReviewFile[];
  onQueryChange: (query: string) => void;
  onViewedChange: (path: string, viewed: boolean) => void;
  query: string;
  viewedFiles: Record<string, boolean>;
}) {
  return (
    <>
      <label className="relative block">
        <span className="sr-only">Search changed files</span>
        <Search className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-ink-muted" />
        <input
          className="h-8 w-full rounded-md border border-line-soft bg-surface pl-8 pr-2 text-sm text-ink outline-none transition-colors placeholder:text-ink-muted hover:border-line focus:border-blue-300 focus:ring-2 focus:ring-blue-100"
          onChange={event => onQueryChange(event.target.value)}
          placeholder="Search files or diff..."
          value={query}
        />
      </label>
      <div className="space-y-3">
        {filteredFiles.length === 0
          ? <EmptyState title="No matching changes" detail="Try a different file name or diff search." />
          : filteredFiles.map(file => (
              <GitFileDiff
                file={file}
                key={file.path}
                viewed={viewedFiles[file.path] ?? false}
                onViewedChange={(viewed) => {
                  onViewedChange(file.path, viewed);
                }}
              />
            ))}
      </div>
    </>
  );
}

function RunChangesetsReview({
  changesetFiles,
  changesets,
  error,
  loading,
  onSelectChangeset,
  onStatusChange,
  selectedChangeset,
  updatingChangesetId,
}: {
  changesetFiles: StoredReviewFileChange[];
  changesets: StoredReviewChangeset[];
  error: string | null;
  loading: boolean;
  onSelectChangeset: (changesetId: string) => void;
  onStatusChange: (changesetId: string, status: ReviewChangesetStatus) => void;
  selectedChangeset: StoredReviewChangeset | null;
  updatingChangesetId: string | null;
}) {
  if (loading) {
    return <div className="rounded-md border border-line-soft bg-surface p-3 text-sm text-ink-muted">Loading run changes...</div>;
  }

  if (changesets.length === 0) {
    return (
      <div className="space-y-3">
        {error ? <div className="rounded-md border border-red-200 bg-red-50 p-3 text-sm text-red-700">{error}</div> : null}
        <EmptyState title="No run changesets" detail="Write and edit operations will appear here after agent runs." />
      </div>
    );
  }

  const selectedStatus = selectedChangeset
    ? normalizeReviewChangesetStatus(selectedChangeset.status)
    : "pending";
  const isUpdating = selectedChangeset ? updatingChangesetId === selectedChangeset.id : false;

  return (
    <div className="space-y-3">
      {error ? <div className="rounded-md border border-red-200 bg-red-50 p-3 text-sm text-red-700">{error}</div> : null}
      <label className="grid gap-1">
        <span className="text-xs font-medium text-ink-muted">Run changeset</span>
        <select
          className="h-8 rounded-md border border-line-soft bg-surface px-2 text-sm text-ink-soft outline-none transition-colors hover:border-line focus:border-blue-300 focus:ring-2 focus:ring-blue-100"
          value={selectedChangeset?.id ?? ""}
          onChange={event => onSelectChangeset(event.target.value)}
        >
          {changesets.map(changeset => (
            <option key={changeset.id} value={changeset.id}>
              {changeset.title || changeset.id}
            </option>
          ))}
        </select>
      </label>
      {selectedChangeset
        ? (
            <section className="rounded-md border border-line-soft bg-surface p-3">
              <div className="flex items-start gap-2">
                <ListTree className="mt-0.5 size-4 shrink-0 text-accent" />
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-semibold text-ink">{selectedChangeset.title}</div>
                  <div className="mt-1 text-xs text-ink-muted">
                    {formatTime(storedTimeToIso(selectedChangeset.createdAt))}
                    {" "}
                    ·
                    {" "}
                    {selectedChangeset.filesChanged}
                    {" "}
                    files
                    {selectedChangeset.runId ? ` · ${selectedChangeset.runId}` : ""}
                  </div>
                  {selectedChangeset.summary ? <p className="mt-2 text-sm leading-5 text-ink-soft">{selectedChangeset.summary}</p> : null}
                </div>
              </div>
            </section>
          )
        : null}
      {selectedChangeset
        ? (
            <div className="flex flex-wrap items-center gap-2">
              <span className={reviewChangesetStatusClass(selectedStatus)}>
                {reviewChangesetStatusLabel(selectedStatus)}
              </span>
              {selectedStatus !== "applied"
                ? (
                    <button
                      className="h-8 rounded-md border border-line-soft bg-surface px-2.5 text-xs font-medium text-ink-soft transition-colors hover:border-green-300 hover:text-green-700 disabled:cursor-not-allowed disabled:opacity-60"
                      disabled={isUpdating}
                      onClick={() => onStatusChange(selectedChangeset.id, "applied")}
                      type="button"
                    >
                      Mark applied
                    </button>
                  )
                : null}
              {selectedStatus !== "discarded"
                ? (
                    <button
                      className="h-8 rounded-md border border-line-soft bg-surface px-2.5 text-xs font-medium text-ink-soft transition-colors hover:border-red-300 hover:text-red-700 disabled:cursor-not-allowed disabled:opacity-60"
                      disabled={isUpdating}
                      onClick={() => onStatusChange(selectedChangeset.id, "discarded")}
                      type="button"
                    >
                      Discard
                    </button>
                  )
                : null}
              {selectedStatus !== "pending"
                ? (
                    <button
                      className="h-8 rounded-md border border-line-soft bg-surface px-2.5 text-xs font-medium text-ink-soft transition-colors hover:border-blue-300 hover:text-blue-700 disabled:cursor-not-allowed disabled:opacity-60"
                      disabled={isUpdating}
                      onClick={() => onStatusChange(selectedChangeset.id, "pending")}
                      type="button"
                    >
                      Reset
                    </button>
                  )
                : null}
            </div>
          )
        : null}
      <div className="space-y-2">
        {changesetFiles.length === 0
          ? <EmptyState title="No files recorded" detail="This changeset does not have file-level details yet." />
          : changesetFiles.map(file => <ChangesetFileChange file={file} key={file.id} />)}
      </div>
    </div>
  );
}

function normalizeReviewChangesetStatus(status: string): ReviewChangesetStatus {
  if (status === "applied" || status === "discarded" || status === "pending")
    return status;
  return "pending";
}

function reviewChangesetStatusLabel(status: ReviewChangesetStatus) {
  if (status === "applied")
    return "Applied";
  if (status === "discarded")
    return "Discarded";
  return "Pending";
}

function reviewChangesetStatusClass(status: ReviewChangesetStatus) {
  const base = "inline-flex h-8 items-center rounded-md border px-2.5 text-xs font-medium";
  if (status === "applied")
    return `${base} border-green-200 bg-green-50 text-green-700`;
  if (status === "discarded")
    return `${base} border-red-200 bg-red-50 text-red-700`;
  return `${base} border-amber-200 bg-amber-50 text-amber-700`;
}

function ChangesetFileChange({ file }: { file: StoredReviewFileChange }) {
  return (
    <section className="rounded-md border border-line-soft bg-surface p-3">
      <div className="flex items-start gap-2">
        <FileDiff className="mt-0.5 size-4 shrink-0 text-orange-500" />
        <div className="min-w-0 flex-1">
          <div className="flex items-center justify-between gap-2">
            <div className="truncate text-sm font-medium text-ink-soft">{file.path ?? file.targetId ?? "Unknown file"}</div>
            <span className="shrink-0 rounded bg-surface-subtle px-1.5 py-0.5 text-[11px] text-ink-muted">{file.changeType}</span>
          </div>
          {file.summary ? <p className="mt-2 text-xs leading-5 text-ink-soft">{file.summary}</p> : null}
          {file.diff
            ? (
                <div className="mt-2 border-t border-line-soft pt-2">
                  <DiffView diff={file.diff} />
                </div>
              )
            : null}
        </div>
      </div>
    </section>
  );
}

function reviewFileMatches(file: GitReviewFile, query: string) {
  const normalized = query.trim().toLowerCase();
  if (!normalized)
    return true;

  return file.path.toLowerCase().includes(normalized)
    || file.status.toLowerCase().includes(normalized)
    || file.diff.toLowerCase().includes(normalized);
}

function ReviewHeader({
  customBase,
  onCustomBaseChange,
  onReviewBaseChange,
  review,
  reviewBase,
  viewedCount,
}: {
  customBase: string;
  onCustomBaseChange: (value: string) => void;
  onReviewBaseChange: (value: ReviewBase) => void;
  review: GitReview;
  reviewBase: ReviewBase;
  viewedCount: number;
}) {
  return (
    <div className="space-y-2 border-b border-line-soft pb-3">
      <div className="flex items-center justify-between gap-3">
        <div className="text-xs font-medium text-ink-muted">Working tree review</div>
        {review.files.length > 0
          ? (
              <div className="text-xs text-ink-muted">
                {viewedCount}
                /
                {review.files.length}
                {" "}
                viewed
              </div>
            )
          : null}
      </div>
      <div className="flex items-center gap-3 text-sm">
        <div className="inline-flex min-w-0 items-center gap-2 text-ink">
          <GitBranch className="size-4 shrink-0 text-ink-soft" />
          <span className="truncate">{review.branch ?? "HEAD"}</span>
        </div>
        <span className="font-medium text-green-700">
          +
          {review.additions.toLocaleString()}
        </span>
        <span className="font-medium text-red-600">
          -
          {review.deletions.toLocaleString()}
        </span>
      </div>
      {review.upstream
        ? (
            <div className="truncate text-xs text-ink-muted">
              {review.branch ?? "HEAD"}
              {" "}
              <span className="text-ink-soft">→</span>
              {" "}
              {review.upstream}
            </div>
          )
        : null}
      <div className="grid grid-cols-1 gap-2">
        <label className="text-xs font-medium text-ink-muted" htmlFor="review-base-select">Diff base</label>
        <select
          className="h-8 rounded-md border border-line-soft bg-surface px-2 text-sm text-ink-soft outline-none transition-colors hover:border-line focus:border-blue-300 focus:ring-2 focus:ring-blue-100"
          id="review-base-select"
          value={reviewBase}
          onChange={event => onReviewBaseChange(event.target.value as ReviewBase)}
        >
          <option value="head">HEAD</option>
          <option disabled={!review.upstream} value="upstream">Upstream</option>
          <option disabled={!review.upstream} value="merge-base">Merge base</option>
          <option value="custom">Custom commit</option>
        </select>
        {reviewBase === "custom"
          ? (
              <input
                className="h-8 rounded-md border border-line-soft bg-surface px-2 text-sm text-ink outline-none transition-colors placeholder:text-ink-muted hover:border-line focus:border-blue-300 focus:ring-2 focus:ring-blue-100"
                onChange={event => onCustomBaseChange(event.target.value)}
                placeholder="Commit, tag, or branch"
                value={customBase}
              />
            )
          : null}
        <div className="truncate text-xs text-ink-muted" title={review.diffBase ?? undefined}>
          Current base:
          {" "}
          {review.diffBaseLabel ?? review.diffBase ?? "HEAD"}
        </div>
      </div>
    </div>
  );
}

function GitFileDiff({
  file,
  onViewedChange,
  viewed,
}: {
  file: GitReviewFile;
  onViewedChange: (viewed: boolean) => void;
  viewed: boolean;
}) {
  const [open, setOpen] = useState(true);

  return (
    <section className="overflow-hidden rounded-md border border-line-soft bg-surface">
      <button
        className="flex w-full items-center justify-between gap-3 px-3 py-2 text-left"
        onClick={() => setOpen(value => !value)}
        type="button"
      >
        <span className="flex min-w-0 items-center gap-2">
          <FileDiff className="size-4 shrink-0 text-orange-500" />
          <span className="min-w-0 truncate text-sm font-medium text-ink-soft">{file.path}</span>
        </span>
        <span className="flex shrink-0 items-center gap-2 text-xs">
          <span className="text-green-700">
            +
            {file.additions}
          </span>
          <span className="text-red-600">
            -
            {file.deletions}
          </span>
          {open ? <ChevronDown className="size-4 text-ink-muted" /> : <ChevronRight className="size-4 text-ink-muted" />}
        </span>
      </button>
      <div className="flex items-center justify-between gap-3 border-t border-line-soft px-3 py-2">
        <div className="text-xs text-ink-muted">{file.status}</div>
        <button
          className={viewed
            ? "inline-flex h-7 items-center gap-1.5 rounded-md bg-green-50 px-2 text-xs font-medium text-green-700 ring-1 ring-green-200"
            : "inline-flex h-7 items-center gap-1.5 rounded-md border border-line bg-surface px-2 text-xs font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"}
          onClick={() => onViewedChange(!viewed)}
          type="button"
        >
          {viewed ? <Check className="size-3.5" /> : null}
          {viewed ? "Viewed" : "Mark viewed"}
        </button>
      </div>
      {open
        ? (
            <div className="border-t border-line-soft">
              {file.diff
                ? <DiffView diff={file.diff} />
                : <div className="px-3 py-3 text-xs text-ink-muted">No textual diff available.</div>}
            </div>
          )
        : null}
    </section>
  );
}
