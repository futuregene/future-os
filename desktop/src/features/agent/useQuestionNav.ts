import type { RefObject } from "react";
import type { MessageIndex, NavRung, TranscriptMessage } from "./questionNav";
import { useCallback, useEffect, useMemo, useState } from "react";
import { isEditableTarget } from "../../lib/editableTarget";
import { hasKeyboardOwner } from "../../lib/keyboardOwners";
import { useCommittedRef } from "../../lib/useCommittedRef";
import { buildNavRungs, nextRung, previousQuestion, sameRung } from "./questionNav";
import { navDirectionForKey } from "./questionNavKeys";

/**
 * How far below the viewport's top edge a landed row comes to rest.
 *
 * The desktop transcript scrolls normally (chronological rows in a plain
 * `overflow-auto` container), so a landing is just "put the target this far
 * below the edge" — no inverted-list geometry to spend.
 */
const JUMP_TOP_INSET_PX = 16;
/**
 * How much a landing has to move the viewport to count as a step. Under this,
 * the target is already where the press would put it — and the press walks on
 * instead (see `step`).
 */
const LANDING_MOVES_PX = 2;
/** A bound on walking past unreachable rungs, so a stuck layout cannot loop. */
const MAX_SKIPPED_RUNGS = 200;

/**
 * The marker the reply row puts on the block a `replyTail` rung lands on (see
 * `replyTailBlock` and `MessageBlock`). The rung only names the *row*; the
 * anchor is what the reader actually came to read.
 */
const REPLY_TAIL_ATTR = "data-reply-tail";

export interface QuestionNavApi {
  /** The rung ↑ would land on (the reader's previous prompt), or null. */
  previous: NavRung | null;
  /** The rung ↓ would land on (the next stop below), or null. */
  next: NavRung | null;
  /** Whether there is a previous question to jump to. */
  hasPrevious: boolean;
  /** Whether there is anything newer below to jump to. */
  hasNext: boolean;
  /** Jump to the previous user question. */
  goToPrevious: () => void;
  /** Jump to the next stop below — a prompt or the end of a reply. */
  goToNext: () => void;
}

/**
 * "↑ previous question / ↓ next stop" for a transcript drawn as a scroll
 * container of `data-message-id` rows.
 *
 * The two directions are one ladder read two ways: ↑ steps to the reader's
 * previous **prompt**, skipping whatever answer lies between, while ↓ takes the
 * very next rung below — so pressing it walks the conversation as it was
 * written, prompt, reply, prompt, reply… A reply is reached at its last output
 * slice rather than at its first line, because a reply opens with the work it
 * ran (see `replyTailBlock`): the row is lifted past its own top to the anchor
 * block the row marked, which is the thing the press was for.
 *
 * Where the walk starts is the row the reader is reading — the topmost one whose
 * middle is on screen, so a sliver of the previous turn under the top edge does
 * not steal the position from the reply that fills the view (see
 * `readerRowIndex`) — read from the rows' own rects. The DOM is the truth here,
 * and both callers render exactly the messages this hook was given, in that
 * order.
 *
 * **A press lands on the first stop it can actually reach.** Not every rung can
 * be landed on: at the end of a conversation the reader's own last prompt is
 * already under the top edge, and aiming at it again asks for an offset past the
 * end of the transcript — the viewport would not move, and the button would look
 * dead. The walk therefore steps on to the next rung in the same direction when
 * one cannot move the viewport, and reports the direction unavailable when none
 * can. The offer is resolved through the same function as the press, so an
 * enabled button always does something.
 *
 * The offer is kept up to date by listening to the container's own scroll rather
 * than by hoping the parent re-renders: an enabled state that lags a scroll is a
 * button that looks dead or lies.
 *
 * **The pair is also on the keyboard** (see `questionNavKeys`): inside the
 * conversation an unmodified ↑/↓ is the same walk as the matching button, so a
 * reader who never looks for the control can still page through the transcript.
 * A press that has nowhere to go is left alone rather than swallowed, so a short
 * transcript still gets its ordinary line scroll.
 *
 * The hook does not own the scroll container; the caller passes its ref and a
 * callback that disables sticky auto-follow, so a jump is not dragged back to
 * the bottom by a streaming reply.
 */
