import { useLayoutEffect, useRef, useState } from "react";

/**
 * How much of the frame the transcript keeps, whatever the footer measures.
 *
 * The footer floats over the transcript and the transcript reserves its height,
 * so a footer taller than the frame leaves the conversation a scroll region made
 * entirely of blank space. The queue panel caps its own list now; this is the
 * floor under it, for the case where a capped queue still shares the footer with
 * an approval card and a long draft.
 */
const MIN_TRANSCRIPT_VISIBLE = 120;

/**
 * Reserve the full floating footer, including notices, approvals and spacing.
 *
 * The reservation is the footer's own measured height — a footer that grows
 * (a notice, an approval card, a queued message) pushes the last turn up by
 * exactly what it added — bounded by the room the frame actually has.
 */
export function useComposerInset() {
  const composerRef = useRef<HTMLDivElement>(null);
  const [composerHeight, setComposerHeight] = useState(0);

  useLayoutEffect(() => {
    const composer = composerRef.current;
    if (!composer)
      return;
    const measure = () => {
      const height = composer.getBoundingClientRect().height;
      // The frame is what the footer floats in (the transcript's relative box),
      // not the window: a session sidebar or a title bar is not room this footer
      // can have. Measuring against it is what keeps the transcript's visible
      // height above zero rather than letting the reservation outrun the frame.
      const frame = composer.parentElement?.getBoundingClientRect().height ?? 0;
      setComposerHeight(
        frame > MIN_TRANSCRIPT_VISIBLE
          ? Math.min(height, frame - MIN_TRANSCRIPT_VISIBLE)
          : height,
      );
    };
    measure();
    // A transcript that is not laid out by a browser (the test environment) has
    // no observer to grow with: the measurement above is then the whole answer,
    // and constructing one would throw before the frame is drawn.
    if (typeof ResizeObserver === "undefined")
      return;
    const observer = new ResizeObserver(measure);
    observer.observe(composer, { box: "border-box" });
    // The clamp is drawn against the frame, so a window that shrinks with the
    // footer's own height unchanged must re-measure too — otherwise the inset
    // keeps a size the frame no longer has.
    const frameElement = composer.parentElement;
    if (frameElement)
      observer.observe(frameElement, { box: "border-box" });
    return () => observer.disconnect();
  }, []);

  return { composerRef, composerHeight };
}
