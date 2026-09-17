import type { AgentMessage } from "@future-os/thread-projection";
import { useTranslation } from "react-i18next";
import { formatDuration } from "../../lib/date";
import { formatNumber } from "../../lib/format";
import { useNow } from "../../lib/useNow";

interface MessageMetaProps {
  message: AgentMessage;
}

/**
 * Faint per-reply footer: `time · N tokens`. While the reply streams it stays
 * visible and the elapsed time ticks live; once the run settles it stays visible
 * on the same right rail. Tokens are the real provider
 * usage, which only lands when the run ends.
 */
export function MessageMeta({ message }: MessageMetaProps) {
  const { t, i18n } = useTranslation("agent");
  const streaming = message.status === "streaming";

  // Tick `now` once a second so the live elapsed time advances while streaming;
  // frozen (no re-renders) once the run settles.
  const now = useNow(1000, streaming);

  const elapsedMs = streaming
    ? (typeof message.runStartedAt === "number" ? now - message.runStartedAt : null)
    : (message.durationMs ?? null);

  const outputTokens = message.outputTokens ?? 0;
  const usage = outputTokens > 0
    ? t("message.tokens", {
        formattedCount: formatNumber(outputTokens, i18n.language),
      })
    : null;
  const parts = [
    elapsedMs != null ? formatDuration(elapsedMs) : null,
    usage,
  ].filter((part): part is string => !!part);

  if (parts.length === 0)
    return null;

  return (
    <div className="select-none text-xs text-ink-muted">
      {parts.join(" · ")}
    </div>
  );
}
