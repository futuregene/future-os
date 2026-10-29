/**
 * Keep one wheel gesture inside the scroll container it started in.
 *
 * The desktop app renders in WebKit on macOS, and WebKit re-targets every wheel
 * event to whichever scrollable element is under the pointer. Start scrolling
 * the conversation list and move the pointer across the divider while the wheel
 * is still turning — a flick's momentum tail, or a free-spinning wheel — and the
 * same gesture carries on in the panel on the other side: that list takes the
 * remaining delta and jumps, while the list the user was reading stops where it
 * was. Chromium latches a gesture to the element it started in, which is what
 * this layout is laid out for.
 *
 * So latch it here: the first wheel of a gesture claims the scroll container
 * under the pointer, and a following event of that same gesture that would
 * scroll a *different* container is cancelled. A gap longer than
 * {@link GESTURE_GAP_MS} ends the gesture, so the next one scrolls whatever is
 * under the pointer by then — scrolling one list, then deliberately scrolling
 * the panel next to it, is untouched.
 *
 * Only a cancelable wheel can be cancelled; WebKit reports the momentum tail of
 * a trackpad or free-spin gesture with `cancelable === false` and there the
 * engine still wins. The latch keeps a mouse wheel's notches where they began,
 * which is the case a spinning wheel produces.
 */

/** Silence that ends a wheel gesture; momentum frames arrive far closer. */
const GESTURE_GAP_MS = 200;

/** Slack for fractional layout, so a 0.5px overflow is not a scroller. */
const SCROLL_EPSILON = 1;

export interface ScrollGestureLatchOptions {
  /** Silence, in ms, that ends the gesture. */
  gapMs?: number;
  /** Clock, injectable so a test controls the gesture's timing. */
  now?: () => number;
}

/**
 * The scroll container this event would scroll, or `null` when the pointer is
 * over something that does not scroll wheel-style (`overflow: hidden` keeps a
 * scroller programmatic-only, so it is not one).
 */
export function scrollContainerFor(
  target: EventTarget | null,
  horizontal: boolean,
): HTMLElement | null {
  if (!(target instanceof Element))
    return null;
  const view = target.ownerDocument.defaultView;
  if (!view)
    return null;
  for (let node: Element | null = target; node !== null; node = node.parentElement) {
    if (!(node instanceof HTMLElement))
      continue;
    const style = view.getComputedStyle(node);
    const overflow = horizontal ? style.overflowX : style.overflowY;
    if (overflow !== "auto" && overflow !== "scroll")
      continue;
    const extent = horizontal
      ? node.scrollWidth - node.clientWidth
      : node.scrollHeight - node.clientHeight;
    if (extent > SCROLL_EPSILON)
      return node;
  }
  return null;
}

/**
 * Install the latch on `win`. One listener for the whole app: it only needs to
 * know which gesture is in flight, and every panel that scrolls is under the
 * same window. Returns a cleanup that removes it.
 */
export function installScrollGestureLatch(
  win: Window = window,
  options: ScrollGestureLatchOptions = {},
): () => void {
  const gapMs = options.gapMs ?? GESTURE_GAP_MS;
  const now = options.now ?? (() => win.performance.now());
  let latched: HTMLElement | null = null;
  let lastWheelAt = Number.NEGATIVE_INFINITY;

  const onWheel = (event: WheelEvent) => {
    // A ctrl+wheel is the browser's pinch zoom and a zero delta is no gesture.
    if (event.ctrlKey || (event.deltaX === 0 && event.deltaY === 0))
      return;
    const at = now();
    const alongside = Math.abs(event.deltaX) > Math.abs(event.deltaY);
    const container = scrollContainerFor(event.target, alongside);
    const continues = at - lastWheelAt <= gapMs;
    lastWheelAt = at;
    // A fresh gesture, or one that began over something that does not scroll,
    // adopts whatever the pointer is over now.
    if (!continues || latched === null) {
      latched = container;
      return;
    }
    if (container === null || container === latched)
      return;
    if (event.cancelable)
      event.preventDefault();
  };

  win.addEventListener("wheel", onWheel, { capture: true, passive: false });
  return () => win.removeEventListener("wheel", onWheel, { capture: true });
}
