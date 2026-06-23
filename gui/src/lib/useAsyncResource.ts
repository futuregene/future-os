import { useCallback, useEffect, useState } from "react";

export interface AsyncResource<T> {
  data: T;
  loading: boolean;
  error: string | null;
  /** Force a refetch, e.g. after a mutation or on a poll tick. */
  reload: () => void;
}

/**
 * Loads an async resource with cancellation safety: a load started by an
 * earlier render (or before `deps` changed, or before unmount) never overwrites
 * state with its late result. Replaces the hand-rolled `let cancelled = false`
 * effect pattern.
 *
 * `loader` is intentionally not part of the dependency list — pass the values
 * it closes over via `deps` so the caller controls exactly when a refetch runs.
 */
export function useAsyncResource<T>(
  loader: () => Promise<T>,
  deps: React.DependencyList,
  initialData: T,
): AsyncResource<T> {
  const [data, setData] = useState<T>(initialData);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [reloadToken, setReloadToken] = useState(0);

  const reload = useCallback(() => {
    setReloadToken(token => token + 1);
  }, []);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);

    loader()
      .then((result) => {
        if (!cancelled) {
          setData(result);
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
