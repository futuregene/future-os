import { useEffect, useRef } from "react";

interface UsePollingOptions {
  /** When false, the timer is not installed and the callback never runs. */
  enabled?: boolean;
  /**
   * Extra dependencies that should restart the poll (re-running it immediately
   * and resetting the interval) when they change — typically the values the
   * callback closes over.
   */
  deps?: React.DependencyList;
}

/**
 * Runs `callback` once immediately and then every `intervalMs` while enabled.
 * Always invokes the latest callback (no stale closure); clears the timer and
 * suppresses any further ticks on unmount or when `enabled` flips false.
 *
 * Note: this manages the timer lifecycle only — it does not cancel an
 * in-flight async callback. For race-sensitive loads (where a stale response
 * must not overwrite newer state) drive the fetch through `useAsyncResource`.
 */
export function usePolling(
  callback: () => void | Promise<void>,
  intervalMs: number,
  options: UsePollingOptions = {},
) {
  const { enabled = true, deps = [] } = options;
  const callbackRef = useRef(callback);
  callbackRef.current = callback;

  useEffect(() => {
    if (!enabled) {
      return;
    }

    let active = true;
    const tick = () => {
      if (active) {
        void callbackRef.current();
      }
    };

    tick();
    const timer = window.setInterval(tick, intervalMs);
    return () => {
      active = false;
      window.clearInterval(timer);
    };
    // eslint-disable-next-line react/exhaustive-deps
  }, [enabled, intervalMs, ...deps]);
}
