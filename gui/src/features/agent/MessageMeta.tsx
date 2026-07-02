import type { AgentMessage } from "./agentThreadTypes";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "../../lib/cn";
import { formatDuration } from "../../lib/date";

interface MessageMetaProps {
  message: AgentMessage;
  /**
   * Whether the row is hovered. Passed down as state (from MessageList's single
   * `hoveredId`) rather than styled via CSS `:hover` — WKWebView drops
   * hover-exit repaints, which left metas of several rows painted at once.
   */
  visible: boolean;
}

/**
 * Faint per-reply footer: `time · N tokens`. While the reply streams it stays
 * visible and the elapsed time ticks live; once the run settles it is hidden and
 * revealed only while the message row is hovered. Tokens are the real provider
 * usage, which only lands when the run ends.
 */
export function MessageMeta({ message, visible }: MessageMetaProps) {
  const { t } = useTranslation("agent");
  const streaming = message.status === "streaming";

  // Tick `now` once a second so the live elapsed time advances while streaming.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!streaming)
      return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [streaming]);

  const elapsedMs = streaming
    ? (typeof message.runStartedAt === "number" ? now - message.runStartedAt : null)
    : (message.durationMs ?? null);

  const tokens = message.outputTokens ?? 0;
  const parts = [
    elapsedMs != null ? formatDuration(elapsedMs) : null,
    tokens > 0 ? t("message.tokens", { count: tokens, formattedCount: tokens.toLocaleString("en") }) : null,
  ].filter((part): part is string => !!part);

  if (parts.length === 0)
    return null;

  return (
    <div
      // Own compositor layer (`will-change-[opacity]`): WKWebView drops in-flow
      // repaints until a window resize (tauri#12800 family), so the hide and its
      // fade must run on the compositor, never through a repaint — see CopyButton
      // in MessageBlock for the full story.
      className={cn(
        "select-none text-xs text-ink-muted will-change-[opacity] transition-opacity duration-200",
        streaming || visible ? "opacity-100" : "opacity-0",
      )}
    >
      {parts.join(" · ")}
    </div>
  );
}
