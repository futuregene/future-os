import type { HistoryAttachment, TimelineItem } from "../../../remote/types";

/**
 * Fetch one tool call's display target after the fact. A lean history page
 * omits a shell call's arguments (the command is the page's largest unread
 * payload), so a row with no target asks for it when it is opened. Implemented
 * by the screen — the row itself stays free of transport.
 */
export type ToolTargetResolver = (toolCallId: string, runId: string, fusion?: { stepId?: string; offset?: number }) => Promise<string | null>;

/**
 * Fetch one reasoning block's text after the fact. The lean lanes send a
 * reasoning row's boundaries and not its body (the reply's largest unopened
 * payload), so opening the row is what asks for it. `index` is the block's
 * position among the run's reasoning blocks — the order the reply renders them
 * in — and `settled` says whether the reply has stopped (only a settled read is
 * worth caching; a block still being written keeps growing).
 */
export type ReasoningResolver = (runId: string, index: number, settled?: boolean) => Promise<string | null>;

/**
 * What a rendered row uses: its own segment in, that block's text out. Built by
 * the reply from `ReasoningResolver` + the reply's slice order, so the rows
 * below stay free of run identity and indices.
 */
export type ReasoningLoader = (segmentId: string) => Promise<string | null>;

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
  /** Fetches a reasoning row's withheld body when the reader opens it. */
  onResolveReasoning?: ReasoningResolver;
}
