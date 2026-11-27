import type { RefObject } from "react";
import { useCallback, useLayoutEffect, useRef, useState } from "react";
import { useCommittedRef } from "../../lib/useCommittedRef";

/** Within this many px of the bottom, auto-follow streaming output. */
const STICK_THRESHOLD_PX = 48;
/** Past this distance below the viewport for the newest message, reveal "jump to latest". */
const JUMP_BUTTON_THRESHOLD_PX = 240;

/**
 * How far the newest message's end sits below the viewport — the distance that
 * decides the "jump to latest" button.
 *
 * Not `scrollHeight - clientHeight - scrollTop`. The transcript ends in a spacer
 * as tall as the composer, so the last line clears the input box, and the pane
 * keeps bottom padding of its own; no scroll closes that tail. Measuring to the
 * content's end therefore reports a distance a reader parked on the newest
 * message cannot reduce, and the button offers a jump to a message already on
 * screen. Measured to the message instead — the same way stickiness is decided,
 * from the caret rather than the document — it reads 0 once that message is in
 * view. Rows are `[data-message-id]`, which both transcripts mark.
 */
function newestMessageBelow(container: HTMLElement): number {
  const rows = container.querySelectorAll<HTMLElement>("[data-message-id]");
  const newest = rows[rows.length - 1];
  if (!newest)
    return 0;
  const overshoot
    = newest.getBoundingClientRect().bottom
      - container.getBoundingClientRect().bottom;
  return Math.max(0, overshoot);
}

interface UseStickyAutoScrollInput {
  scrollRef: RefObject<HTMLElement | null>;
  /** Changing this (e.g. the message list) re-runs the follow effect. */
  contentKey: unknown;
  /**
   * Which conversation the viewport state below belongs to.
   *
   * A caller can swap the transcript under one scroll container without
   * remounting the view — the remote view does exactly that when the reader
   * opens another conversation — and this hook's refs outlive that swap. The
   * previous conversation's "the reader scrolled up" state and its reading
   * anchor then describe messages the new transcript does not have, and
   * `settleViewport` keeps the current position rather than following, so a long
   * conversation opens on whatever pixel offset the last one was left at: a
   * historical message, seemingly at random.
   *
   * Changing this drops both and follows the newest message again. A caller that
   * already remounts per conversation may leave it out.
   */
  resetKey?: unknown;
  /** False while the real message content is temporarily not mounted. */
  followEnabled?: boolean;
  /** Consume a WebKit momentum scroll that must restore the reading anchor. */
  shouldRestoreReadingAnchor?: () => boolean;
  /** Extra work to run on every scroll event (e.g. floating-scrollbar visibility). */
  onScroll?: () => void;
  /** Run after a content-driven follow settles (e.g. update floating scrollbar). */
  onContentSettled?: () => void;
}

/**
 * Sticky auto-scroll: follow streaming output only while pinned near the bottom.
 * When the user scrolls up past the threshold we stop following (so they can read
 * history) and offer a "jump to latest" button once they're far. Orthogonal to
 * message/run state — depends only on the scroll container and the two keys.
 */
