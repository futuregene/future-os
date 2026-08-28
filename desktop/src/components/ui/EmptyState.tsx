import { cn } from "../../lib/cn";

export function EmptyState({
  className,
  detail,
  title,
}: {
  className?: string;
  detail?: string;
  title: string;
}) {
  return (
    <div
      className={cn(
        "rounded-md border border-dashed border-line-soft bg-surface/60 p-4 text-center",
        className,
      )}
    >
      <div className="text-sm font-medium text-ink-soft">{title}</div>
      {detail
        ? (
            <div className="mt-1 text-xs leading-5 text-ink-muted">{detail}</div>
          )
        : null}
    </div>
  );
}
