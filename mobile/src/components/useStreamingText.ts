import { useEffect, useState } from "react";
import { AccessibilityInfo } from "react-native";

/** The sync engine already coalesces live text. Display each committed snapshot
 * directly instead of starting a second 32ms typewriter timer that reparses
 * intermediate Markdown and keeps waking JS after the stream has stopped.
 * Reduced motion still controls the native fade of newly inserted blocks. */
export function useStreamingText(text: string, streaming: boolean) {
  const [reduceMotion, setReduceMotion] = useState(true);
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
  return { text, reduceMotion };
}
