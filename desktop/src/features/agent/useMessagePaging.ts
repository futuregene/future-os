import type { AgentMessage } from "@future-os/thread-projection";
import type { RefObject } from "react";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import { useStickyAutoScroll } from "./useStickyAutoScroll";

/**
 * One rendered page of conversation history: a run of exchanges starting at a
 * user message (an "exchange" is one user question plus the reply that follows
 * it). The window is defined by the index of its first user message, so the
 * top of a loaded page is always a user bubble — the "must have a user
 * question" rule. The tail beyond the window (streaming bubbles, replies whose
 * user message landed inside the window) is always included.
 */
export function computePageStart(
  messages: AgentMessage[],
  userExchangeCount: number,
): number {
  if (userExchangeCount <= 0 || messages.length === 0)
    return 0;
  // Walk backwards from the tail, counting user messages until the window is
  // full; the page starts at that user's index.
  let remaining = userExchangeCount;
  for (let index = messages.length - 1; index >= 0; index--) {
    if (messages[index]!.role === "user") {
      remaining -= 1;
      if (remaining <= 0)
        return index;
    }
  }
  return 0;
}

interface UseMessagePagingInput {
  messages: AgentMessage[];
  /**
   * Scroll container. The shared viewport controller manages `scrollTop` to preserve
   * the user's viewport across a page load. `scrollTop` stays `0` while the
   * user is stuck at the top, so top-gesture detection must listen to `wheel`
   * (see `handleScroll`).
   */
  scrollRef: RefObject<HTMLElement | null>;
  /** How many user exchanges each page renders. The first page shows the last N. */
  userExchangeCount: number;
  /** Caller's scroll handler — composed in front of the paging handler. */
  onScroll?: () => void;
  hasOlderHistory?: boolean;
  loadOlderHistory?: (
    beforeCommit?: (messages: AgentMessage[]) => void,
  ) => Promise<void>;
  followEnabled?: boolean;
  onContentSettled?: () => void;
}

interface UseMessagePagingResult {
  visibleMessages: AgentMessage[];
  canLoadOlder: boolean;
  /** True when the user is pinned to the top and more history exists. */
  showLoadOlderHint: boolean;
  handleScroll: () => void;
  loadOlder: () => void;
  scrollToLatest: () => void;
  showJumpToLatest: boolean;
}

/** Distance from the top that counts as "at the top" for the load hint. */
const TOP_THRESHOLD_PX = 8;
/** Block upward momentum after reaching the top or restoring a history page. */
const WHEEL_COOLDOWN_MS = 500;
/**
 * How long the user must rest at the top before the load button appears. This
 * is the visual confirm gate; the separate wheel protection window blocks
 * upward momentum while the hint appears and history is restored.
 */
const TOP_SETTLE_MS = 350;

/**
 * Windowed rendering over the loaded history. Once the local window reaches
 * its oldest entry, ask the storage hook for another page. Pages are counted in user exchanges, so loading an older
 * page never splits a user question from its reply.
 *
 * A stable leading message defines the window. The viewport controller owns
 * both bottom-follow and persistent reading anchors, including late resizes.
 */
