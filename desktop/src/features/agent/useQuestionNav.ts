import type { RefObject } from "react";
import type { MessageIndex, TranscriptMessage } from "./questionNav";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { isEditableTarget } from "../../lib/editableTarget";
import { hasKeyboardOwner } from "../../lib/keyboardOwners";
import { useCommittedRef } from "../../lib/useCommittedRef";
import { nextQuestion, previousQuestion, questionIndices } from "./questionNav";
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
 * How many frames to keep looking for the rows a fetched page brings.
 *
 * A page's rows are React's to draw and the browser's to lay out, so a press
 * whose target lives in the page is resolved from a later frame (see `pageUp`).
 * Bounded because a page that never arrives — a read that failed — must not
 * leave a frame loop running behind it.
 */
const PAGE_LANDING_FRAMES = 10;

export interface QuestionNavApi {
  /** The prompt ↑ would land on (the reader's previous question), or null. */
  previous: MessageIndex | null;
  /** The prompt ↓ would land on (the next question below), or null. */
  next: MessageIndex | null;
  /** Whether there is a previous question to jump to. */
  hasPrevious: boolean;
  /** Whether there is a later question below to jump to. */
  hasNext: boolean;
  /** Jump to the previous user question. */
  goToPrevious: () => void;
  /** Jump to the next user question. */
  goToNext: () => void;
}

/**
 * "↑ previous question / ↓ next question" for a transcript drawn as a scroll
 * container of `data-message-id` rows.
 *
 * One walk read two ways, over the same list: the reader's own **prompts**.
 * ↑ steps to the closest prompt above, ↓ to the closest prompt below, and the
 * replies in between are never a stop — a press asks "what else was asked", not
 * "move one item down the page", so a long answer costs one press to pass, not
 * one per screenful.
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
 * conversation an unmodified Page Up / Page Down is the same walk as the
 * matching button, so a reader who never looks for the control can still page
 * through the transcript. The bare arrows are left to the page — they are the
 * reader's ordinary line scroll, and stealing them would make every arrow press
 * a jump. A press that has nowhere to go is left alone rather than swallowed, so
 * a short transcript still gets its ordinary screenful scroll.
 *
 * The hook does not own the scroll container; the caller passes its ref and a
 * callback that disables sticky auto-follow, so a jump is not dragged back to
 * the bottom by a streaming reply.
 */
