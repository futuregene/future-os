import { CopyButton } from "./CopyButton";
import { useCopyState } from "./useCopyState";

/**
 * A scrollable `<pre>` with a copy-to-clipboard button overlaid in the corner.
 * Used to show tool input/output and other raw text blocks.
 */
export function CopyablePre({
  className,
  maxHeightClassName,
  text,
}: {
  className?: string;
  maxHeightClassName: string;
  text: string;
}) {
  const { copiedKey, copy } = useCopyState();

  return (
    <div className={`relative ${className ?? ""}`}>
      <CopyButton
        copied={copiedKey !== null}
        onCopy={() => void copy(text)}
        variant="floating"
      />
      <pre className={`${maxHeightClassName} overflow-auto whitespace-pre-wrap rounded-md bg-surface-subtle p-2 pr-9 text-[11px] leading-4 text-ink-soft`}>
        <code>{text}</code>
      </pre>
    </div>
  );
}
