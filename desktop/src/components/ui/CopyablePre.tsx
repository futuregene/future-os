import { cn } from "../../lib/cn";
import { CopyButton } from "./CopyButton";
import { useCopyState } from "./useCopyState";

/**
 * A scrollable `<pre>` with a copy-to-clipboard button overlaid in the corner.
 * Used to show tool input/output and other raw text blocks.
 *
 * `fill` makes the block shrink-to-fit inside a flex column: it grows with its
 * content but, once the column runs out of room, caps at the available height
 * and scrolls internally instead of forcing a fixed height. Long unbroken
 * tokens wrap rather than triggering a horizontal scrollbar.
 */
export function CopyablePre({
  className,
  fill = false,
  maxHeightClassName,
  text,
}: {
  className?: string;
  fill?: boolean;
  maxHeightClassName: string;
  text: string;
}) {
  const { copiedKey, copy } = useCopyState();

  return (
    <div className={cn("relative", fill && "flex min-h-0 flex-col", className)}>
      <CopyButton
        copied={copiedKey !== null}
        onCopy={() => void copy(text)}
        variant="floating"
      />
      <pre className={cn(
        fill ? "min-h-0" : maxHeightClassName,
        "overflow-auto whitespace-pre-wrap wrap-break-word rounded-md bg-surface-subtle p-2 pr-9 text-[11px] leading-4 text-ink-soft",
      )}
      >
        <code>{text}</code>
      </pre>
    </div>
  );
}
