import { TriangleAlert } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "../../../lib/cn";
import { formatNumber } from "../../../lib/format";

export function StatusDivider({ label, pulsing = false, title, warning = false }: {
  label: string;
  pulsing?: boolean;
  title?: string;
  warning?: boolean;
}) {
  return (
    <div
      aria-label={label}
      className={cn("flex select-none items-center gap-3 py-1", pulsing && "animate-pulse")}
      role="status"
      title={title}
    >
      <span className="h-px flex-1 bg-line" />
      <span className="flex items-center gap-1 whitespace-nowrap text-xs text-ink-muted">
        {warning ? <TriangleAlert aria-hidden="true" className="size-3 shrink-0" /> : null}
        {label}
      </span>
      <span className="h-px flex-1 bg-line" />
    </div>
  );
}

/** Inline divider marking where the agent auto-compacted the conversation. */
export function CompactionDivider({
  tokensBefore,
  tokensAfter,
  status = "completed",
  error,
  trigger,
}: {
  tokensBefore?: number;
  tokensAfter?: number;
  status?: "running" | "completed" | "failed";
  error?: string;
  trigger?: string;
}) {
  const { t, i18n } = useTranslation("agent");
  const manual = trigger === "manual";
  // Both counts come from the committed checkpoint: `tokensBefore` is what the
  // turn was about to send, `tokensAfter` the agent's estimate of the next
  // turn's prompt. They are only meaningful as a pair — a lone `tokensBefore`
  // (released journal, legacy row) keeps the older label.
  const delta = tokensBefore && tokensBefore > 0 && tokensAfter && tokensAfter > 0
    ? {
        before: formatNumber(tokensBefore, i18n.language),
        after: formatNumber(tokensAfter, i18n.language),
      }
    : null;
  const label = status === "running"
    ? manual ? t("message.manuallyCompacting") : t("message.compacting")
    : status === "failed"
      ? manual ? t("message.manualCompactionFailed") : t("message.compactionFailed")
      : delta
        ? t(
            manual
              ? "message.manuallyCompactedTokensDelta"
              : "message.compactedTokensDelta",
            delta,
          )
        : manual
          ? t("message.manuallyCompacted")
          : tokensBefore && tokensBefore > 0
            ? t("message.compactedTokens", {
                formattedCount: formatNumber(tokensBefore, i18n.language),
              })
            : t("message.compacted");
  return (
    <StatusDivider
      label={label}
      pulsing={status === "running"}
      title={error}
      warning={status === "failed"}
    />
  );
}
