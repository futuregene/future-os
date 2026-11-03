import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type RefObject,
} from "react";
import type {
  FlatList,
  NativeScrollEvent,
  NativeSyntheticEvent,
  FlatListProps,
} from "react-native";
import type { TimelineItem } from "../../remote/types";
import { spacing } from "../../theme/tokens";
import {
  nextQuestion,
  previousQuestion,
  questionIndices,
} from "./questionNav";

/** The offset `useChatScroll` already treats as "at the latest". */
const AT_LATEST_THRESHOLD_PX = 32;
/**
 * The geometry every landing aims at: the landed row sits this far below the
 * viewport's top edge.
 *
 * The list is inverted, so `viewPosition: 1` aligns the target's *cell* with
 * that edge — and an inverted cell lays its separator out above its row
 * (`column-reverse`), which is the list's own inter-row gap. Aligning the cell
 * therefore already leaves `ROW_GAP` above the row, so what a jump passes as
 * `viewOffset` is the difference. Exported so a test can pin the wiring; the
 * harness measures the delivered inset itself.
 */
const QUESTION_TOP_INSET = 12;
const ROW_GAP = spacing.md;
export const JUMP_VIEW_OFFSET = ROW_GAP - QUESTION_TOP_INSET;
/**
 * A jump is only as exact as VirtualizedList's knowledge of the target row. A
 * row outside the rendered window is estimated from the average row height, so
 * the jump lands approximately, renders the target, and is re-issued against the
 * freshly measured frame.
 */
const REALIGN_MS = 320;
/** Bounded so a target that never enters the render window cannot loop. */
const MAX_JUMP_ATTEMPTS = 4;

type TimelineListProps = FlatListProps<TimelineItem>;
type ViewabilityConfig = NonNullable<TimelineListProps["viewabilityConfig"]>;
type ViewabilityConfigCallbackPair = NonNullable<TimelineListProps["viewabilityConfigCallbackPairs"]>[number];
export type ViewToken = Parameters<NonNullable<TimelineListProps["onViewableItemsChanged"]>>[0]["viewableItems"][number];

const PARTLY_VISIBLE: ViewabilityConfig = { itemVisiblePercentThreshold: 1 };

export interface QuestionNavApi {
  /**
   * The row ↑ would land on — the reader's previous prompt — or null. Offered
   * from the tail as well, where it is the prompt of the turn on screen.
   */
  previous: number | null;
  /**
   * The row ↓ would land on — the next prompt below — or null. Only once the
   * reader has left the tail: at the tail there is nothing newer below.
   */
  next: number | null;
  /** Whether ↑ has anywhere to go — a rung, or a page of history to pull. */
  hasPrevious: boolean;
  /** Whether the control has anything to offer. */
  visible: boolean;
  goToPrevious: () => void;
  goToNext: () => void;
  onScroll: (event: NativeSyntheticEvent<NativeScrollEvent>) => void;
  /**
   * The reader took the viewport by hand: the ladder walk restarts from where
   * they left it rather than continuing the last jump's sequence.
   */
  onScrollBeginDrag: () => void;
  onScrollToIndexFailed: (info: {
    index: number;
    averageItemLength: number;
  }) => void;
  viewabilityConfigCallbackPairs: ViewabilityConfigCallbackPair[];
}

/** The topmost (visually highest) row in a viewable set. */
function topOf(viewableItems: ViewToken[]): number | null {
  let top: number | null = null;
  for (const token of viewableItems) {
    const index = token.index;
    if (index == null || !token.isViewable) continue;
    if (top === null || index > top) top = index;
  }
  return top;
}

