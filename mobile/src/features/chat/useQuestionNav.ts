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
  ViewabilityConfig,
  ViewabilityConfigCallbackPair,
  ViewToken,
} from "react-native";
import type { TimelineItem } from "../../remote/types";
import { spacing } from "../../theme/tokens";
import {
  EMPTY_ROWS,
  nextQuestion,
  previousQuestion,
  questionIndices,
  viewportRows,
  type ViewportRows,
} from "./questionNav";

/** The offset `useChatScroll` already treats as "at the latest". */
const AT_LATEST_THRESHOLD_PX = 32;
/**
 * The geometry a jump aims at: the landed question sits this far below the
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

const PARTLY_VISIBLE: ViewabilityConfig = { itemVisiblePercentThreshold: 1 };
const FULLY_VISIBLE: ViewabilityConfig = { itemVisiblePercentThreshold: 100 };

export interface QuestionNavApi {
  /**
   * View-space index of the ↑ target (the question start above the reader), or
   * null. Offered from the tail as well, where it is the start of the turn on
   * screen.
   */
  previous: number | null;
  /**
   * View-space index of the ↓ target (the next question down), or null. Only
   * once the reader has left the tail: at the tail there is nothing newer below.
   */
  next: number | null;
  /** Whether the control has anything to offer. */
  visible: boolean;
  goToPrevious: () => void;
  goToNext: () => void;
  onScroll: (event: NativeSyntheticEvent<NativeScrollEvent>) => void;
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
 * The anchor comes from what the list reports as viewable — partly and fully
 * visible rows — rather than from row geometry: a row's `onLayout` `y` is
 * relative to its own parent, the virtualized cell, so it says nothing about
 * where the row sits in the transcript. ↑ is offered whether or not the reader
 * has scrolled: a conversation opens at its tail, where the question that
 * starts the turn on screen is still one press away. ↓ is the direction that
 * needs the tail off screen — with the tail on screen every newer question is
 * already visible, so there is nothing below to go to. That reading state is
 * the union of the native "a drag took the viewport off the tail" signal owned
 * by `useChatScroll` and the measured offset, because a web build never reports
 * a drag: `onScrollBeginDrag` has no equivalent there.
 */
export function useQuestionNav({
  sessionId,
  items,
  listRef,
  atLatest,
  onTakeOver,
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
}): QuestionNavApi {
  const questions = useMemo(() => questionIndices(items), [items]);
  const [nav, setNav] = useState<{
    sessionId: string;
    previous: number | null;
    next: number | null;
  }>({ sessionId, previous: null, next: null });
  const sessionIdRef = useRef(sessionId);
  const questionsRef = useRef(questions);
  const atLatestRef = useRef(atLatest);
  const rowsRef = useRef<ViewportRows>(EMPTY_ROWS);
  const partialTopRef = useRef<number | null>(null);
  const fullTopRef = useRef<number | null>(null);
  const offsetRef = useRef(0);
  const jumpRef = useRef<{ index: number; attempts: number } | null>(null);
  const realignTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const refreshRows = useCallback(() => {
    rowsRef.current = viewportRows({
      partialTop: partialTopRef.current,
      fullTop: fullTopRef.current,
    });
  }, []);

  /** Resolve what ↑/↓ would do right now and publish it if it changed. */
  const sync = useCallback(() => {
    const session = sessionIdRef.current;
    const reading =
      !atLatestRef.current || offsetRef.current > AT_LATEST_THRESHOLD_PX;
    // ↑ reads the viewport rows alone: the question above the top edge is a
    // question the reader can reach, from the tail as much as from anywhere
    // else. ↓ is the tail-dependent half of the ladder.
    const previous = previousQuestion(questionsRef.current, rowsRef.current);
    const next = reading
      ? nextQuestion(questionsRef.current, rowsRef.current)
      : null;
    setNav(current =>
      current.sessionId === session &&
      current.previous === previous &&
      current.next === next
        ? current
        : { sessionId: session, previous, next },
    );
  }, []);

  useEffect(() => {
    sessionIdRef.current = sessionId;
    questionsRef.current = questions;
    atLatestRef.current = atLatest;
  });

  // Another session opens at its tail: whatever the previous transcript had
  // scrolled to says nothing about this one.
  useEffect(() => {
    offsetRef.current = 0;
    partialTopRef.current = null;
    fullTopRef.current = null;
    rowsRef.current = EMPTY_ROWS;
    sync();
  }, [sessionId, sync]);

  // A new question, a page of history or a session switch all change the answer
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

  const onPartlyVisible = useCallback(
    (info: { viewableItems: ViewToken[] }) => {
      partialTopRef.current = topOf(info.viewableItems);
      refreshRows();
      sync();
    },
    [refreshRows, sync],
  );
  const onFullyVisible = useCallback(
    (info: { viewableItems: ViewToken[] }) => {
      fullTopRef.current = topOf(info.viewableItems);
      refreshRows();
      sync();
    },
    [refreshRows, sync],
  );
  // FlatList forbids replacing this prop on the fly and the callbacks above are
  // stable, so one array owns the two configurations for the list's lifetime.
  // (A ref would be the obvious home, but refs may not be read during render,
  // which is where the list needs this prop.)
  const [viewabilityConfigCallbackPairs] = useState<
    ViewabilityConfigCallbackPair[]
  >(() => [
    {
      viewabilityConfig: PARTLY_VISIBLE,
      onViewableItemsChanged: onPartlyVisible,
    },
    { viewabilityConfig: FULLY_VISIBLE, onViewableItemsChanged: onFullyVisible },
  ]);

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

  const jumpTo = useCallback(
    (index: number | null) => {
      if (index === null || listRef.current === null) return;
      if (realignTimerRef.current !== null) clearTimeout(realignTimerRef.current);
      jumpRef.current = { index, attempts: 0 };
      onTakeOver();
      align(index);
      realignTimerRef.current = setTimeout(() => align(index), REALIGN_MS);
    },
    [align, listRef, onTakeOver],
  );

  const goToPrevious = useCallback(() => {
    jumpTo(nav.sessionId === sessionId ? nav.previous : null);
  }, [jumpTo, nav, sessionId]);
  const goToNext = useCallback(() => {
    jumpTo(nav.sessionId === sessionId ? nav.next : null);
  }, [jumpTo, nav, sessionId]);

  /**
   * The target is outside the measured window: move to where the average row
   * height says it is, then re-issue the jump once it has been rendered and
   * measured. Attempts are capped — `realignTimerRef` already re-issues once, so
   * this only covers jumps of many screens.
   */
  const onScrollToIndexFailed = useCallback(
    ({ index, averageItemLength }: { index: number; averageItemLength: number }) => {
      const jump = jumpRef.current;
      if (jump === null || jump.index !== index || jump.attempts >= MAX_JUMP_ATTEMPTS)
        return;
      jump.attempts += 1;
      listRef.current?.scrollToOffset({
        offset: Math.max(0, averageItemLength * index),
        animated: false,
      });
      if (realignTimerRef.current !== null) clearTimeout(realignTimerRef.current);
      realignTimerRef.current = setTimeout(() => align(index), REALIGN_MS);
    },
    [align, listRef],
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

  const current = nav.sessionId === sessionId ? nav : { previous: null, next: null };
  return {
    previous: current.previous,
    next: current.next,
    visible: current.previous !== null || current.next !== null,
    goToPrevious,
    goToNext,
    onScroll,
    onScrollToIndexFailed,
    viewabilityConfigCallbackPairs,
  };
}
