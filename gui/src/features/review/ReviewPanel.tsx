import type {
  GitReview,
  GitReviewFile,
  LastRunReviewData,
  StoredReviewFileChange,
} from "../../integrations/storage/types";
import { AlertTriangle, GitBranch } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { DiffView } from "../../components/ui/DiffView";
import { EmptyState } from "../../components/ui/EmptyState";
import { Select } from "../../components/ui/Select";
import { getLastRunReview, retryRunReview, storedTimeToIso } from "../../integrations/storage/threadStore";
import { formatTime } from "../../lib/date";
import { onFutureEvent } from "../../lib/futureEvents";
import { CollapsibleFileDiff } from "./CollapsibleFileDiff";
import { useExpandableFiles } from "./useExpandableFiles";

export type ReviewBase = "custom" | "head" | "merge-base" | "upstream";

type ReviewView = "git_changes" | "last_run";

export function ReviewPanel({
  changePreview = "ready",
  customBase,
  onCustomBaseChange,
  onReviewBaseChange,
  review,
  reviewBase,
  threadId,
}: {
  changePreview?: "ready" | "unsupported_too_large";
  customBase: string;
  onCustomBaseChange: (value: string) => void;
  onReviewBaseChange: (value: ReviewBase) => void;
  review: GitReview | null;
  reviewBase: ReviewBase;
  threadId: string;
}) {
  const isGit = review?.isGitWorkspace ?? false;
  const [activeView, setActiveView] = useState<ReviewView>(isGit ? "git_changes" : "last_run");
  const [runReview, setRunReview] = useState<LastRunReviewData | null>(null);
  const [runError, setRunError] = useState<string | null>(null);
  const [loadingRun, setLoadingRun] = useState(false);
  const [retrying, setRetrying] = useState(false);

  // A non-git Workspace only ever shows the last-run view.
  useEffect(() => {
    if (!isGit)
      setActiveView("last_run");
  }, [isGit]);

  const loadRunReview = useCallback(async () => {
    if (changePreview === "unsupported_too_large") {
      setRunReview(null);
      return;
    }
    setLoadingRun(true);
    setRunError(null);
    try {
      setRunReview(await getLastRunReview(threadId));
    }
    catch (error) {
      setRunError(error instanceof Error ? error.message : String(error));
    }
    finally {
      setLoadingRun(false);
    }
  }, [threadId, changePreview]);

  // Initial load + reload when the thread or capability changes.
  useEffect(() => {
    void loadRunReview();
  }, [loadRunReview]);

  // Refresh when a Run on this thread finishes (its changeset just landed).
  useEffect(() => onFutureEvent("review-updated", (detail) => {
    if (detail.threadId === threadId)
      void loadRunReview();
  }), [threadId, loadRunReview]);

  async function handleRetry() {
    const runId = runReview?.run?.id ?? runReview?.changeset.runId;
    if (!runId)
      return;
    setRetrying(true);
    setRunError(null);
    try {
      const next = await retryRunReview(runId);
      setRunReview(next);
    }
    catch (error) {
      setRunError(error instanceof Error ? error.message : String(error));
    }
    finally {
      setRetrying(false);
    }
  }

  const lastRun = (
    <LastRunReview
      changePreview={changePreview}
      error={runError}
      loading={loadingRun}
      retrying={retrying}
      review={runReview}
      onRetry={handleRetry}
    />
  );

  // Non-git Workspace: just the last-run view under a static heading (§3.2).
  if (!isGit) {
    return (
      <div className="space-y-3">
        <div className="border-b border-line-soft pb-3 text-xs font-medium text-ink-muted">上一轮变更</div>
        {lastRun}
      </div>
    );
  }

  const reviewData = review!;

  return (
    <div className="space-y-3">
      {activeView === "git_changes"
        ? (
            <ReviewHeader
              customBase={customBase}
              review={reviewData}
              reviewBase={reviewBase}
              onCustomBaseChange={onCustomBaseChange}
              onReviewBaseChange={onReviewBaseChange}
            />
          )
        : null}
      <div className="grid grid-cols-2 gap-1 rounded-md bg-surface p-1">
        <ViewTab active={activeView === "git_changes"} label="Git changes" onClick={() => setActiveView("git_changes")} />
        <ViewTab active={activeView === "last_run"} label="上一轮变更" onClick={() => setActiveView("last_run")} />
      </div>
      {activeView === "last_run"
        ? lastRun
        : <WorkingTreeReview files={reviewData.files} />}
    </div>
  );
}

function ViewTab({ active, label, onClick }: { active: boolean; label: string; onClick: () => void }) {
  return (
    <button
      className={active
        ? "h-8 rounded bg-surface-subtle text-sm font-medium text-ink"
        : "h-8 rounded text-sm font-medium text-ink-muted transition-colors hover:text-ink"}
      onClick={onClick}
      type="button"
    >
      {label}
    </button>
  );
}