/**
 * "↑ previous question / ↓ next question" for a long transcript.
 *
 * One walk over the reader's own **prompts**, read two ways: ↑ steps to the
 * closest prompt above, ↓ to the closest prompt below, and the answers in
 * between are never a stop. A press asks "what else was asked", so a long reply
 * costs one press to pass, not one per screenful.
 *
 * Every landing is a `scrollToIndex` against the list's own measured frames, and
 * takes the prompt's *start* just under the viewport's top edge: there is one
 * landing geometry, and every row is a prompt (see `previousQuestion` /
 * `nextQuestion`).
 *
 * Where the reader *is* comes from what the list reports as viewable — partly
 * and fully visible rows — rather than from row geometry: a row's `onLayout` `y`
 * is relative to its own parent, the virtualized cell, so it says nothing about
 * where the row sits in the transcript. ↑ is offered whether or not the reader
 * has scrolled: a conversation opens at its tail, where the prompt of the turn
 * on screen is one press away. ↓ is the direction that needs the tail off
 * screen — with the tail on screen every newer row is already visible, so there
 * is nothing below to go to. That reading state is the union of the native "a
 * drag took the viewport off the tail" signal owned by `useChatScroll` and the
 * measured offset, because a web build never reports a drag:
 * `onScrollBeginDrag` has no equivalent there.
 *
 * **A landed row is remembered, and the next press walks off it.** The viewport
 * alone cannot say which way a press should go once a row is taller than the
 * screen: a prompt that fills the viewport reports the same topmost row whether
 * the reader is at its first line or its last, so a walk derived from the
 * viewport alone would re-select the row it just landed on and the button would
 * look dead. Keeping the landed row is also what makes the two directions read
 * as one pair: ↓ then ↑ returns to the prompt above it. A manual drag drops the
 * memory, so the next press is derived from the viewport again (a pointer at the
 * row where the finger left off).
 */
