import type { AgentMessage } from "@future-os/thread-projection";
import { useTranslation } from "react-i18next";
import { cn } from "../../lib/cn";
import { formatDuration } from "../../lib/date";
import { formatNumber } from "../../lib/format";
import { useNow } from "../../lib/useNow";

interface MessageMetaProps {
  message: AgentMessage;
  hovered: boolean;
}

/**
 * Hover-only reply stats: `time · N tokens`. The elapsed time ticks live while
 * streaming. Tokens are the real provider usage, which lands when the run ends.
 * Keep the stats mounted so hover changes never shift the footer layout.
 */
export function MessageMeta({ message, hovered }: MessageMetaProps) {
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
    <div className={cn(
      "select-none text-xs text-ink-muted will-change-[opacity] transition-opacity duration-200",
      hovered ? "opacity-100" : "pointer-events-none opacity-0",
    )}
    >
      {parts.join(" · ")}
    </div>
  );
}