export function useQuestionNav<T extends TranscriptMessage>({
  messages,
  scrollRef,
  onTakeOver,
}: {
  messages: T[];
  scrollRef: RefObject<HTMLElement | null>;
  onTakeOver: () => void;
}): QuestionNavApi {
  const rungs = useMemo(() => buildNavRungs(messages), [messages]);
  const rungsRef = useCommittedRef(rungs);
  const onTakeOverRef = useCommittedRef(onTakeOver);
  const [offer, setOffer] = useState<{ previous: NavRung | null; next: NavRung | null }>({
    next: null,
    previous: null,
  });

  /**
   * The rows of the transcript, in render order. Both views draw one
   * `data-message-id` element per message, so the row index *is* the message
   * index the ladder speaks in.
   */
  const rows = useCallback((): HTMLElement[] => {
    const container = scrollRef.current;
    if (!container)
      return [];
    return Array.from(container.querySelectorAll<HTMLElement>("[data-message-id]"));
  }, [scrollRef]);

  /**
   * The row the reader is reading: the topmost row whose own middle is on
   * screen.
   *
   * The row at the top edge is not always that one: a sliver of the previous
   * turn can sit under the edge while the reply they are reading fills the rest,
   * and reading the sliver as "where I am" skips the row below it — at the end
   * of a conversation that is the press that has to reach the prompt of the turn
   * on screen. A row the reader is inside of (one taller than the viewport, with
   * its middle on screen) still counts, which is the case this must not lose.
   *
   * Falls back to the topmost row with any part on screen when no middle is (a
   * row taller than two viewports, or an unmeasured layout), and to the first
   * row when nothing reports itself at all.
   */
  const readerRowIndex = useCallback((measured: readonly HTMLElement[]): MessageIndex => {
    const container = scrollRef.current;
    if (!container || measured.length === 0)
      return 0;
    const box = container.getBoundingClientRect();
    let partlyVisible: MessageIndex | null = null;
    for (let index = 0; index < measured.length; index += 1) {
      const rect = measured[index]!.getBoundingClientRect();
      if (rect.bottom <= box.top + 1)
        continue;
      if (rect.top >= box.bottom - 1)
        break;
      if (partlyVisible === null)
        partlyVisible = index;
      const middle = (rect.top + rect.bottom) / 2;
      if (middle >= box.top && middle <= box.bottom)
        return index;
    }
    return partlyVisible ?? 0;
  }, [scrollRef]);

  /**
   * Where landing a rung would put the viewport: the target's own top edge
   * `JUMP_TOP_INSET_PX` below the viewport's, as far as the content allows.
   *
   * A `replyTail` rung aims past the row's start at the block inside it that
   * still has output — the press asked for the reply's answer, not for the
   * reasoning rows it opened with. A row that has not marked one (plain content,
   * or a reply the renderer found no output block in) keeps the row's start,
   * which is all there is to aim at.
   */
  const landingOffset = useCallback((rung: NavRung, measured: readonly HTMLElement[]): number | null => {
    const container = scrollRef.current;
    if (!container)
      return null;
    const row = measured[rung.index];
    if (!row)
      return null;
    const anchor = rung.kind === "replyTail" ? row.querySelector<HTMLElement>(`[${REPLY_TAIL_ATTR}]`) : null;
    const target = anchor ?? row;
    const containerTop = container.getBoundingClientRect().top;
    const targetTop = target.getBoundingClientRect().top - containerTop + container.scrollTop;
    const max = Math.max(0, container.scrollHeight - container.clientHeight);
    return Math.min(Math.max(0, targetTop - JUMP_TOP_INSET_PX), max);
  }, [scrollRef]);

  /**
   * One step of the walk from the row the reader is reading: ↑ the closest
   * prompt above it, ↓ the very next rung below — stepping on while a candidate
   * could not move the viewport (see the hook's note), and returning null when
   * the direction has nowhere left to go.
   *
   * Rungs are in transcript order (oldest first), so ↑ walks back through the
   * array and ↓ forward.
   */
  const step = useCallback((direction: "previous" | "next"): NavRung | null => {
    const container = scrollRef.current;
    if (!container)
      return null;
    const ladder = rungsRef.current;
    const measured = rows();
    const here = container.scrollTop;
    const from = (index: MessageIndex) =>
      direction === "previous" ? previousQuestion(ladder, index) : nextRung(ladder, index);
    let candidate = from(readerRowIndex(measured));
    for (let skipped = 0; candidate !== null && skipped < MAX_SKIPPED_RUNGS; skipped += 1) {
      const offset = landingOffset(candidate, measured);
      if (offset !== null && Math.abs(offset - here) > LANDING_MOVES_PX)
        return candidate;
      candidate = from(candidate.index);
    }
    return null;
  }, [landingOffset, readerRowIndex, rows, rungsRef, scrollRef]);

  /** What the two buttons offer right now. */
  const publish = useCallback(() => {
    const previous = step("previous");
    const next = step("next");
    setOffer(current =>
      sameRung(current.previous, previous) && sameRung(current.next, next)
        ? current
        : { next, previous },
    );
  }, [step]);

  // Keep the offer honest while the reader scrolls: the enabled state of a
  // direction is a fact about where the viewport is, and the scroll listener is
  // the only thing that sees a movement the parent does not re-render for.
  useEffect(() => {
    const container = scrollRef.current;
    publish();
    if (!container)
      return;
    container.addEventListener("scroll", publish, { passive: true });
    return () => container.removeEventListener("scroll", publish);
  }, [publish, rungs, scrollRef]);

  /**
   * Resolve the target at the press rather than from the published offer: the
   * DOM can have moved (a streamed delta, a paged-in row, a scroll the parent
   * has not re-rendered for) between the render that produced the offer and the
   * click.
   */
  const navigate = useCallback((direction: "previous" | "next") => {
    const container = scrollRef.current;
    if (!container)
      return;
    const target = step(direction);
    if (target === null)
      return;
    const offset = landingOffset(target, rows());
    if (offset === null)
      return;
    container.scrollTop = offset;
    // Hand the viewport over *after* the move, within the same tick: the
    // handover stops the list following the tail (so a streaming reply cannot
    // pull the reader back), and it also records where the reader now is — a
    // handover taken before the move records the position the press is leaving,
    // and the app's own next settle faithfully drags the jump back to it.
    onTakeOverRef.current();
    // The buttons' own offer follows the press: a landing that could not move
    // the viewport produces no scroll event to publish from.
    publish();
  }, [landingOffset, onTakeOverRef, publish, rows, scrollRef, step]);

  const goToPrevious = useCallback(() => navigate("previous"), [navigate]);
  const goToNext = useCallback(() => navigate("next"), [navigate]);

  /**
   * The arrow keys, for as long as a transcript is on screen.
   *
   * The direction is resolved against the live DOM — the same `step` a press
   * takes, right now — rather than against the published offer: the DOM moves
   * between a keydown and the render that would follow it, and a key that
   * resolves to nothing must be left to whatever else wanted it.
   */
  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      const direction = navDirectionForKey(event, {
        editable: isEditableTarget(event.target),
        owned: hasKeyboardOwner(),
      });
      if (direction === null || step(direction) === null)
        return;
      event.preventDefault();
      navigate(direction);
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [navigate, step]);

  return {
    hasNext: offer.next !== null,
    hasPrevious: offer.previous !== null,
    goToNext,
    goToPrevious,
    next: offer.next,
    previous: offer.previous,
  };
}
