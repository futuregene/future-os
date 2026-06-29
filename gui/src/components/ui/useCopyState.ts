import { useCallback, useEffect, useRef, useState } from "react";
import { copyText } from "../../lib/clipboard";

/**
 * Copy text to the clipboard and flash a transient "copied" flag. The flag is
 * keyed so one hook can back several copy targets (e.g. path vs content). The
 * reset timer is cleared on unmount and on rapid re-copy, so there are no
 * stacked timers and no post-unmount state updates.
 */
export function useCopyState<K extends string = "default">(resetMs = 1400) {
  const [copiedKey, setCopiedKey] = useState<K | null>(null);
  const timerRef = useRef<number | null>(null);

  const clearTimer = useCallback(() => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  const copy = useCallback(async (text: string, key: K = "default" as K) => {
    await copyText(text);
    setCopiedKey(key);
    clearTimer();
    timerRef.current = window.setTimeout(setCopiedKey, resetMs, null);
  }, [clearTimer, resetMs]);

  useEffect(() => clearTimer, [clearTimer]);

  return { copiedKey, copy };
}
