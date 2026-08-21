import type { GitReview, GitReviewFile } from "../../integrations/storage/types";
import { GitBranch } from "lucide-react";
import { useTranslation } from "react-i18next";
import { DiffView } from "../../components/ui/DiffView";
import { EmptyState } from "../../components/ui/EmptyState";
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
      <div className="space-y-2">
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
  review,
}: { review: GitReview }) {
  const { i18n } = useTranslation("review");
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
