import { useEffect, useRef, useState } from "react";
import { AccessibilityInfo } from "react-native";

const FRAME_MS = 32;
const REVEAL_MS = 192;

/** Presentation only: the authoritative transcript and copy action stay intact.
 * Reveal appended text in bounded batches, never type out initially cached
 * history or animate a non-prefix replacement/correction. */
export function useStreamingText(text: string, streaming: boolean) {
  // Default to no motion until the OS preference has been read.
  const [reduceMotion, setReduceMotion] = useState(true);
  const [visible, setVisible] = useState(text);
  const visibleRef = useRef(text);
  const targetRef = useRef(text);
  const wasStreaming = useRef(streaming);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (!streaming) return;
    let current = true;
    let changed = false;
    const subscription = AccessibilityInfo.addEventListener("reduceMotionChanged", value => {
      changed = true;
      if (current) setReduceMotion(value);
    });
    void AccessibilityInfo.isReduceMotionEnabled().then(value => {
      if (current && !changed) setReduceMotion(value);
    }).catch(() => { /* Keep the conservative no-motion default. */ });
    return () => { current = false; subscription.remove(); };
  }, [streaming]);

  useEffect(() => {
    targetRef.current = text;
    const continuing = streaming || wasStreaming.current || timerRef.current !== null;
    wasStreaming.current = streaming;
    if (reduceMotion || !continuing || !text.startsWith(visibleRef.current)) {
      if (timerRef.current !== null) clearTimeout(timerRef.current);
      timerRef.current = null;
      visibleRef.current = text;
      setVisible(text);
      return;
    }
    if (text === visibleRef.current || timerRef.current !== null) return;
    const start = visibleRef.current.length;
    const startedAt = Date.now();
    const frame = () => {
      const target = targetRef.current;
      const progress = Math.min(1, (Date.now() - startedAt) / REVEAL_MS);
      let end = Math.max(visibleRef.current.length, start + Math.ceil((target.length - start) * progress));
      // Never expose half of a UTF-16 surrogate pair (emoji and CJK extensions).
      if (end < target.length && end > 0 && /[\uD800-\uDBFF]/.test(target[end - 1]!)) end += 1;
      const next = target.slice(0, end);
      visibleRef.current = next;
      setVisible(next);
      timerRef.current = next === target ? null : setTimeout(frame, FRAME_MS);
    };
    timerRef.current = setTimeout(frame, FRAME_MS);
  }, [text, streaming, reduceMotion]);

  useEffect(() => () => {
    if (timerRef.current !== null) clearTimeout(timerRef.current);
  }, []);

  // Corrections and reduced motion must not leave one paint of obsolete text.
  const displayed = reduceMotion || !text.startsWith(visible) ? text : visible;
  return { text: displayed, reduceMotion };
}
