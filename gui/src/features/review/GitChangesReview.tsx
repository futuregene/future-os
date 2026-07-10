import type { GitReview, GitReviewFile } from "../../integrations/storage/types";
import type { ReviewBase } from "./ReviewPanel";
import { GitBranch } from "lucide-react";
import { useTranslation } from "react-i18next";
import { DiffView } from "../../components/ui/DiffView";
import { EmptyState } from "../../components/ui/EmptyState";
import { Select } from "../../components/ui/Select";
import { TextInput } from "../../components/ui/TextInput";
import { CollapsibleFileDiff, ExpandCollapseAll } from "./CollapsibleFileDiff";
import { useExpandableFiles } from "./useExpandableFiles";

export function WorkingTreeReview({ files }: { files: GitReviewFile[] }) {
  const { t } = useTranslation("review");
  // Files default collapsed; open state is keyed by path.
  const { allOpen, isOpen, toggle, toggleAll } = useExpandableFiles(files, file => file.path);

  return (
    <>
      {files.length > 0
        ? <ExpandCollapseAll allOpen={allOpen} onToggle={toggleAll} />
        : null}
      <div className="space-y-3">
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
    </>
  );
}

export function ReviewHeader({
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
  const { t, i18n } = useTranslation("review");
  const numberFormat = new Intl.NumberFormat(i18n.language);
  return (
    <div className="space-y-2 border-b border-line-soft pb-3">
      <div className="flex items-center gap-3 text-sm">
        <div className="inline-flex min-w-0 items-center gap-2 text-ink">
          <GitBranch className="size-4 shrink-0 text-ink-soft" />
          <span className="truncate">{review.branch ?? "HEAD"}</span>
        </div>
        <span className="font-medium text-success">
          +
          {numberFormat.format(review.additions)}
        </span>
        <span className="font-medium text-danger">
          -
          {numberFormat.format(review.deletions)}
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
        <label className="text-xs font-medium text-ink-muted" htmlFor="review-base-select">{t("header.diffBase")}</label>
        <Select
          className="text-ink-soft hover:border-line"
          id="review-base-select"
          onChange={event => onReviewBaseChange(event.target.value as ReviewBase)}
          size="sm"
          value={reviewBase}
        >
          <option value="head">{t("header.base.head")}</option>
          <option disabled={!review.upstream} value="upstream">{t("header.base.upstream")}</option>
          <option disabled={!review.upstream} value="merge-base">{t("header.base.mergeBase")}</option>
          <option value="custom">{t("header.base.custom")}</option>
        </Select>
        {reviewBase === "custom"
          ? (
              <TextInput
                className="h-8 w-auto px-2 hover:border-line"
                onChange={event => onCustomBaseChange(event.target.value)}
                placeholder={t("header.customPlaceholder")}
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
  const { t } = useTranslation("review");
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
        : <div className="px-3 py-3 text-xs text-ink-muted">{t("git.noTextDiff")}</div>}
    </CollapsibleFileDiff>
  );
}
