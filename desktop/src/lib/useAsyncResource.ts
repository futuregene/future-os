import { useCallback, useEffect, useRef, useState } from "react";

export interface AsyncResource<T> {
  data: T;
  loading: boolean;
  error: string | null;
  /** Force a refetch, e.g. after a mutation or on a poll tick. */
  reload: () => void;
}

export interface UseAsyncResourceOptions<T> {
  /**
   * When supplied, a load result that is equal to the current `data` will
   * NOT trigger a re-render — `setState` is short-circuited. Equality is
   * compared against the previous value (new vs. old), so the same array
   * identity check via `Object.is` is already free; pass this only for
   * structural comparison (e.g. arrays of records keyed by id).
   */
  isEqual?: (previous: T, next: T) => boolean;
}

/**
 * Loads an async resource with cancellation safety.  When deps change (e.g.
 * the active thread switched) a full fetch runs and `loading` flips to true.
 * When ONLY `reload()` is called (poll tick, deps unchanged) the fetch is
 * silent — `data` stays visible and `loading` stays false so the UI never
 * shows a stale spinner mid-poll.
 */
export function useAsyncResource<T>(
  loader: () => Promise<T>,
  deps: React.DependencyList,
  initialData: T,
  options: UseAsyncResourceOptions<T> = {},
): AsyncResource<T> {
  const { isEqual } = options;
  const [data, setData] = useState<T>(initialData);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [reloadToken, setReloadToken] = useState(0);
  const prevDepsRef = useRef(deps);
  const mountedRef = useRef(false);

  const reload = useCallback(() => {
    setReloadToken(token => token + 1);
  }, []);

  // Track whether this load is a silent poll tick vs. a deps change (where
  // `loading` should flip so the user sees a spinner). Computed on EVERY
  // render against the deps snapshotted by the last effect run — a useMemo
  // keyed on `deps` would only recompute when deps change, so it could never
  // observe an unchanged-deps poll tick and would stay `true` forever.
  const depsChanged = !mountedRef.current
    || prevDepsRef.current.length !== deps.length
    || prevDepsRef.current.some((dep, idx) => !Object.is(dep, deps[idx]));

  useEffect(() => {
    // Snapshot deps for the *next* tick's comparison.
    prevDepsRef.current = deps;
    mountedRef.current = true;

    let cancelled = false;
    const silent = !depsChanged;
    if (!silent) {
      setLoading(true);
    }
    setError(null);

    loader()
      .then((result) => {
        if (!cancelled) {
          setData((prev) => {
            if (isEqual && prev !== initialData && isEqual(prev, result)) {
              return prev; // structural equal — skip re-render
            }
            return result;
          });
          setLoading(false);
        }
      })
      .catch((cause) => {
        if (!cancelled) {
          setError(cause instanceof Error ? cause.message : String(cause));
          setLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react/exhaustive-deps
  }, [...deps, reloadToken]);

  return { data, error, loading, reload };
}
