import type { GitReview, GitReviewFile } from "../../integrations/storage/types";
import { GitBranch } from "lucide-react";
import { useTranslation } from "react-i18next";
import { DiffView } from "../../components/ui/DiffView";
import { EmptyState } from "../../components/ui/EmptyState";
import { FloatingScrollbar } from "../../components/ui/FloatingScrollbar";
import { useFloatingScrollbar } from "../../lib/useFloatingScrollbar";
import { CollapsibleFileDiff, ExpandCollapseAll } from "./CollapsibleFileDiff";
import { useExpandableFiles } from "./useExpandableFiles";

export function WorkingTreeReview({
  review,
  showBranch = true,
}: {
  review: GitReview;
  showBranch?: boolean;
}) {
  const { t } = useTranslation("review");
  const { files } = review;
  // Files default collapsed; open state is keyed by path.
  const { hasOpen, isOpen, toggle, toggleAll } = useExpandableFiles(files, file => file.path);
  const fileScrollbar = useFloatingScrollbar();

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 items-center justify-between gap-2 px-4 py-2">
        <ReviewHeader review={review} files={files} showBranch={showBranch} />
        {files.length > 0 ? <ExpandCollapseAll hasOpen={hasOpen} onToggle={toggleAll} /> : null}
      </div>
      <div className="group relative min-h-0 flex-1 border-t border-line-soft">
        <div
          className="floating-scrollbar h-full overflow-x-hidden overflow-y-auto"
          onScroll={fileScrollbar.handleScroll}
          ref={fileScrollbar.scrollRef}
        >
          {files.length === 0
            ? <EmptyState title={t("workingTree.emptyTitle")} detail={t("workingTree.emptyDetail")} />
            : files.map(file => (
                <GitFileDiff
                  file={file}
                  key={file.path}
                  open={isOpen(file)}
                  onToggle={() => toggle(file)}
                />
              ))}
        </div>
        <FloatingScrollbar scrollbar={fileScrollbar.scrollbar} onPointerDown={fileScrollbar.handleThumbPointerDown} />
      </div>
    </div>
  );
}

export function ReviewHeader({
  review,
  files,
  showBranch,
}: { review: GitReview; files: GitReviewFile[]; showBranch: boolean }) {
  const { i18n } = useTranslation("review");
  const numberFormat = new Intl.NumberFormat(i18n.language);
  return (
    <div className="flex min-w-0 items-center gap-2 text-xs">
      {showBranch
        ? (
            <div className="inline-flex min-w-0 items-center gap-1.5 text-ink">
              <GitBranch className="size-3.5 shrink-0 text-ink-soft" />
              <span className="truncate font-medium">{review.branch ?? "HEAD"}</span>
            </div>
          )
        : null}
      <ReviewStats
        additions={review.additions}
        deletions={review.deletions}
        filesChanged={files.length}
        numberFormat={numberFormat}
      />
    </div>
  );
}

export function ReviewStats({
  additions,
  deletions,
  filesChanged,
  numberFormat,
}: {
  additions: number;
  deletions: number;
  filesChanged: number;
  numberFormat: Intl.NumberFormat;
}) {
  const { t } = useTranslation("review");
  return (
    <span className="flex shrink-0 items-center gap-2 text-xs text-ink-muted">
      <span>{t("lastRun.filesChanged", { count: filesChanged })}</span>
      <span className="font-medium text-success">
        +
        {numberFormat.format(additions)}
      </span>
      <span className="font-medium text-danger">
        -
        {numberFormat.format(deletions)}
      </span>
    </span>
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
  const { t } = useTranslation("review");
  return (
    <CollapsibleFileDiff
      title={file.path}
      additions={file.additions}
      deletions={file.deletions}
      showCounts={!file.binary}
      open={open}
      onToggle={onToggle}
    >
      {file.omissionReason === "sensitive"
        ? <div className="px-3 py-3 text-xs text-warning">{t("file.sensitiveContent")}</div>
        : file.binary
          ? <div className="px-3 py-3 text-xs text-ink-muted">{t("binary.notSupported")}</div>
          : file.omissionReason === "too_large" || file.omissionReason === "total_limit"
            ? <div className="px-3 py-3 text-xs text-ink-muted">{t("git.diffOmitted")}</div>
            : file.diff
              ? <DiffView diff={file.diff} />
              : <div className="px-3 py-3 text-xs text-ink-muted">{t("git.noTextDiff")}</div>}
    </CollapsibleFileDiff>
  );
}