function WorkingTreeReview({ files }: { files: GitReviewFile[] }) {
  // Files default collapsed; open state is keyed by path.
  const { allOpen, isOpen, toggle, toggleAll } = useExpandableFiles(files, file => file.path);

  return (
    <>
      {files.length > 0
        ? <ExpandCollapseAll allOpen={allOpen} onToggle={toggleAll} />
        : null}
      <div className="space-y-3">
        {files.length === 0
          ? <EmptyState title="工作树没有未提交变化" detail="当前分支相对所选 base 没有变化。" />
          : files.map(file => (
              <GitFileDiff
                file={file}
                key={file.path}
                open={isOpen(file)}
                onToggle={() => toggle(file)}
              />
            ))}
      </div>
    </>
  );
}

function ExpandCollapseAll({ allOpen, onToggle }: { allOpen: boolean; onToggle: () => void }) {
  return (
    <div className="flex items-center justify-end">
      <button className="text-xs text-ink-muted transition-colors hover:text-ink" onClick={onToggle} type="button">
        {allOpen ? "全部收起" : "全部展开"}
      </button>
    </div>
  );
}

function LastRunReview({
  changePreview,
  error,
  loading,
  onRetry,
  retrying,
  review,
}: {
  changePreview: "ready" | "unsupported_too_large";
  error: string | null;
  loading: boolean;
  onRetry: () => void;
  retrying: boolean;
  review: LastRunReviewData | null;
}) {
  // Files default collapsed; open state is keyed by file-change id.
  const files = review?.files ?? [];
  const { allOpen, isOpen, toggle, toggleAll } = useExpandableFiles(files, file => file.id);

  if (changePreview === "unsupported_too_large")
    return <EmptyState title="目录过大，已关闭改动预览" detail="该 Workspace 文件过多，暂不生成改动预览。" />;

  if (loading && !review)
    return <div className="rounded-md border border-line-soft bg-surface p-3 text-sm text-ink-muted">加载上一轮变更…</div>;

  if (error)
    return <div className="rounded-md border border-danger-line bg-danger-soft p-3 text-sm text-danger">{error}</div>;

  if (!review)
    return <EmptyState title="还没有可供审查的上一轮运行" detail="完成一轮 Agent 运行后，文件变化会显示在这里。" />;

  const { changeset } = review;
  return (
    <div className="space-y-3">
      <RunReviewBanners review={review} retrying={retrying} onRetry={onRetry} />
      <section className="rounded-md border border-line-soft bg-surface p-3">
        <div className="text-sm font-semibold text-ink">{changeset.title}</div>
        <div className="mt-1 text-xs text-ink-muted">
          {formatTime(storedTimeToIso(changeset.createdAt))}
          {" · "}
          {changeset.filesChanged}
          {" 个文件 "}
          <span className="text-success">{`+${changeset.additions}`}</span>
          {" "}
          <span className="text-danger">{`-${changeset.deletions}`}</span>
        </div>
      </section>
      {files.length > 0
        ? <ExpandCollapseAll allOpen={allOpen} onToggle={toggleAll} />
        : null}
      <div className="space-y-2">
        {files.length === 0
          ? <EmptyState title="上一轮没有文件变化" detail="该轮运行没有产生工作区文件变化。" />
          : files.map(file => (
              <ChangesetFileChange
                file={file}
                key={file.id}
                open={isOpen(file)}
                onToggle={() => toggle(file)}
              />
            ))}
      </div>
    </div>
  );
}

function RunReviewBanners({
  onRetry,
  retrying,
  review,
}: {
  onRetry: () => void;
  retrying: boolean;
  review: LastRunReviewData;
}) {
  const banners: Array<{ key: string; text: string; retry?: boolean }> = [];
  if (review.overlapped)
    banners.push({ key: "overlapped", text: "本轮运行期间该 Workspace 有其他运行，部分变更可能来自并发运行" });
  if (review.confidence === "recovered")
    banners.push({ key: "recovered", text: "应用重启后恢复的快照，变更归属可能不精确" });
  if (review.snapshotStatus === "partial")
    banners.push({ key: "partial", text: "部分文件因大小限制未生成 diff" });
  if (review.snapshotStatus === "incomplete")
    banners.push({ key: "incomplete", text: "本轮快照不完整，可点击重试", retry: true });
  if (review.snapshotStatus === "unavailable")
    banners.push({ key: "unavailable", text: "本轮变更快照不可用" });

  if (banners.length === 0)
    return null;

  return (
    <div className="space-y-2">
      {banners.map(banner => (
        <div
          className="flex items-start gap-2 rounded-md border border-warning-line bg-warning-soft p-2.5 text-xs text-warning"
          key={banner.key}
        >
          <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
          <span className="min-w-0 flex-1">{banner.text}</span>
          {banner.retry
            ? (
                <button
                  className="shrink-0 rounded border border-warning-line px-1.5 py-0.5 font-medium transition-colors hover:bg-surface disabled:opacity-60"
                  disabled={retrying}
                  onClick={onRetry}
                  type="button"
                >
                  {retrying ? "重试中…" : "重试"}
                </button>
              )
            : null}
        </div>
      ))}
    </div>
  );
}