export function useMessagePaging({
  messages,
  scrollRef,
  userExchangeCount,
  onScroll,
  hasOlderHistory = false,
  loadOlderHistory,
  followEnabled,
  onContentSettled,
}: UseMessagePagingInput): UseMessagePagingResult {
  const [windowStartId, setWindowStartId] = useState<string | null>(null);
  const [atTop, setAtTop] = useState(false);
  const [topSettled, setTopSettled] = useState(false);
  const topSettleTimerRef = useRef<number | null>(null);

  // Synchronous re-entrancy guard. This ref is read in the same tick a wheel
  // event fires (a state flag would only flip after React commits), so a single
  // scroll gesture can't queue a dozen page loads — and a trailing wheel event
  // after the commit is blocked by the viewport protection window.
  const loadingOlderRef = useRef(false);
  const wheelBlockedUntilRef = useRef(0);
  const wheelProtectionRef = useRef(false);
  const wasAtTopRef = useRef(false);

  const protectViewport = useCallback(() => {
    wheelProtectionRef.current = true;
    wheelBlockedUntilRef.current = performance.now() + WHEEL_COOLDOWN_MS;
  }, []);

  // Once rendered, the leading message stays in the window even when new
  // exchanges arrive. Counting backwards from the tail would evict that row.
  const pinnedStart
    = windowStartId === null
      ? -1
      : messages.findIndex(message => message.id === windowStartId);
  const effectivePageStart
    = pinnedStart >= 0
      ? pinnedStart
      : computePageStart(messages, userExchangeCount);
  const visibleMessages = useMemo(
    () => messages.slice(effectivePageStart),
    [messages, effectivePageStart],
  );
  const {
    handleScroll: handleViewportScroll,
    preserveViewport,
    scrollToLatest,
    showJumpToLatest,
  } = useStickyAutoScroll({
    scrollRef,
    contentKey: visibleMessages,
    followEnabled,
    onScroll,
    onContentSettled,
  });
  useLayoutEffect(() => {
    setWindowStartId(visibleMessages[0]?.id ?? null);
  }, [visibleMessages]);
  const canLoadOlder = effectivePageStart > 0 || hasOlderHistory;
  // The button only appears after the user has rested at the top for the settle
  // window — arriving at the top must not, by itself, ever trigger a load.
  const showLoadOlderHint = canLoadOlder && atTop && topSettled;

  const loadOlder = useCallback(() => {
    if (
      loadingOlderRef.current
      || (effectivePageStart <= 0 && !hasOlderHistory)
    ) {
      return;
    }
    loadingOlderRef.current = true;
    protectViewport();
    if (topSettleTimerRef.current !== null) {
      window.clearTimeout(topSettleTimerRef.current);
      topSettleTimerRef.current = null;
    }
    setTopSettled(false);
    if (effectivePageStart <= 0 && loadOlderHistory) {
      // The callback runs only for a valid page, immediately before its state
      // update. Capture the CURRENT viewport, not the one at request start.
      void loadOlderHistory((page) => {
        protectViewport();
        preserveViewport();
        if (page.length > 0)
          setWindowStartId(page[0]!.id);
      }).finally(() => {
        wheelBlockedUntilRef.current = performance.now() + WHEEL_COOLDOWN_MS;
        loadingOlderRef.current = false;
      });
    }
    else {
      preserveViewport();
      const start = computePageStart(
        messages.slice(0, effectivePageStart),
        userExchangeCount,
      );
      setWindowStartId(messages[start]?.id ?? null);
    }
  }, [
    effectivePageStart,
    hasOlderHistory,
    loadOlderHistory,
    messages,
    preserveViewport,
    protectViewport,
    userExchangeCount,
  ]);

  useLayoutEffect(() => {
    loadingOlderRef.current = false;
  }, [windowStartId]);

  // Compose the caller's scroll handling with top detection. `scrollTop === 0`
  // means the user is at the very top; resting there for the settle window turns
  // the load button on. Any scroll away (or a load starting) cancels it — the
  // button must re-settle before the next pull counts.
  const handleScroll = useCallback(() => {
    handleViewportScroll();
    const container = scrollRef.current;
    if (!container)
      return;
    const isAtTop = container.scrollTop <= TOP_THRESHOLD_PX;
    if (isAtTop && !wasAtTopRef.current && canLoadOlder)
      protectViewport();
    wasAtTopRef.current = isAtTop;
    setAtTop(isAtTop);
    if (!isAtTop) {
      if (topSettleTimerRef.current !== null) {
        window.clearTimeout(topSettleTimerRef.current);
        topSettleTimerRef.current = null;
      }
      setTopSettled(false);
      return;
    }
    if (topSettleTimerRef.current !== null)
      return;
    topSettleTimerRef.current = window.setTimeout(() => {
      topSettleTimerRef.current = null;
      setTopSettled(true);
    }, TOP_SETTLE_MS);
  }, [canLoadOlder, handleViewportScroll, protectViewport, scrollRef]);

  // Keep the non-passive listener mounted across loading and anchor restoration.
  // Otherwise the momentum tail can scroll the newly prepended content even
  // though the load hint has disappeared. Reversing direction releases the gate.
  const wheelStateRef = useRef({ canLoadOlder, atTop, topSettled, loadOlder });
  useLayoutEffect(() => {
    wheelStateRef.current = { canLoadOlder, atTop, topSettled, loadOlder };
  }, [atTop, canLoadOlder, loadOlder, topSettled]);

  useEffect(() => {
    const container = scrollRef.current;
    if (!container)
      return;
    const onWheel = (event: WheelEvent) => {
      if (scrollRef.current !== container)
        return;
      if (event.ctrlKey || event.deltaY === 0)
        return;
      if (event.deltaY > 0) {
        wheelProtectionRef.current = false;
        return;
      }
      if (
        wheelProtectionRef.current
        && (loadingOlderRef.current || performance.now() < wheelBlockedUntilRef.current)
      ) {
        if (event.cancelable)
          event.preventDefault();
        return;
      }
      const state = wheelStateRef.current;
      if (!state.canLoadOlder || !state.atTop || !state.topSettled)
        return;
      if (event.cancelable)
        event.preventDefault();
      state.loadOlder();
    };
    container.addEventListener("wheel", onWheel, { passive: false });
    return () => container.removeEventListener("wheel", onWheel);
  }, [scrollRef]);

  // Don't leave a pending settle timer firing setState after unmount.
  useEffect(
    () => () => {
      if (topSettleTimerRef.current !== null) {
        window.clearTimeout(topSettleTimerRef.current);
        topSettleTimerRef.current = null;
      }
    },
    [],
  );

  return {
    visibleMessages,
    canLoadOlder,
    showLoadOlderHint,
    handleScroll,
    loadOlder,
    scrollToLatest,
    showJumpToLatest,
  };
}
