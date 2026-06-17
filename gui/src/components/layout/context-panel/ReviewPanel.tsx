import type { GitReview, GitReviewFile } from "../../../integrations/storage/types";
import { ChevronDown, ChevronRight, FileDiff, GitBranch } from "lucide-react";
import { useState } from "react";
import { EmptyState } from "./ContextEmptyState";

export function ReviewPanel({ review }: { review: GitReview | null }) {
  if (!review?.isGitWorkspace) {
    return <EmptyState title="No Git workspace" detail="Review is available for Git-backed workspaces." />;
  }

  if (review.files.length === 0) {
    return (
      <div className="space-y-3">
        <ReviewHeader review={review} />
        <EmptyState title="No changes" detail="Working tree changes will appear here." />
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <ReviewHeader review={review} />
      <div className="space-y-3">
        {review.files.map(file => <GitFileDiff file={file} key={file.path} />)}
      </div>
    </div>
  );
}

function ReviewHeader({ review }: { review: GitReview }) {
  return (
    <div className="space-y-2 border-b border-line-soft pb-3">
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
    </div>
  );
}

function GitFileDiff({ file }: { file: GitReviewFile }) {
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
      {open
        ? (
            <div className="border-t border-line-soft">
              {file.diff
                ? <DiffBlock diff={file.diff} />
                : <div className="px-3 py-3 text-xs text-ink-muted">No textual diff available.</div>}
            </div>
          )
        : null}
    </section>
  );
}

function DiffBlock({ diff }: { diff: string }) {
  const rows = diffRows(diff);

  return (
    <div className="max-h-[70vh] overflow-auto bg-white font-mono text-[12px] leading-5">
      {rows.map(row => (
        <DiffLine key={row.key} line={row.line} lineNumber={row.lineNumber} />
      ))}
    </div>
  );
}

function DiffLine({ line, lineNumber }: { line: string; lineNumber: number }) {
  const kind = diffLineKind(line);
  const content = line.length === 0 ? " " : line;

  return (
    <div className={diffLineClass(kind)}>
      <span className="w-12 shrink-0 select-none border-r border-white/70 px-2 text-right text-ink-muted">
        {kind === "meta" ? "" : lineNumber}
      </span>
      <code className="min-w-0 flex-1 whitespace-pre-wrap break-words px-3">{content}</code>
    </div>
  );
}

function diffRows(diff: string) {
  const seen = new Map<string, number>();
  let lineNumber = 0;
  return diff
    .split("\n")
    .filter(line => !line.startsWith("diff --git ") && !line.startsWith("index "))
    .map((line) => {
      const count = (seen.get(line) ?? 0) + 1;
      seen.set(line, count);
      if (diffLineKind(line) !== "meta") {
        lineNumber += 1;
      }
      return {
        key: `${count}:${line}`,
        line,
        lineNumber,
      };
    });
}

function diffLineKind(line: string) {
  if (line.startsWith("@@") || line.startsWith("---") || line.startsWith("+++") || line.startsWith("new file")) {
    return "meta";
  }
  if (line.startsWith("+")) {
    return "add";
  }
  if (line.startsWith("-")) {
    return "delete";
  }
  return "context";
}

function diffLineClass(kind: string) {
  const base = "flex min-w-0 border-l-2";
  switch (kind) {
    case "add":
      return `${base} border-green-500 bg-green-50 text-green-900`;
    case "delete":
      return `${base} border-red-500 bg-red-50 text-red-900`;
    case "meta":
      return `${base} border-transparent bg-surface-subtle text-ink-muted`;
    default:
      return `${base} border-transparent text-ink-soft`;
  }
}
