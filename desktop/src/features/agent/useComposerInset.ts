import { useLayoutEffect, useRef, useState } from "react";

/** Reserve the full floating footer, including notices, approvals and spacing. */
export function useComposerInset() {
  const composerRef = useRef<HTMLDivElement>(null);
  const [composerHeight, setComposerHeight] = useState(0);

  useLayoutEffect(() => {
    const composer = composerRef.current;
    if (!composer)
      return;
    const measure = () => setComposerHeight(composer.getBoundingClientRect().height);
    measure();
    // A transcript that is not laid out by a browser (the test environment) has
    // no observer to grow with: the measurement above is then the whole answer,
    // and constructing one would throw before the frame is drawn.
    if (typeof ResizeObserver === "undefined")
      return;
    const observer = new ResizeObserver(measure);
    observer.observe(composer, { box: "border-box" });
    return () => observer.disconnect();
  }, []);

  return { composerRef, composerHeight };
}
