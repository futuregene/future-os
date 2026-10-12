import type { HistoryAttachment, TimelineItem } from "../../../remote/types";

/**
 * Fetch one tool call's display target after the fact. A lean history page
 * omits a shell call's arguments (the command is the page's largest unread
 * payload), so a row with no target asks for it when it is opened. Implemented
 * by the screen — the row itself stays free of transport.
 */
export type ToolTargetResolver = (toolCallId: string, runId: string) => Promise<string | null>;

export interface TimelineCardProps {
  item: TimelineItem;
  isLatestAssistant?: boolean;
  onOpenAttachment?(attachment: HistoryAttachment): void;
  onOpenFile?(path: string): void;
  /** Opens the conversation a `#` reference in a sent message points at. */
  onOpenSession?(sessionId: string): void;
  onRetry?(item: TimelineItem): void;
  onContinue?(item: TimelineItem): void;
  /** Fork the conversation at this reply's turn (desktop Fork parity). */
  onFork?(item: TimelineItem): void;
  onResolveToolTarget?: ToolTargetResolver;
}
