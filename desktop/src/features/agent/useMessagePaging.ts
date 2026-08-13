import type { AgentMessage } from "@future-os/thread-projection";
import type { RefObject } from "react";
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

/**
 * One rendered page of conversation history: a run of exchanges starting at a
 * user message (an "exchange" is one user question plus the reply that follows
 * it). The window is defined by the index of its first user message, so the
 * top of a loaded page is always a user bubble — the "must have a user
 * question" rule. The tail beyond the window (streaming bubbles, replies whose
 * user message landed inside the window) is always included.
 */
export function computePageStart(messages: AgentMessage[], userExchangeCount: number): number {
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
   * Scroll container. The paging hook reads/writes its `scrollTop` to preserve
   * the user's viewport across a page load. `scrollTop` stays `0` while the
   * user is stuck at the top, so top-gesture detection must listen to `wheel`
   * (see `handleScroll`).
   */
  scrollRef: RefObject<HTMLElement | null>;
  /** How many user exchanges each page renders. The first page shows the last N. */
  userExchangeCount: number;
  /** Caller's scroll handler — composed in front of the paging handler. */
  onScroll?: () => void;
}

interface UseMessagePagingResult {
  visibleMessages: AgentMessage[];
  canLoadOlder: boolean;
  /** True when the user is pinned to the top and more history exists. */
  showLoadOlderHint: boolean;
  handleScroll: () => void;
  loadOlder: () => void;
}

/** Distance from the top that counts as "at the top" for the load hint. */
const TOP_THRESHOLD_PX = 8;
/** Wheel events within this window after a load are ignored — one page per gesture. */
const WHEEL_COOLDOWN_MS = 300;
/**
 * How long the user must rest at the top before the load button appears. This
 * is the "confirm gate": the gesture that brought the user to the top ends
 * while the timer runs, so its trailing wheel events can never auto-load — the
 * button must be visibly settled before a pull counts.
 */
const TOP_SETTLE_MS = 350;

/**
 * Windowed rendering for long threads. `messages` stays fully loaded in memory
 * (the agent session JSONL is projected once, cheaply); this hook only controls
 * which slice renders. Pages are counted in user exchanges, so loading an older
 * page never splits a user question from its reply.
 *
 * Scroll anchoring across a page load follows the same idea opencode uses: when
 * a page is prepended, record the first visible message and its offset from the
 * viewport top, then restore that exact offset in a layout effect (before the
 * browser paints) — the viewport never visibly jumps.
 */
