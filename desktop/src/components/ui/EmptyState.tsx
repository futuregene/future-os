export function EmptyState({ detail, title }: { detail?: string; title: string }) {
  return (
    <div className="rounded-md border border-dashed border-line-soft bg-surface/60 p-4 text-center">
      <div className="text-sm font-medium text-ink-soft">{title}</div>
      {detail ? <div className="mt-1 text-xs leading-5 text-ink-muted">{detail}</div> : null}
    </div>
  );
}