function ChangesetFileChange({
  file,
  onToggle,
  open,
}: {
  file: StoredReviewFileChange;
  onToggle: () => void;
  open: boolean;
}) {
  return (
    <CollapsibleFileDiff
      title={(
        <>
          {file.previousPath ? `${file.previousPath} → ` : ""}
          {file.path ?? "Unknown file"}
        </>
      )}
      headerExtras={(
        <span className="rounded bg-surface-subtle px-1.5 py-0.5 text-[11px] text-ink-muted">
          {changeTypeLabel(file)}
        </span>
      )}
      additions={file.additions}
      deletions={file.deletions}
      showCounts={!file.binary}
      open={open}
      onToggle={onToggle}
    >
      {file.omissionReason === "sensitive"
        ? <div className="px-3 py-3 text-xs text-warning">敏感文件发生变化，内容未保存。</div>
        : file.binary
          ? <BinaryFileDetail file={file} />
          : file.diff
            ? <DiffView diff={file.diff} />
            : <div className="px-3 py-3 text-xs text-ink-muted">无文本 diff。</div>}
      {file.diffTruncated
        ? <div className="px-3 py-2 text-xs text-warning">diff 过大，已截断显示。</div>
        : null}
    </CollapsibleFileDiff>
  );
}

function BinaryFileDetail({ file }: { file: StoredReviewFileChange }) {
  return (
    <div className="space-y-1 px-3 py-3 text-xs text-ink-muted">
      <div>二进制文件，不支持文本 diff。</div>
      {file.mime
        ? (
            <div>
              类型：
              {file.mime}
            </div>
          )
        : null}
      <div>
        大小：
        {formatBytes(file.beforeSize)}
        {" → "}
        {formatBytes(file.afterSize)}
      </div>
    </div>
  );
}

function changeTypeLabel(file: StoredReviewFileChange) {
  if (file.omissionReason === "sensitive")
    return "敏感文件";
  if (file.binary)
    return "二进制";
  switch (file.changeType) {
    case "A":
      return "已添加";
    case "M":
      return "已修改";
    case "D":
      return "已删除";
    case "R":
      return "已重命名";
    case "C":
      return "已复制";
    default:
      return file.changeType;
  }
}

function formatBytes(size?: number | null) {
  if (size == null)
    return "—";
  if (size < 1024)
    return `${size} B`;
  if (size < 1024 * 1024)
    return `${(size / 1024).toFixed(1)} KiB`;
  return `${(size / (1024 * 1024)).toFixed(1)} MiB`;
}

function ReviewHeader({
  customBase,
  onCustomBaseChange,
  onReviewBaseChange,
  review,
  reviewBase,
}: {
  customBase: string;
  onCustomBaseChange: (value: string) => void;
  onReviewBaseChange: (value: ReviewBase) => void;
  review: GitReview;
  reviewBase: ReviewBase;
}) {
  return (
    <div className="space-y-2 border-b border-line-soft pb-3">
      <div className="flex items-center gap-3 text-sm">
        <div className="inline-flex min-w-0 items-center gap-2 text-ink">
          <GitBranch className="size-4 shrink-0 text-ink-soft" />
          <span className="truncate">{review.branch ?? "HEAD"}</span>
        </div>
        <span className="font-medium text-success">
          +
          {review.additions.toLocaleString()}
        </span>
        <span className="font-medium text-danger">
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
        <Select
          className="text-ink-soft hover:border-line"
          id="review-base-select"
          onChange={event => onReviewBaseChange(event.target.value as ReviewBase)}
          size="sm"
          value={reviewBase}
        >
          <option value="head">HEAD</option>
          <option disabled={!review.upstream} value="upstream">Upstream</option>
          <option disabled={!review.upstream} value="merge-base">Merge base</option>
          <option value="custom">Custom commit</option>
        </Select>
        {reviewBase === "custom"
          ? (
              <input
                className="h-8 rounded-md border border-line-soft bg-surface px-2 text-sm text-ink outline-none transition-colors placeholder:text-ink-muted hover:border-line focus:border-focus focus:ring-2 focus:ring-focus"
                onChange={event => onCustomBaseChange(event.target.value)}
                placeholder="Commit, tag, or branch"
                value={customBase}
              />
            )
          : null}
      </div>
    </div>
  );
}

function GitFileDiff({
  file,
  onToggle,
  open,
}: {
  file: GitReviewFile;
  onToggle: () => void;
  open: boolean;
}) {
  return (
    <CollapsibleFileDiff
      title={file.path}
      additions={file.additions}
      deletions={file.deletions}
      open={open}
      onToggle={onToggle}
    >
      {file.diff
        ? <DiffView diff={file.diff} />
        : <div className="px-3 py-3 text-xs text-ink-muted">No textual diff available.</div>}
    </CollapsibleFileDiff>
  );
}