export function useMessagePaging({
  messages,
  scrollRef,
  userExchangeCount,
  onScroll,
}: UseMessagePagingInput): UseMessagePagingResult {
  const [loadedPages, setLoadedPages] = useState(1);
  const [atTop, setAtTop] = useState(false);
  const [topSettled, setTopSettled] = useState(false);
  const topSettleTimerRef = useRef<number | null>(null);
  const onScrollRef = useRef(onScroll);
  onScrollRef.current = onScroll;

  // Synchronous re-entrancy guard. This ref is read in the same tick a wheel
  // event fires (a state flag would only flip after React commits), so a single
  // scroll gesture can't queue a dozen page loads — and a trailing wheel event
  // after the commit is swallowed by the cooldown stamp.
  const loadingOlderRef = useRef(false);
  const lastWheelAtRef = useRef(0);

  // computePageStart always returns a valid index (or 0 for an empty/short
  // list), so no clamping is needed here.
  const effectivePageStart = computePageStart(messages, loadedPages * userExchangeCount);
  const visibleMessages = messages.slice(effectivePageStart);
  const canLoadOlder = effectivePageStart > 0;
  // The button only appears after the user has rested at the top for the settle
  // window — arriving at the top must not, by itself, ever trigger a load.
  const showLoadOlderHint = canLoadOlder && atTop && topSettled;

  // Pending scroll restore for the in-flight page load. Written synchronously in
  // `loadOlder`, consumed (and cleared) by the layout effect after commit.
  const restoreRef = useRef<{ anchor: Anchor | null } | null>(null);

  const loadOlder = useCallback(() => {
    // The synchronous ref guard is the real re-entrancy fence: it blocks every
    // wheel event of the gesture until the restore effect clears it.
    if (loadingOlderRef.current)
      return;
    if (effectivePageStart <= 0)
      return;
    loadingOlderRef.current = true;
    if (topSettleTimerRef.current !== null) {
      window.clearTimeout(topSettleTimerRef.current);
      topSettleTimerRef.current = null;
    }
    restoreRef.current = {
      anchor: captureAnchor(scrollRef.current, "data-message-id"),
    };
    setLoadedPages(pages => pages + 1);
  }, [effectivePageStart, scrollRef]);

  // Restore the viewport after the new page renders, before paint. The anchor
  // was captured relative to the container's viewport top; move the scroll
  // offset by the anchor's new position so the user's reading position doesn't
  // move. A lost anchor (ids regenerated on a reload) falls back to pinning the
  // top of the freshly loaded page — the user was at the top when they asked.
  useLayoutEffect(() => {
    const pending = restoreRef.current;
    if (!pending)
      return;
    restoreRef.current = null;
    loadingOlderRef.current = false;
    setTopSettled(false);
    const container = scrollRef.current;
    if (!container)
      return;
    if (pending.anchor) {
      const target = container.querySelector<HTMLElement>(`[data-message-id="${CSS.escape(pending.anchor.id)}"]`);
      if (target) {
        const containerRect = container.getBoundingClientRect();
        const targetRect = target.getBoundingClientRect();
        const delta = (targetRect.top - containerRect.top) - pending.anchor.offset;
        container.scrollTop += delta;
        return;
      }
    }
    container.scrollTop = 0;
  }, [loadedPages, scrollRef]);

  // Compose the caller's scroll handling with top detection. `scrollTop === 0`
  // means the user is at the very top; resting there for the settle window turns
  // the load button on. Any scroll away (or a load starting) cancels it — the
  // button must re-settle before the next pull counts.
  const handleScroll = useCallback(() => {
    onScrollRef.current?.();
    const container = scrollRef.current;
    if (!container)
      return;
    const isAtTop = container.scrollTop <= TOP_THRESHOLD_PX;
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
  }, [scrollRef]);

  // Second channel for the load gesture: a wheel-scroll up while the load button
  // is visible fires the load, alongside clicking it. The listener only mounts
  // once the button has settled (`topSettled`), so the gesture that arrived at
  // the top can never auto-load — the pull must happen after the button shows.
  // The sync ref guard + cooldown stamp keep one gesture to one page load.
  useEffect(() => {
    if (!canLoadOlder || !atTop || !topSettled)
      return;
    const container = scrollRef.current;
    if (!container)
      return;
    const onWheel = (event: WheelEvent) => {
      if (event.deltaY >= 0)
        return;
      const now = performance.now();
      // Swallow the tail of the same gesture so it can't queue a second page.
      if (now - lastWheelAtRef.current < WHEEL_COOLDOWN_MS)
        return;
      lastWheelAtRef.current = now;
      loadOlder();
    };
    container.addEventListener("wheel", onWheel, { passive: true });
    return () => container.removeEventListener("wheel", onWheel);
  }, [atTop, canLoadOlder, loadOlder, scrollRef, topSettled]);

  // Don't leave a pending settle timer firing setState after unmount.
  useEffect(() => () => {
    if (topSettleTimerRef.current !== null) {
      window.clearTimeout(topSettleTimerRef.current);
      topSettleTimerRef.current = null;
    }
  }, []);

  return {
    visibleMessages,
    canLoadOlder,
    showLoadOlderHint,
    handleScroll,
    loadOlder,
  };
}

/** The first visible message's id + its offset from the viewport top. */
interface Anchor {
  id: string;
  offset: number;
}

function captureAnchor(container: HTMLElement | null, attribute: string): Anchor | null {
  if (!container)
    return null;
  const viewTop = container.getBoundingClientRect().top;
  let best: Anchor | null = null;
  let bestTop = Number.POSITIVE_INFINITY;
  for (const element of container.querySelectorAll<HTMLElement>(`[${attribute}]`)) {
    const rect = element.getBoundingClientRect();
    // Only elements actually crossing the viewport top are candidates — the one
    // with the smallest top (highest on screen) anchors the view.
    if (rect.bottom <= viewTop)
      continue;
    const id = element.getAttribute(attribute);
    /* v8 ignore next 2 -- the selector only matches elements carrying the attribute */
    if (!id)
      continue;
    if (rect.top < bestTop) {
      bestTop = rect.top;
      best = { id, offset: rect.top - viewTop };
    }
  }
  return best;
}
