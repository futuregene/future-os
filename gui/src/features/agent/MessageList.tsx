import type { AgentMessage } from "./agentThreadTypes";
import { useCallback, useEffect, useRef, useState } from "react";
import { previousUserMessageBefore } from "./agentMessageFormatters";
import { MessageBlock } from "./MessageBlock";

/**
 * How long a row's controls linger after the pointer leaves it. Debounces the
 * hide so sweeping across the gap between rows (or briefly overshooting) doesn't
 * flicker — re-entering any row within the window cancels the pending hide.
 */
const HIDE_DELAY_MS = 200;

interface MessageListProps {
  messages: AgentMessage[];
  showThinking?: boolean;
  onContinue?: (message: AgentMessage) => void;
  onFork?: (message: AgentMessage) => void;
  onRetry?: (message: AgentMessage, source: AgentMessage) => void;
  workspaceId?: string | null;
  workspacePath?: string | null;
}

/**
 * Renders the thread and owns which message is hovered. Hover lives in React
 * state instead of CSS `:hover` because WKWebView (Tauri on macOS) drops
 * hover-exit events and end-of-transition repaints, which left several rows'
 * controls painted at once. A single `hoveredId` guarantees at most one row
 * shows its controls: a lost `pointerleave` is corrected by the next row's
 * `pointerover`, and leaving the list clears it outright.
 */
export function MessageList({ messages, showThinking, onContinue, onFork, onRetry, workspaceId, workspacePath }: MessageListProps) {
  const [hoveredId, setHoveredId] = useState<string | null>(null);
  const hideTimerRef = useRef<number | null>(null);

  const cancelPendingHide = useCallback(() => {
    if (hideTimerRef.current !== null) {
      window.clearTimeout(hideTimerRef.current);
      hideTimerRef.current = null;
    }
  }, []);

  useEffect(() => cancelPendingHide, [cancelPendingHide]);

  const handleHover = useCallback((id: string) => {
    cancelPendingHide();
    setHoveredId(id);
  }, [cancelPendingHide]);

  // Clear only if this row is still the hovered one — a `pointerover` from the
  // next row may already have replaced it by the time the delay elapses.
  const handleLeave = useCallback((id: string) => {
    cancelPendingHide();
    hideTimerRef.current = window.setTimeout(() => {
      hideTimerRef.current = null;
      setHoveredId(current => (current === id ? null : current));
    }, HIDE_DELAY_MS);
  }, [cancelPendingHide]);

  const handleListLeave = useCallback(() => {
    cancelPendingHide();
    hideTimerRef.current = window.setTimeout(() => {
      hideTimerRef.current = null;
      setHoveredId(null);
    }, HIDE_DELAY_MS);
  }, [cancelPendingHide]);

  // Only the LAST message can be a recovery target — computing the previous
  // user message for every row is O(n²) and causes scroll jank during
  // streaming (every 220 ms tick rescans the whole list).
  const lastUserMessage = messages.length > 0
    ? previousUserMessageBefore(messages, messages.length - 1)
    : null;

  return (
    <div className="space-y-5" onPointerLeave={handleListLeave}>
      {messages.map((message, index) => {
        const isLast = index === messages.length - 1;
        const recoverySource = isLast ? lastUserMessage : null;
        return (
          <MessageBlock
            key={message.id}
            message={message}
            hovered={hoveredId === message.id}
            isLast={isLast}
            recoverySource={recoverySource}
            showThinking={showThinking}
            workspaceId={workspaceId}
            workspacePath={workspacePath}
            onContinue={onContinue}
            onFork={onFork}
            onHover={handleHover}
            onLeave={handleLeave}
            onRetry={onRetry}
          />
        );
      })}
    </div>
  );
}
