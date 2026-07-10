import { useEffect, useState } from "react";

/**
 * Returns `Date.now()` and re-renders the caller every `intervalMs` so that
 * time-relative labels ("3 minutes ago") stay fresh without a manual refresh.
 * A static clock timestamp never needs this; a relative one does, since it
 * silently goes stale between renders.
 *
 * Keep the interval coarse (default 60s) — relative labels only change at the
 * minute granularity, so ticking faster just wastes renders.
 *
 * Pass `enabled: false` to freeze the clock (no interval, no re-renders) — e.g.
 * a live elapsed timer that should only tick while its run is streaming.
 */
export function useNow(intervalMs: number = 60_000, enabled: boolean = true): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!enabled)
      return;
    setNow(Date.now());
    const id = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(id);
  }, [intervalMs, enabled]);
  return now;
}
