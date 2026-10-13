import type { AgentMessage } from "@future-os/thread-projection";

/** A row index in the rendered message list, from 0 (oldest) to length - 1 (newest). */
export type MessageIndex = number;

/** Information about a "turn" — a user question and the reply that follows it. */
export interface QuestionTurn {
  /** Index of the user message within the visible list. */
  question: MessageIndex;
  /** Id of the user message, for stable identification / test expectations. */
  messageId: string;
}

/**
 * Build the list of user questions, in the order they appear in the transcript.
 * Only user messages count; assistant messages, compaction markers and tool
 * rows are ignored. This matches the semantics of mobile's `questionIndices`.
 */
export function buildQuestionTurns(messages: readonly AgentMessage[]): QuestionTurn[] {
  const turns: QuestionTurn[] = [];
  messages.forEach((message, index) => {
    if (message.role === "user") {
      turns.push({ question: index, messageId: message.id });
    }
  });
  return turns;
}

/**
 * Find the previous and next user question relative to the current topmost
 * visible row. A question that is partially above the viewport is still the
 * current question; the *previous* question is the one before it.
 *
 * @returns indices into `turns` (not message indices), or `null` if none.
 */
export function findQuestionTurns(
  turns: readonly QuestionTurn[],
  topVisibleIndex: number | null,
): {
  current: number | null;
  previous: number | null;
  next: number | null;
} {
  if (turns.length === 0 || topVisibleIndex === null) {
    return { current: null, previous: null, next: null };
  }

  let current: number | null = null;
  for (let i = turns.length - 1; i >= 0; i -= 1) {
    if (turns[i]!.question <= topVisibleIndex) {
      current = i;
      break;
    }
  }

  if (current === null) {
    // The viewport starts above every question; the first question is below.
    return { current: null, previous: null, next: 0 };
  }

  const previous = current > 0 ? current - 1 : null;
  // Treat the last question as having no next, even if there is still a reply
  // below it, so the button becomes disabled at the conversation end.
  const next = current < turns.length - 1 ? current + 1 : null;
  return { current, previous, next };
}

/**
 * Determine which row to scroll to so the target question lands in a
 * comfortable reading position near the top of the viewport.
 */
export function targetRowForQuestion(
  turns: readonly QuestionTurn[],
  turnIndex: number,
  lastMessageIndex: number,
): number {
  const turn = turns[turnIndex];
  if (!turn)
    return 0;
  // Prefer the question itself. If the question is the very last message,
  // fall back to the latest message so the scroll container can reach it.
  return Math.min(turn.question, lastMessageIndex);
}
