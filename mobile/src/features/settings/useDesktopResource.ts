import { useCallback, useEffect, useRef, useState } from "react";
import { AppState } from "react-native";

/** Short-lived view state only. Reads on open, invalidation and foreground;
 * generation fencing prevents a late snapshot replacing a newer read. */
export function useDesktopResource<T>(load: () => Promise<T>, revision: number, enabled: boolean) {
  const [data, setData] = useState<T | null>(null);
  const [loading, setLoading] = useState(false);
  const [failed, setFailed] = useState(false);
  const generation = useRef(0);
  const reload = useCallback(async () => {
    const current = ++generation.current;
    if (!enabled) return;
    setLoading(true);
    setFailed(false);
    try {
      const result = await load();
      if (current === generation.current) setData(result);
    } catch {
      if (current === generation.current) setFailed(true);
    } finally {
      if (current === generation.current) setLoading(false);
    }
  }, [enabled, load]);
  useEffect(() => {
    // Loading/error state belongs to this asynchronous desktop read lifecycle.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void reload();
    const subscription = AppState.addEventListener("change", state => {
      if (state === "active") void reload();
    });
    return () => {
      generation.current += 1;
      subscription.remove();
    };
  }, [reload, revision]);
  // A reload with a snapshot already on screen is a refresh, not a load: the
  // page keeps drawing what it has, and nothing reports "loading" for it.
  // Reporting it as loading made a page that renders a spinner row jump the
  // moment any write invalidated it (reported on the model-visibility list:
  // every switch flipped the row's own resource and the list shifted under
  // the reader's finger).
  return { data: enabled ? data : null, loading: loading && data === null, failed, refresh: loading && data !== null, reload };
}
