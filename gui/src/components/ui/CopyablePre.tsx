import { Check, Clipboard } from "lucide-react";
import { useState } from "react";
import { copyText } from "../../lib/clipboard";

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
  const [copied, setCopied] = useState(false);

  async function handleCopy() {
    await copyText(text);
    setCopied(true);
    window.setTimeout(setCopied, 1400, false);
  }

  return (
    <div className={`relative ${className ?? ""}`}>
      <button
        aria-label="Copy content"
        className="absolute right-1.5 top-1.5 inline-flex size-6 items-center justify-center rounded-md bg-white/90 text-ink-muted shadow-sm ring-1 ring-line-soft transition-colors hover:text-ink"
        onClick={() => void handleCopy()}
        title="Copy"
        type="button"
      >
        {copied ? <Check className="size-3.5" /> : <Clipboard className="size-3.5" />}
      </button>
      <pre className={`${maxHeightClassName} overflow-auto whitespace-pre-wrap rounded-md bg-surface-subtle p-2 pr-9 text-[11px] leading-4 text-ink-soft`}>
        <code>{text}</code>
      </pre>
    </div>
  );
}
