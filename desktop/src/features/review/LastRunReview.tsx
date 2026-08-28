import type { TFunction } from "i18next";
import type {
  LastRunReviewData,
  StoredReviewFileChange,
} from "../../integrations/storage/types";
import { AlertTriangle } from "lucide-react";
import { useTranslation } from "react-i18next";
import { DiffView } from "../../components/ui/DiffView";
import { EmptyState } from "../../components/ui/EmptyState";
import { FloatingScrollbar } from "../../components/ui/FloatingScrollbar";
import { formatBytes } from "../../lib/format";
import { useFloatingScrollbar } from "../../lib/useFloatingScrollbar";
import { CollapsibleFileDiff, ExpandCollapseAll } from "./CollapsibleFileDiff";
import { ReviewStats } from "./GitChangesReview";
import { useExpandableFiles } from "./useExpandableFiles";

export function LastRunReview({
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
  const { i18n, t } = useTranslation("review");
  // Files default collapsed; open state is keyed by file-change id.
  const files = review?.files ?? [];
  const { hasOpen, isOpen, toggle, toggleAll } = useExpandableFiles(
    files,
    file => file.id,
  );
  const fileScrollbar = useFloatingScrollbar();

  if (changePreview === "unsupported_too_large") {
    return (
      <EmptyState
        className="m-4"
        title={t("lastRun.tooLargeTitle")}
        detail={t("lastRun.tooLargeDetail")}
      />
    );
  }

  if (loading && !review) {
    return (
      <div className="rounded-md border border-line-soft bg-surface p-3 text-sm text-ink-muted">
        {t("lastRun.loading")}
      </div>
    );
  }

  if (error) {
    return (
      <div className="rounded-md border border-danger-line bg-danger-soft p-3 text-sm text-danger">
        {error}
      </div>
    );
  }

  if (!review) {
    return (
      <EmptyState
        className="m-4"
        title={t("lastRun.noReviewTitle")}
        detail={t("lastRun.noReviewDetail")}
      />
    );
  }

  const { changeset } = review;
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="shrink-0 space-y-3 px-4 py-2">
        <RunReviewBanners
          review={review}
          retrying={retrying}
          onRetry={onRetry}
        />
        <div className="flex items-center justify-between gap-2">
          <ReviewStats
            additions={changeset.additions}
            deletions={changeset.deletions}
            filesChanged={changeset.filesChanged}
            numberFormat={new Intl.NumberFormat(i18n.language)}
          />
          {files.length > 0
            ? (
                <ExpandCollapseAll hasOpen={hasOpen} onToggle={toggleAll} />
              )
            : null}
        </div>
      </div>
      <div className="group relative min-h-0 flex-1 border-t border-line-soft/70">
        <div
          className="floating-scrollbar h-full overflow-x-hidden overflow-y-auto bg-surface/50"
          onScroll={fileScrollbar.handleScroll}
          ref={fileScrollbar.scrollRef}
        >
          {files.length === 0
            ? (
                <EmptyState
                  className="m-4"
                  title={t("lastRun.noFilesTitle")}
                  detail={t("lastRun.noFilesDetail")}
                />
              )
            : (
                files.map(file => (
                  <ChangesetFileChange
                    file={file}
                    key={file.id}
                    open={isOpen(file)}
                    onToggle={() => toggle(file)}
                  />
                ))
              )}
        </div>
        <FloatingScrollbar
          scrollbar={fileScrollbar.scrollbar}
          onPointerDown={fileScrollbar.handleThumbPointerDown}
        />
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
  const { t } = useTranslation("review");
  const banners: Array<{ key: string; text: string; retry?: boolean }> = [];
  if (review.overlapped)
    banners.push({ key: "overlapped", text: t("banner.overlapped") });
  if (review.confidence === "recovered")
    banners.push({ key: "recovered", text: t("banner.recovered") });
  if (review.snapshotStatus === "partial")
    banners.push({ key: "partial", text: t("banner.partial") });
  if (review.snapshotStatus === "incomplete") {
    banners.push({
      key: "incomplete",
      text: t("banner.incomplete"),
      retry: true,
    });
  }
  if (review.snapshotStatus === "unavailable")
    banners.push({ key: "unavailable", text: t("banner.unavailable") });

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
                  {retrying ? t("banner.retrying") : t("banner.retry")}
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
  const { t } = useTranslation("review");
  return (
    <CollapsibleFileDiff
      title={(
        <>
          {file.previousPath ? `${file.previousPath} → ` : ""}
          {file.path ?? t("file.unknown")}
        </>
      )}
      headerExtras={(
        <span className="rounded bg-surface-subtle px-1.5 py-0.5 text-[11px] text-ink-muted">
          {changeTypeLabel(file, t)}
        </span>
      )}
      additions={file.additions}
      deletions={file.deletions}
      showCounts={!file.binary}
      open={open}
      onToggle={onToggle}
    >
      {file.omissionReason === "sensitive"
        ? (
            <div className="px-3 py-3 text-xs text-warning">
              {t("file.sensitiveContent")}
            </div>
          )
        : file.binary
          ? (
              <BinaryFileDetail file={file} />
            )
          : file.diff
            ? (
                <DiffView diff={file.diff} />
              )
            : (
                <div className="px-3 py-3 text-xs text-ink-muted">
                  {t("file.noTextDiff")}
                </div>
              )}
      {file.diffTruncated
        ? (
            <div className="px-3 py-2 text-xs text-warning">
              {t("file.diffTruncated")}
            </div>
          )
        : null}
    </CollapsibleFileDiff>
  );
}

function BinaryFileDetail({ file }: { file: StoredReviewFileChange }) {
  const { t } = useTranslation("review");
  return (
    <div className="space-y-1 px-3 py-3 text-xs text-ink-muted">
      <div>{t("binary.notSupported")}</div>
      {file.mime
        ? (
            <div>
              {t("binary.type")}
              {file.mime}
            </div>
          )
        : null}
      <div>
        {t("binary.size")}
        {formatBytes(file.beforeSize)}
        {" → "}
        {formatBytes(file.afterSize)}
      </div>
    </div>
  );
}

function changeTypeLabel(file: StoredReviewFileChange, t: TFunction<"review">) {
  if (file.omissionReason === "sensitive")
    return t("changeType.sensitive");
  if (file.binary)
    return t("changeType.binary");
  switch (file.changeType) {
    case "A":
      return t("changeType.added");
    case "M":
      return t("changeType.modified");
    case "D":
      return t("changeType.deleted");
    case "R":
      return t("changeType.renamed");
    case "C":
      return t("changeType.copied");
    default:
      return file.changeType;
  }
}