export function useQuestionNav<T extends TranscriptMessage>({
  messages,
  scrollRef,
  onTakeOver,
  hasOlderHistory = false,
  loadingOlder = false,
  loadOlder,
}: {
  messages: T[];
  scrollRef: RefObject<HTMLElement | null>;
  onTakeOver: () => void;
  /**
   * More transcript pages exist above the loaded window.
   *
   * A remote conversation is read a page at a time, and the loaded window is the
   * newest pages of one that continues above it — so the walk's own answer at the
   * window's top ("nothing above") is not the end of the conversation, and ↑
   * fetches the page rather than stalling there. Unset — a local transcript,
   * whose paging hangs off its own top edge — leaves the walk exactly as it was.
   */
  hasOlderHistory?: boolean;
  /** A page is already being fetched. */
  loadingOlder?: boolean;
  /** Ask for the next page up. Resolves once the page has been fetched. */
  loadOlder?: () => void | Promise<unknown>;
}): QuestionNavApi {
  const questions = useMemo(() => questionIndices(messages), [messages]);
  const questionsRef = useCommittedRef(questions);
  const onTakeOverRef = useCommittedRef(onTakeOver);
  const hasOlderRef = useCommittedRef(hasOlderHistory);
  const loadingOlderRef = useCommittedRef(loadingOlder);
  const loadOlderRef = useCommittedRef(loadOlder);
  /** A page press in flight: one press per page, however it is repeated. */
  const pagePendingRef = useRef(false);
  /** The frame loop looking for that page's rows, or null when none is armed. */
  const pageFrameRef = useRef<number | null>(null);
  const [offer, setOffer] = useState<{ previous: MessageIndex | null; next: MessageIndex | null }>({
    next: null,
    previous: null,
  });

  /**
   * The rows of the transcript, in render order. Both views draw one
   * `data-message-id` element per message, so the row index *is* the message
   * index the walk speaks in.
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
   * Where landing a rung would put the viewport: the prompt's own top edge
   * `JUMP_TOP_INSET_PX` below the viewport's, as far as the content allows.
   */
  const landingOffset = useCallback((index: MessageIndex, measured: readonly HTMLElement[]): number | null => {
    const container = scrollRef.current;
    if (!container)
      return null;
    const row = measured[index];
    if (!row)
      return null;
    const containerTop = container.getBoundingClientRect().top;
    const targetTop = row.getBoundingClientRect().top - containerTop + container.scrollTop;
    const max = Math.max(0, container.scrollHeight - container.clientHeight);
    return Math.min(Math.max(0, targetTop - JUMP_TOP_INSET_PX), max);
  }, [scrollRef]);

  /**
   * One step of the walk from the row the reader is reading: ↑ the closest
   * prompt above it, ↓ the closest prompt below — stepping on while a candidate
   * could not move the viewport (see the hook's note), and returning null when
   * the direction has nowhere left to go.
   *
   * `questions` is in transcript order (oldest first), so ↑ walks back through
   * the array and ↓ forward.
   */
  const step = useCallback((direction: "previous" | "next"): MessageIndex | null => {
    const container = scrollRef.current;
    if (!container)
      return null;
    const walk = questionsRef.current;
    const measured = rows();
    const here = container.scrollTop;
    const from = (index: MessageIndex) =>
      direction === "previous" ? previousQuestion(walk, index) : nextQuestion(walk, index);
    let candidate = from(readerRowIndex(measured));
    for (let skipped = 0; candidate !== null && skipped < MAX_SKIPPED_RUNGS; skipped += 1) {
      const offset = landingOffset(candidate, measured);
      if (offset !== null && Math.abs(offset - here) > LANDING_MOVES_PX)
        return candidate;
      candidate = from(candidate);
    }
    return null;
  }, [landingOffset, readerRowIndex, rows, questionsRef, scrollRef]);

  /** What the two buttons offer right now. */
  const publish = useCallback(() => {
    const previous = step("previous");
    const next = step("next");
    setOffer(current =>
      current.previous === previous && current.next === next
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
  }, [publish, questions, scrollRef]);

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

  /**
   * ↑ above the loaded window's top: fetch the page there, then land the press.
   *
   * A remote conversation is paged from the host, and the walk's own answer at
   * the window's top is "nothing above" — exactly where the reader must not be
   * stalled: the prompt they are looking for is the first one of the page above.
   * The landing therefore waits for the rows that page brings (they are a React
   * commit away), and the press is re-resolved against them — so it comes to rest
   * on the prompt the page begins with, measured, rather than on an offset
   * guessed before the browser had laid it out.
   */
  const pageUp = useCallback(() => {
    const request = loadOlderRef.current;
    if (
      !request
      || pagePendingRef.current
      || pageFrameRef.current !== null
      || loadingOlderRef.current
    ) {
      return;
    }
    pagePendingRef.current = true;
    // Hand the viewport over before the page lands: the transcript stops
    // following its tail, so the prepended rows are drawn above the reader
    // instead of arriving under a viewport pinned to the bottom.
    onTakeOverRef.current();
    const rowsBefore = rows().length;
    const settle = (frame: number) => {
      if (rows().length > rowsBefore) {
        pageFrameRef.current = null;
        navigate("previous");
        return;
      }
      if (frame >= PAGE_LANDING_FRAMES) {
        pageFrameRef.current = null;
        return;
      }
      pageFrameRef.current = window.requestAnimationFrame(() => settle(frame + 1));
    };
    const fetched = () => {
      pagePendingRef.current = false;
      settle(0);
    };
    try {
      // Waiting on the read itself is what makes the landing reliable: a page
      // from another machine can take longer than the frames this looks through.
      Promise.resolve(request()).then(fetched, fetched);
    }
    catch {
      fetched();
    }
  }, [loadOlderRef, loadingOlderRef, navigate, onTakeOverRef, rows]);

  const goToPrevious = useCallback(() => {
    // The target is resolved against the DOM now, as everywhere else in the walk;
    // only when there is nothing above the reader does history come into it.
    if (step("previous") !== null) {
      navigate("previous");
      return;
    }
    if (hasOlderRef.current)
      pageUp();
  }, [hasOlderRef, navigate, pageUp, step]);
  const goToNext = useCallback(() => navigate("next"), [navigate]);

  /**
   * The page keys, for as long as a transcript is on screen.
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
      if (direction === null)
        return;
      if (step(direction) === null) {
        // ↑ at the window's top is the button's own press, not a key the walk
        // cannot answer: the keys and the buttons are one walk, so the key pages
        // too. ↓ has no equivalent — there is no page newer than the window —
        // and is left to the page, which is the ordinary screenful scroll.
        if (direction === "previous" && hasOlderRef.current) {
          event.preventDefault();
          goToPrevious();
        }
        return;
      }
      event.preventDefault();
      navigate(direction);
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [goToPrevious, hasOlderRef, navigate, step]);

  // A landing that is still looking for its rows must not outlive the view: it
  // would move a viewport nobody asked it to touch.
  useEffect(
    () => () => {
      if (pageFrameRef.current !== null)
        window.cancelAnimationFrame(pageFrameRef.current);
      pageFrameRef.current = null;
      pagePendingRef.current = false;
    },
    [],
  );

  return {
    hasNext: offer.next !== null,
    // ↑ stays alive one step past the loaded walk when the host has more
    // history: the press fetches it and lands where the page begins.
    hasPrevious: offer.previous !== null || (hasOlderHistory && !loadingOlder),
    goToNext,
    goToPrevious,
    next: offer.next,
    previous: offer.previous,
  };
}