export function useQuestionNav({
  sessionId,
  items,
  listRef,
  atLatest,
  onTakeOver,
  hasOlderHistory = false,
  loadingOlder = false,
  loadOlder,
}: {
  sessionId: string;
  /** The rows the list renders, in view order (newest first). */
  items: readonly TimelineItem[];
  listRef: RefObject<FlatList<TimelineItem> | null>;
  atLatest: boolean;
  /**
   * A jump is the reader taking the viewport over from the tail, and it has to
   * say so: while the list is still following the tail, the next streaming
   * commit pins it back to the bottom and the jump is gone. `useChatScroll`
   * owns that handover — the same one the load-older hint performs — and it is
   * required so a host cannot wire a jump that silently undoes itself.
   */
  onTakeOver: () => void;
  /** More transcript pages exist above the loaded slice (remote contract). */
  hasOlderHistory?: boolean;
  /** A page is already being fetched. */
  loadingOlder?: boolean;
  /** Ask for the next page up; resolves to the prepended ids, or false. */
  loadOlder?: () => Promise<false | string[]>;
}): QuestionNavApi {
  const questions = useMemo(() => questionIndices(items), [items]);
  const [nav, setNav] = useState<{
    sessionId: string;
    previous: number | null;
    next: number | null;
  }>({ sessionId, previous: null, next: null });

  const sessionIdRef = useRef(sessionId);
  const itemsRef = useRef(items);
  const questionsRef = useRef(questions);
  const atLatestRef = useRef(atLatest);
  const hasOlderRef = useRef(hasOlderHistory);
  const loadingOlderRef = useRef(loadingOlder);
  const loadOlderRef = useRef(loadOlder);
  const onTakeOverRef = useRef(onTakeOver);
  /** The topmost row with any part on screen, in view space. */
  const topRef = useRef<number | null>(null);
  const offsetRef = useRef(0);
  const jumpRef = useRef<{ index: number; attempts: number } | null>(null);
  /** The prompt the last press landed on, or null while the reader is free. */
  const cursorRef = useRef<number | null>(null);
  const pendingPageRef = useRef(false);
  const realignTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  /**
   * One step of the walk: off the landed prompt when the last press left one,
   * otherwise off the row the viewport points at. `direction` is a step in view
   * space — `1` is older (up the ladder), `-1` newer.
   *
   * Both directions read the same list of prompts, so the answer to "which one
   * is next" depends only on where the reader is: a press never stops on an
   * answer.
   */
  const step = useCallback((direction: 1 | -1): number | null => {
    const walk = questionsRef.current;
    const from = cursorRef.current ?? topRef.current;
    return direction === 1
      ? previousQuestion(walk, from)
      : nextQuestion(walk, from);
  }, []);

  /** Resolve what ↑/↓ would do right now and publish it if it changed. */
  const sync = useCallback(() => {
    const session = sessionIdRef.current;
    const reading =
      !atLatestRef.current || offsetRef.current > AT_LATEST_THRESHOLD_PX;
    // ↑ reads the prompts and the visible rows alone: the prompt above the
    // reader is one they can reach, from the tail as much as from anywhere
    // else. ↓ is the tail-dependent half of the walk.
    const previous = step(1);
    const next = reading ? step(-1) : null;
    setNav(current =>
      current.sessionId === session &&
      current.previous === previous &&
      current.next === next
        ? current
        : { sessionId: session, previous, next },
    );
  }, [step]);

  useEffect(() => {
    sessionIdRef.current = sessionId;
    itemsRef.current = items;
    questionsRef.current = questions;
    atLatestRef.current = atLatest;
    hasOlderRef.current = hasOlderHistory;
    loadingOlderRef.current = loadingOlder;
    loadOlderRef.current = loadOlder;
    onTakeOverRef.current = onTakeOver;
  });

  // Another session opens at its tail: whatever the previous transcript had
  // scrolled to says nothing about this one.
  useEffect(() => {
    offsetRef.current = 0;
    topRef.current = null;
    pendingPageRef.current = false;
    cursorRef.current = null;
    sync();
  }, [sessionId, sync]);

  // Back at the tail (a fresh open, or "back to latest"): the walk starts over
  // from there rather than resuming a jump sequence the reader has left.
  useEffect(() => {
    if (!atLatest) return;
    cursorRef.current = null;
    sync();
  }, [atLatest, sync]);

  // A new turn, a page of history or a session switch all change the answer
  // without any scroll or viewability event.
  useEffect(() => {
    sync();
  }, [sync, questions, atLatest, sessionId]);

  useEffect(
    () => () => {
      if (realignTimerRef.current !== null) clearTimeout(realignTimerRef.current);
    },
    [],
  );

  const onViewable = useCallback(
    (info: { viewableItems: ViewToken[] }) => {
      topRef.current = topOf(info.viewableItems);
      sync();
    },
    [sync],
  );
  // FlatList forbids replacing this prop on the fly and the callback above is
  // stable, so one array owns the configuration for the list's lifetime. (A ref
  // would be the obvious home, but refs may not be read during render, which is
  // where the list needs this prop.)
  const [viewabilityConfigCallbackPairs] = useState<
    ViewabilityConfigCallbackPair[]
  >(() => [
    { viewabilityConfig: PARTLY_VISIBLE, onViewableItemsChanged: onViewable },
  ]);

  /**
   * Scroll a landing into place: the prompt's start just under the viewport's
   * top edge. `viewPosition: 1` is that edge in the inverted list, and a
   * positive `viewOffset` lifts the content, so `JUMP_VIEW_OFFSET` is what turns
   * the alignment into the 12px inset.
   */
  const align = useCallback(
    (index: number) => {
      listRef.current?.scrollToIndex({
        index,
        viewPosition: 1,
        viewOffset: JUMP_VIEW_OFFSET,
        animated: false,
      });
    },
    [listRef],
  );

  /**
   * One more landing pass: a row outside the measured window is estimated from
   * the average row height first, and exact once the landing has rendered it.
   */
  const settleJump = useCallback(
    (index: number) => {
      if (realignTimerRef.current !== null) clearTimeout(realignTimerRef.current);
      realignTimerRef.current = setTimeout(() => {
        realignTimerRef.current = null;
        align(index);
      }, REALIGN_MS);
    },
    [align],
  );

  /**
   * Start a landing: scroll, remember it for a failed scroll, and schedule the
   * re-issue every landing gets.
   */
  const land = useCallback(
    (index: number) => {
      jumpRef.current = { index, attempts: 0 };
      align(index);
      settleJump(index);
    },
    [align, settleJump],
  );

  const jumpTo = useCallback(
    (index: number | null) => {
      if (index === null || listRef.current === null) return;
      onTakeOverRef.current();
      // The press is remembered: the next one walks off this prompt even when
      // the row it landed on fills the screen and the viewport cannot say which
      // end of it the reader is at.
      cursorRef.current = index;
      land(index);
      // What the two buttons offer next follows the walk, not the viewport the
      // jump has just left: the prompt behind the reader is the one they came
      // from.
      sync();
    },
    [land, listRef, sync],
  );

  /**
   * ↑ from the loaded slice's top: pull the next page of history, then land on
   * the oldest turn it contains — which is exactly where the reader was
   * headed. `loadOlder` resolves to the ids the page prepended, in conversation
   * order, so the first of them is the jump's target without any index math
   * across the prepend.
   */
  const pageUp = useCallback(() => {
    if (pendingPageRef.current || loadingOlderRef.current) return;
    const request = loadOlderRef.current;
    if (!request) return;
    // Keep the reading position through the page: without this the list
    // follows the tail (offset 0) while the fetch is in flight, and the
    // landed jump that follows would read as a teleport back to the newest
    // message instead of one step up the transcript.
    onTakeOverRef.current();
    pendingPageRef.current = true;
    void request()
      .then(ids => {
        pendingPageRef.current = false;
        if (!ids || ids.length === 0) return;
        // A backward page arrives in conversation order, so its **first** id is
        // the oldest row it added — one more step up from where the reader was,
        // which is where they were headed. (The host reverses its own
        // descending scan before replying, and both clients prepend a page in
        // the order it arrives.)
        const oldest = ids[0]!;
        const itemsNow = itemsRef.current;
        const index = itemsNow.findIndex(item => item.id === oldest);
        if (index < 0) return;
        land(index);
      })
      .catch(() => {
        pendingPageRef.current = false;
      });
  }, [land]);

  const goToPrevious = useCallback(() => {
    if (nav.sessionId !== sessionId) return;
    // Resolved at the press, not read from the render: the offer published by
    // the last sync is the prompt the reader has just landed on, and walking off
    // that one is exactly what this press is for.
    const question = step(1);
    if (question !== null) {
      jumpTo(question);
      return;
    }
    // The walk ran out at the loaded slice's top: the next prompt lives in
    // history, and the button's contract is to fetch it rather than stall.
    if (hasOlderRef.current) pageUp();
  }, [jumpTo, nav.sessionId, pageUp, sessionId, step]);
  const goToNext = useCallback(() => {
    if (nav.sessionId !== sessionId) return;
    jumpTo(step(-1));
  }, [jumpTo, nav.sessionId, sessionId, step]);

  /**
   * The target is outside the measured window: move to where the average row
   * height says it is, then re-issue the jump once it has been rendered and
   * measured. Attempts are capped — `realignTimerRef` already re-issues once, so
   * this only covers jumps of many screens.
   */
  const onScrollToIndexFailed = useCallback(
    ({ index, averageItemLength }: { index: number; averageItemLength: number }) => {
      const jump = jumpRef.current;
      if (
        jump === null ||
        jump.index !== index ||
        jump.attempts >= MAX_JUMP_ATTEMPTS
      )
        return;
      jump.attempts += 1;
      listRef.current?.scrollToOffset({
        offset: Math.max(0, averageItemLength * index),
        animated: false,
      });
      // The row is still unmeasured, so the landing is still worth re-issuing:
      // this only restarts the window in which that happens.
      settleJump(jump.index);
    },
    [listRef, settleJump],
  );

  const onScroll = useCallback(
    (event: NativeSyntheticEvent<NativeScrollEvent>) => {
      const { contentOffset, contentSize, layoutMeasurement } = event.nativeEvent;
      // A list that fits its viewport cannot be scrolled away from the tail;
      // whatever offset it reports is layout noise, not reading position.
      const fits = contentSize.height <= layoutMeasurement.height + 1;
      offsetRef.current = fits ? 0 : Math.max(0, contentOffset.y);
      sync();
    },
    [sync],
  );

  // The reader's own drag: the walk restarts from where their finger left the
  // transcript instead of continuing a jump sequence they have departed from.
  const onScrollBeginDrag = useCallback(() => {
    cursorRef.current = null;
    sync();
  }, [sync]);

  const inSession = nav.sessionId === sessionId;
  const current = inSession ? nav : { previous: null, next: null };
  // ↑ stays alive one step past the loaded walk when history is known to
  // continue: the press pages first and lands where the page begins.
  const hasPrevious =
    inSession && (current.previous !== null || (hasOlderHistory && !loadingOlder));
  return {
    previous: current.previous,
    next: current.next,
    hasPrevious,
    visible: hasPrevious || current.next !== null,
    goToPrevious,
    goToNext,
    onScroll,
    onScrollBeginDrag,
    onScrollToIndexFailed,
    viewabilityConfigCallbackPairs,
  };
}
