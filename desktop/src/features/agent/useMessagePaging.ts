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
  /** Visible for exactly the active loading/cooldown transaction. */
  showLoadOlderHint: boolean;
  coolingDown: boolean;
  handleScroll: () => void;
  loadOlder: () => void;
  scrollToLatest: () => void;
  showJumpToLatest: boolean;
}

/** Distance from the top that counts as "at the top" for the load hint. */
const TOP_THRESHOLD_PX = 8;
/** Block upward momentum after reaching the top or restoring a history page. */
const WHEEL_COOLDOWN_MS = 1500;
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

  // Synchronous re-entrancy guard. This ref is read in the same tick a wheel
  // event fires (a state flag would only flip after React commits), so a single
  // scroll gesture can't queue a dozen page loads — and a trailing wheel event
  // after the commit is blocked by the viewport protection window.
  const loadingOlderRef = useRef(false);
  const wheelBlockedUntilRef = useRef(0);
  const wheelProtectionRef = useRef(false);
  const acceptingManualScrollRef = useRef(false);
  const wasAtTopRef = useRef(false);
  const [coolingDown, setCoolingDown] = useState(false);
  const [viewportRevision, setViewportRevision] = useState(0);
  const dataPendingRef = useRef(false);
  const renderPendingRef = useRef(false);
  const cooldownTimerRef = useRef<number | null>(null);
  const renderFrameRef = useRef<number | null>(null);
  const mountedRef = useRef(true);
  const nativeScrollLockRef = useRef<{
    container: HTMLElement;
    overflowY: string;
    priority: string;
  } | null>(null);

  const releaseNativeScrollLock = useCallback(() => {
    const lock = nativeScrollLockRef.current;
    if (!lock)
      return;
    if (lock.overflowY)
      lock.container.style.setProperty("overflow-y", lock.overflowY, lock.priority);
    else
      lock.container.style.removeProperty("overflow-y");
    nativeScrollLockRef.current = null;
  }, []);

  const finishCooldown = useCallback(() => {
    if (dataPendingRef.current || renderPendingRef.current)
      return;
    const remaining = wheelBlockedUntilRef.current - performance.now();
    if (remaining > 0) {
      if (cooldownTimerRef.current !== null)
        window.clearTimeout(cooldownTimerRef.current);
      cooldownTimerRef.current = window.setTimeout(finishCooldown, remaining);
      return;
    }
    wheelProtectionRef.current = false;
    releaseNativeScrollLock();
    setCoolingDown(false);
  }, [releaseNativeScrollLock]);

  // A React commit alone is not a painted, stable viewport. Wait for two
  // animation frames with unchanged geometry after anchor correction. Resizes
  // restart this check. Image/network completion is not a render barrier; late
  // resizes continue to be handled by the shared anchor controller.
  const settleRenderedViewport = useCallback(() => {
    if (!wheelProtectionRef.current)
      return;
    renderPendingRef.current = true;
    if (renderFrameRef.current !== null)
      window.cancelAnimationFrame(renderFrameRef.current);
    if (dataPendingRef.current)
      return;
    let previousGeometry = "";
    let stableFrames = 0;
    const checkFrame = () => {
      const container = scrollRef.current;
      const geometry = container
        ? `${container.scrollHeight}:${container.clientHeight}:${container.scrollTop}`
        : "detached";
      stableFrames = geometry === previousGeometry ? stableFrames + 1 : 0;
      previousGeometry = geometry;
      if (stableFrames < 2) {
        renderFrameRef.current = window.requestAnimationFrame(checkFrame);
        return;
      }
      renderFrameRef.current = null;
      renderPendingRef.current = false;
      finishCooldown();
    };
    renderFrameRef.current = window.requestAnimationFrame(checkFrame);
  }, [finishCooldown, scrollRef]);

  const protectViewport = useCallback(() => {
    wheelProtectionRef.current = true;
    const container = scrollRef.current;
    if (container && !nativeScrollLockRef.current) {
      nativeScrollLockRef.current = {
        container,
        overflowY: container.style.getPropertyValue("overflow-y"),
        priority: container.style.getPropertyPriority("overflow-y"),
      };
      // preventDefault cannot cancel the tail of every WebKit gesture. Stop
      // native scrolling itself, while retaining programmatic anchor correction.
      container.style.setProperty("overflow-y", "hidden");
    }
    wheelBlockedUntilRef.current = performance.now() + WHEEL_COOLDOWN_MS;
    renderPendingRef.current = true;
    setCoolingDown(true);
    setViewportRevision(revision => revision + 1);
    if (cooldownTimerRef.current !== null)
      window.clearTimeout(cooldownTimerRef.current);
    cooldownTimerRef.current = window.setTimeout(finishCooldown, WHEEL_COOLDOWN_MS);
  }, [finishCooldown, scrollRef]);

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
    isReadingAnchorLocked: () => wheelProtectionRef.current && !acceptingManualScrollRef.current,
    onScroll,
    onContentSettled: () => {
      // Anchor restoration can leave the top before its scroll event arrives.
      // Re-arm top-entry detection synchronously for the next collision.
      if (scrollRef.current && scrollRef.current.scrollTop > TOP_THRESHOLD_PX) {
        wasAtTopRef.current = false;
      }
      settleRenderedViewport();
      onContentSettled?.();
    },
  });
  useLayoutEffect(() => {
    setWindowStartId(visibleMessages[0]?.id ?? null);
  }, [visibleMessages]);
  const canLoadOlder = effectivePageStart > 0 || hasOlderHistory;
  // Keep the hint visible throughout protection, including after the anchor
  // moves away from the top or the final history page has been loaded.
  const showLoadOlderHint = coolingDown;

  const loadOlder = useCallback(() => {
    if (
      loadingOlderRef.current
      || (effectivePageStart <= 0 && !hasOlderHistory)
    ) {
      return;
    }
    loadingOlderRef.current = true;
    wasAtTopRef.current = (scrollRef.current?.scrollTop ?? 0) <= TOP_THRESHOLD_PX;
    preserveViewport();
    protectViewport();
    if (effectivePageStart <= 0 && loadOlderHistory) {
      // Keep the protected anchor through the request. Explicit downward input
      // updates it; recapturing here could adopt a not-yet-delivered native drift.
      dataPendingRef.current = true;
      void loadOlderHistory((page) => {
        if (page.length > 0)
          setWindowStartId(page[0]!.id);
      }).finally(() => {
        if (!mountedRef.current)
          return;
        dataPendingRef.current = false;
        loadingOlderRef.current = false;
        // Force a commit even for an empty/failed page before checking layout.
        setViewportRevision(revision => revision + 1);
      });
    }
    else {
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
    scrollRef,
    userExchangeCount,
  ]);

  useLayoutEffect(() => {
    if (!dataPendingRef.current)
      loadingOlderRef.current = false;
    settleRenderedViewport();
  }, [visibleMessages, coolingDown, viewportRevision, settleRenderedViewport]);

  // Entering the top starts one transaction: hint + timer + history load.
  // Programmatic anchor restoration re-arms entry detection above; events
  // during this transaction must never restart its timer or load another page.
  const handleScroll = useCallback(() => {
    handleViewportScroll();
    const container = scrollRef.current;
    if (!container)
      return;
    const isAtTop = container.scrollTop <= TOP_THRESHOLD_PX;
    const enteredTop = isAtTop && !wasAtTopRef.current;
    wasAtTopRef.current = isAtTop;
    if (enteredTop && canLoadOlder && !wheelProtectionRef.current)
      loadOlder();
  }, [canLoadOlder, handleViewportScroll, loadOlder, scrollRef]);

  const acceptManualViewport = useCallback(() => {
    acceptingManualScrollRef.current = true;
    try {
      // Adopt both the new anchor and normal bottom-follow semantics.
      handleViewportScroll();
    }
    finally {
      acceptingManualScrollRef.current = false;
    }
  }, [handleViewportScroll]);

  // Keep this listener attached during the entire transaction. A wheel also
  // detects a top collision when scrollTop is clamped and no scroll event fires.
  const wheelStateRef = useRef({ canLoadOlder, loadOlder, acceptManualViewport });
  useLayoutEffect(() => {
    wheelStateRef.current = { canLoadOlder, loadOlder, acceptManualViewport };
  }, [acceptManualViewport, canLoadOlder, loadOlder]);

  useEffect(() => {
    const container = scrollRef.current;
    if (!container)
      return;
    const onWheel = (event: WheelEvent) => {
      if (scrollRef.current !== container)
        return;
      if (event.ctrlKey || event.deltaY === 0)
        return;
      if (wheelProtectionRef.current) {
        if (event.cancelable)
          event.preventDefault();
        if (event.deltaY > 0) {
          // Native vertical scrolling is locked, but an explicit downward
          // wheel still establishes a new reading position immediately.
          const lineHeight = Number.parseFloat(getComputedStyle(container).lineHeight) || 16;
          const unit = event.deltaMode === 1 ? lineHeight : event.deltaMode === 2 ? container.clientHeight : 1;
          container.scrollTop += event.deltaY * unit;
          wheelStateRef.current.acceptManualViewport();
        }
        return;
      }
      if (event.deltaY > 0)
        return;
      const state = wheelStateRef.current;
      if (!state.canLoadOlder || container.scrollTop > TOP_THRESHOLD_PX)
        return;
      if (event.cancelable)
        event.preventDefault();
      state.loadOlder();
    };
    container.addEventListener("wheel", onWheel, { passive: false });
    return () => container.removeEventListener("wheel", onWheel);
  }, [scrollRef]);

  // Cancel timer and layout work when switching conversations.
  useEffect(
    () => {
      mountedRef.current = true;
      return () => {
        mountedRef.current = false;
        releaseNativeScrollLock();
        if (cooldownTimerRef.current !== null)
          window.clearTimeout(cooldownTimerRef.current);
        if (renderFrameRef.current !== null)
          window.cancelAnimationFrame(renderFrameRef.current);
      };
    },
    [releaseNativeScrollLock],
  );

  return {
    visibleMessages,
    canLoadOlder,
    showLoadOlderHint,
    coolingDown,
    handleScroll,
    loadOlder,
    scrollToLatest,
    showJumpToLatest,
  };
}
