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
    const observer = new ResizeObserver(measure);
    observer.observe(composer, { box: "border-box" });
    return () => observer.disconnect();
  }, []);

  return { composerRef, composerHeight };
}