export function useStickyAutoScroll({
  scrollRef,
  contentKey,
  resetKey,
  followEnabled = true,
  shouldRestoreReadingAnchor,
  onScroll,
  onContentSettled,
}: UseStickyAutoScrollInput) {
  const stickToBottomRef = useRef(true);
  const anchorRef = useRef<Anchor | null>(null);
  const writtenTopRef = useRef<number | null>(null);

  const preserveViewport = useCallback(() => {
    stickToBottomRef.current = false;
    anchorRef.current = captureAnchor(scrollRef.current);
  }, [scrollRef]);
  const [showJumpToLatest, setShowJumpToLatest] = useState(false);
  // Keep the callbacks in refs so the effect/handlers always call the latest
  // without listing them as deps (which would re-run the follow effect on every
  // render when the parent passes inline closures).
  const shouldRestoreAnchorRef = useCommittedRef(shouldRestoreReadingAnchor);
  const onScrollRef = useCommittedRef(onScroll);
  const onContentSettledRef = useCommittedRef(onContentSettled);

  // Jump straight to the latest message and re-enable auto-follow.
  const scrollToLatest = useCallback(() => {
    const scrollContainer = scrollRef.current;
    if (!scrollContainer)
      return;
    stickToBottomRef.current = true;
    anchorRef.current = null;
    setShowJumpToLatest(false);
    scrollContainer.scrollTop = scrollContainer.scrollHeight;
  }, [scrollRef]);

  // A conversation's viewport belongs to that conversation: a swap of the
  // transcript under this container (see `resetKey`) must not inherit the last
  // one's stickiness or reading anchor. Runs before the follow effect in the same
  // commit, so a conversation opened this way settles on its own newest message.
  const resetKeyRef = useRef(resetKey);
  useLayoutEffect(() => {
    if (Object.is(resetKeyRef.current, resetKey))
      return;
    resetKeyRef.current = resetKey;
    stickToBottomRef.current = true;
    anchorRef.current = null;
    setShowJumpToLatest(false);
    const container = scrollRef.current;
    if (container) {
      container.scrollTop = container.scrollHeight;
      writtenTopRef.current = container.scrollTop;
    }
  }, [resetKey, scrollRef]);

  const settleViewport = useCallback(() => {
    if (!followEnabled)
      return;
    const container = scrollRef.current;
    if (!container)
      return;
    const before = container.scrollTop;
    if (stickToBottomRef.current) {
      container.scrollTop = container.scrollHeight;
    }
    else {
      const anchor = anchorRef.current;
      const target
        = anchor
          && Array.from(
            container.querySelectorAll<HTMLElement>("[data-message-id]"),
          ).find(element => element.dataset.messageId === anchor.id);
      if (target) {
        const delta
          = target.getBoundingClientRect().top
            - container.getBoundingClientRect().top
            - anchor!.offset;
        if (Math.abs(delta) > 0.5)
          container.scrollTop += delta;
      }
      else {
        // A replaced transcript can lose the old ID. Keep the current position
        // and adopt a surviving message; never jump unconditionally to zero.
        anchorRef.current = captureAnchor(container);
      }
    }
    if (container.scrollTop !== before)
      writtenTopRef.current = container.scrollTop;
    setShowJumpToLatest(
      !stickToBottomRef.current
      && newestMessageBelow(container) > JUMP_BUTTON_THRESHOLD_PX,
    );
    onContentSettledRef.current?.();
  }, [followEnabled, onContentSettledRef, scrollRef]);

  // Compose external scroll handling (e.g. floating scrollbar visibility) with
  // sticky detection: re-derive stickiness from the caret's distance to the
  // bottom. A programmatic scroll-to-bottom lands here too, leaving distance ≈ 0
  // → stays stuck; a user scroll-up grows the distance → unsticks. Two
  // thresholds: a tight one to keep following, a looser one to reveal the button.
  const handleScroll = useCallback(() => {
    // WebKit can deliver scrolling from an already-latched, noncancelable
    // gesture. Correct it before it can replace the preserved reading anchor.
    if (shouldRestoreAnchorRef.current?.() && anchorRef.current) {
      settleViewport();
      onScrollRef.current?.();
      return;
    }
    onScrollRef.current?.();
    const scrollContainer = scrollRef.current;
    if (scrollContainer) {
      const distance
        = scrollContainer.scrollHeight
          - scrollContainer.clientHeight
          - scrollContainer.scrollTop;
      // Ignore our own anchor correction. All other movement (wheel, keyboard,
      // scrollbar or search navigation) establishes a new reading position.
      if (
        writtenTopRef.current === null
        || Math.abs(scrollContainer.scrollTop - writtenTopRef.current) > 0.5
      ) {
        stickToBottomRef.current = distance <= STICK_THRESHOLD_PX;
        anchorRef.current = stickToBottomRef.current
          ? null
          : captureAnchor(scrollContainer);
      }
      writtenTopRef.current = null;
      setShowJumpToLatest(
        newestMessageBelow(scrollContainer) > JUMP_BUTTON_THRESHOLD_PX,
      );
    }
  }, [onScrollRef, scrollRef, settleViewport, shouldRestoreAnchorRef]);

  useLayoutEffect(settleViewport, [contentKey, settleViewport]);

  // Keep the same anchor through deferred image/markdown layout, not merely
  // the first React commit. Observe both viewport and content dimensions.
  useLayoutEffect(() => {
    const container = scrollRef.current;
    if (!container || typeof ResizeObserver === "undefined")
      return;
    const observer = new ResizeObserver(settleViewport);
    observer.observe(container);
    for (const child of container.children) observer.observe(child);
    return () => observer.disconnect();
  }, [contentKey, scrollRef, settleViewport]);

  return { handleScroll, scrollToLatest, showJumpToLatest, preserveViewport };
}

interface Anchor {
  id: string;
  offset: number;
}

function captureAnchor(container: HTMLElement | null): Anchor | null {
  if (!container)
    return null;
  const top = container.getBoundingClientRect().top;
  for (const element of container.querySelectorAll<HTMLElement>(
    "[data-message-id]",
  )) {
    const rect = element.getBoundingClientRect();
    if (rect.bottom > top)
      return { id: element.dataset.messageId!, offset: rect.top - top };
  }
  return null;
}
