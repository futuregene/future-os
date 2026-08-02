import type { RefObject } from "react";
import type { AgentMessage } from "./agentThreadTypes";
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

/**
 * One rendered page of conversation history: a run of turns starting at a user
 * message (a "turn" is one user question plus the reply that follows it). The
 * window is defined by the index of its first user message, so the top of a
 * loaded page is always a user bubble — the "must have a user question" rule.
 * The tail beyond the window (streaming bubbles, replies whose user message
 * landed inside the window) is always included.
 */
export function computePageStart(messages: AgentMessage[], userTurnCount: number): number {
  if (userTurnCount <= 0 || messages.length === 0)
    return 0;
  // Walk backwards from the tail, counting user messages until the window is
  // full; the page starts at that user's index.
  let remaining = userTurnCount;
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
  /** How many user turns each page renders. The first page shows the last N. */
  userTurnCount: number;
  /** Changing this (the active thread id) resets the window to the latest page. */
  resetKey: unknown;
  /** Caller's scroll handler — composed in front of the paging handler. */
  onScroll?: () => void;
}

interface UseMessagePagingResult {
  visibleMessages: AgentMessage[];
  canLoadOlder: boolean;
  loadingOlder: boolean;
  /** True when the user is pinned to the top and more history exists. */
  showLoadOlderHint: boolean;
  handleScroll: () => void;
  loadOlder: () => void;
}

/** Distance from the top that counts as "at the top" for the load hint. */
const TOP_THRESHOLD_PX = 8;

/**
 * Windowed rendering for long threads. `messages` stays fully loaded in memory
 * (the agent session JSONL is projected once, cheaply); this hook only controls
 * which slice renders. Pages are counted in user turns, so loading an older page
 * never splits a user question from its reply.
 *
 * Scroll anchoring across a page load follows the same idea opencode uses: when
 * a page is prepended, record the first visible message and its offset from the
 * viewport top, then restore that exact offset in a layout effect (before the
 * browser paints) — the viewport never visibly jumps.
 */
export function useMessagePaging({
  messages,
  scrollRef,
  userTurnCount,
  resetKey,
  onScroll,
}: UseMessagePagingInput): UseMessagePagingResult {
  const [loadedPages, setLoadedPages] = useState(1);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [atTop, setAtTop] = useState(false);
  const onScrollRef = useRef(onScroll);
  onScrollRef.current = onScroll;

  const pageStart = computePageStart(messages, loadedPages * userTurnCount);
  // Clamp to the current list (the list can shrink during an in-flight load)
  // and never produce an empty window.
  const clampedPageStart = Math.min(pageStart, messages.length);
  const effectivePageStart = clampedPageStart >= messages.length
    ? Math.max(0, messages.length - 1)
    : clampedPageStart;
  const visibleMessages = messages.slice(effectivePageStart);
  const canLoadOlder = effectivePageStart > 0;
  const showLoadOlderHint = canLoadOlder && atTop && !loadingOlder;

  // Pending scroll restore for the in-flight page load. Written synchronously in
  // `loadOlder`, consumed (and cleared) by the layout effect after commit.
  const restoreRef = useRef<{ anchor: Anchor | null } | null>(null);

  const loadOlder = useCallback(() => {
    if (loadingOlder)
      return;
    if (effectivePageStart <= 0)
      return;
    restoreRef.current = {
      anchor: captureAnchor(scrollRef.current, "data-message-id"),
    };
    setLoadingOlder(true);
    setLoadedPages(pages => pages + 1);
  }, [effectivePageStart, loadingOlder, scrollRef]);

  // Reset on thread switch: newest page, top hint hidden. A layout effect
  // declared before the restore effect below, so on a reset that races an
  // in-flight page load the reset's clear runs first and the restore can't
  // read a pending anchor captured for the previous thread.
  useLayoutEffect(() => {
    setLoadedPages(1);
    setLoadingOlder(false);
    setAtTop(false);
    restoreRef.current = null;
    // `messages` isn't a dependency: the reset targets the thread change only.
  }, [resetKey]);

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
    setLoadingOlder(false);
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
  }, [scrollRef]);

  // Compose the caller's scroll handling with top detection. `scrollTop === 0`
  // means the user is at the very top of the thread.
  const handleScroll = useCallback(() => {
    onScrollRef.current?.();
    const container = scrollRef.current;
    if (container)
      setAtTop(container.scrollTop <= TOP_THRESHOLD_PX);
  }, [scrollRef]);

  // Second channel for the load gesture: a wheel-scroll up while already stuck
  // at the top fires the load, alongside clicking the hint button. The listener
  // only mounts while the hint is live, so a single fast scroll-up can't fire
  // two loads (the guard in `loadOlder` would ignore the second anyway).
  useEffect(() => {
    if (!canLoadOlder || !atTop || loadingOlder)
      return;
    const container = scrollRef.current;
    if (!container)
      return;
    const onWheel = (event: WheelEvent) => {
      if (event.deltaY < 0)
        loadOlder();
    };
    container.addEventListener("wheel", onWheel, { passive: true });
    return () => container.removeEventListener("wheel", onWheel);
  }, [atTop, canLoadOlder, loadOlder, loadingOlder, scrollRef]);

  return {
    visibleMessages,
    canLoadOlder,
    loadingOlder,
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
    if (!id)
      continue;
    if (rect.top < bestTop) {
      bestTop = rect.top;
      best = { id, offset: rect.top - viewTop };
    }
  }
  return best;
}
