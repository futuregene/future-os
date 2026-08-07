/** Centered neutral text shown while a preview is loading. */
export function PreviewNotice({ message }: { message: string }) {
  return (
    <div className="flex items-center justify-center p-8">
      <div className="rounded-md bg-surface/80 px-3 py-2 text-sm text-ink-muted shadow-panel">
        {message}
      </div>
    </div>
  );
}
