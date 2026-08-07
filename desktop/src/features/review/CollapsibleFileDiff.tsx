import type { ReactNode } from "react";
import { ChevronDown, ChevronRight, FileDiff } from "lucide-react";
import { useTranslation } from "react-i18next";

/**
 * Collapsible file row shared by the working-tree (git) and last-run (shadow)
 * review views: a `FileDiff` icon + path header with optional `+/-` counts and
 * extra header chips, and a collapsible body. Each view assembles its own title
 * / extras / body and renders this.
 */
export function CollapsibleFileDiff({
  title,
  headerExtras,
  additions,
  deletions,
  showCounts = true,
  open,
  onToggle,
  children,
}: {
  title: ReactNode;
  headerExtras?: ReactNode;
  additions?: number;
  deletions?: number;
  showCounts?: boolean;
  open: boolean;
  onToggle: () => void;
  children: ReactNode;
}) {
  return (
    <section className="overflow-hidden rounded-md border border-line-soft bg-surface">
      <button
        className="flex w-full items-center justify-between gap-3 px-3 py-2 text-left"
        onClick={onToggle}
        type="button"
      >
        <span className="flex min-w-0 items-center gap-2">
          <FileDiff className="size-4 shrink-0 text-ink-soft" />
          <span className="min-w-0 truncate text-sm font-medium text-ink-soft">{title}</span>
        </span>
        <span className="flex shrink-0 items-center gap-2 text-xs">
          {headerExtras}
          {showCounts
            ? (
                <>
                  <span className="text-success">{`+${additions ?? 0}`}</span>
                  <span className="text-danger">{`-${deletions ?? 0}`}</span>
                </>
              )
            : null}
          {open ? <ChevronDown className="size-4 text-ink-muted" /> : <ChevronRight className="size-4 text-ink-muted" />}
        </span>
      </button>
      {open ? <div className="border-t border-line-soft">{children}</div> : null}
    </section>
  );
}

/**
 * "Expand all / Collapse all" toggle shown above a file-diff list. Shared by the
 * working-tree (git) and last-run (shadow) review views.
 */
export function ExpandCollapseAll({ allOpen, onToggle }: { allOpen: boolean; onToggle: () => void }) {
  const { t } = useTranslation("review");
  return (
    <div className="flex items-center justify-end">
      <button className="text-xs text-ink-muted transition-colors hover:text-ink" onClick={onToggle} type="button">
        {allOpen ? t("collapseAll") : t("expandAll")}
      </button>
    </div>
  );
}
