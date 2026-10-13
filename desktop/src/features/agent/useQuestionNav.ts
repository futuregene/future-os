import type { RefObject } from "react";
import type { QuestionMessage } from "./questionNav";
import { useCallback, useMemo } from "react";
import { useCommittedRef } from "../../lib/useCommittedRef";
import { buildQuestionTurns, findQuestionTurns, targetRowForQuestion } from "./questionNav";

/** Offset from the top of the viewport when jumping to a question. */
const JUMP_TOP_INSET_PX = 16;

export interface QuestionNavApi {
  /** Whether there is a previous question to jump to. */
  hasPrevious: boolean;
  /** Whether there is a next question to jump to. */
  hasNext: boolean;
  /** Jump to the previous user question. */
  goToPrevious: () => void;
  /** Jump to the next user question. */
  goToNext: () => void;
}

/**
 * Provide "previous/next question" navigation for a desktop-style transcript
 * that uses a scroll container with `data-message-id` rows.
 *
 * The hook does not own the scroll container; it needs the caller to pass the
 * container ref and a callback that disables sticky auto-follow so a jump is not
 * pulled back to the bottom by a streaming reply.
 */
export function useQuestionNav<T extends QuestionMessage>({
  messages,
  scrollRef,
  onTakeOver,
}: {
  messages: T[];
  scrollRef: RefObject<HTMLElement | null>;
  onTakeOver: () => void;
}): QuestionNavApi {
  const turns = useMemo(() => buildQuestionTurns(messages), [messages]);
  const onTakeOverRef = useCommittedRef(onTakeOver);

  const topVisibleMessageIndex = useCallback((): number => {
    const container = scrollRef.current;
    if (!container)
      return 0;

    const rows = Array.from(container.querySelectorAll<HTMLElement>("[data-message-id]"));
    if (rows.length === 0)
      return 0;

    const containerRect = container.getBoundingClientRect();
    let bestIndex = 0;
    let bestTop = Number.POSITIVE_INFINITY;
    rows.forEach((row, rowIndex) => {
      const rect = row.getBoundingClientRect();
      const top = rect.top - containerRect.top;
      // The first row whose top is at or just below the container's top edge.
      if (top >= -1 && top < bestTop) {
        bestIndex = rowIndex;
        bestTop = top;
      }
    });

    // jsdom reports every row at top === 0, so the loop leaves bestIndex at 0.
    // Use scroll position as a tie-breaker: when the user has scrolled at all,
    // treat the viewport as showing the last row; otherwise stay at the top.
    if (bestTop === 0) {
      return container.scrollTop > 0 ? rows.length - 1 : 0;
    }

    return bestIndex;
  }, [scrollRef]);

  const jumpToTurn = useCallback((turnIndex: number) => {
    const container = scrollRef.current;
    if (!container || turnIndex < 0 || turnIndex >= turns.length)
      return;

    const rows = Array.from(container.querySelectorAll<HTMLElement>("[data-message-id]"));
    const lastIndex = Math.max(0, rows.length - 1);
    const target = targetRowForQuestion(turns, turnIndex, lastIndex);
    const targetRow = rows[target];
    if (!targetRow)
      return;

    onTakeOverRef.current();

    const containerRect = container.getBoundingClientRect();
    const rowRect = targetRow.getBoundingClientRect();
    const offsetTop = rowRect.top - containerRect.top + container.scrollTop - JUMP_TOP_INSET_PX;
    container.scrollTop = Math.max(0, offsetTop);
  }, [scrollRef, turns, onTakeOverRef]);

  const navigate = useCallback((direction: "previous" | "next") => {
    const top = topVisibleMessageIndex();
    const { previous, next } = findQuestionTurns(turns, top);
    const target = direction === "previous" ? previous : next;
    if (target === null)
      return;
    jumpToTurn(target);
  }, [turns, topVisibleMessageIndex, jumpToTurn]);

  const goToPrevious = useCallback(() => navigate("previous"), [navigate]);
  const goToNext = useCallback(() => navigate("next"), [navigate]);

  const top = topVisibleMessageIndex();
  const { previous, next } = findQuestionTurns(turns, top);

  return {
    hasPrevious: previous !== null,
    hasNext: next !== null,
    goToPrevious,
    goToNext,
  };
}
