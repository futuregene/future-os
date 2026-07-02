import { Check, Clipboard } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "../../lib/cn";

/**
 * Clipboard button showing a check while `copied`. `variant="floating"` is the
 * corner overlay used on code/preview blocks; `variant="inline"` sits in a row.
 * Pair with `useCopyState` for the transient flag + timer cleanup.
 */
export function CopyButton({
  className,
  copied,
  label,
  onCopy,
  variant = "inline",
}: {
  className?: string;
  copied: boolean;
  label?: string;
  onCopy: () => void;
  variant?: "floating" | "inline";
}) {
  const { t } = useTranslation("common");
  const resolvedLabel = label ?? t("copy");
  return (
    <button
      aria-label={resolvedLabel}
      className={cn(
        "inline-flex size-6 items-center justify-center rounded-md transition-colors",
        variant === "floating"
          ? "absolute right-1.5 top-1.5 bg-surface/90 text-ink-muted shadow-xs ring-1 ring-line-soft hover:text-ink"
          : "text-ink-muted hover:text-ink",
        className,
      )}
      onClick={onCopy}
      title={resolvedLabel}
      type="button"
    >
      {copied ? <Check className="size-3.5" /> : <Clipboard className="size-3.5" />}
    </button>
  );
}
