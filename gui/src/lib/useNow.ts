import { useSyncExternalStore } from "react";

/**
 * Returns `Date.now()` and re-renders the caller every `intervalMs` so that
 * time-relative labels ("3 minutes ago") stay fresh without a manual refresh.
 * A static clock timestamp never needs this; a relative one does, since it
 * silently goes stale between renders.
 *
 * All subscribers share ONE global 1s ticker (previously every MessageBlock
 * installed its own interval — hundreds of timers on long threads). A
 * subscriber re-renders only when its own bucket (`floor(now / intervalMs)`)
 * changes, so a 60s relative-timestamp subscriber sleeps through the 1s
 * ticks a streaming elapsed timer needs.
 *
 * Pass `enabled: false` to freeze the clock (no subscription, no re-renders)
 * — e.g. a live elapsed timer that should only tick while its run streams.
 */

const TICK_MS = 1000;
const listeners = new Set<() => void>();
let timer: number | null = null;
// Updated only on ticks (and on subscribe), so getSnapshot is stable between
// notifications — no mid-render tearing.
let currentTick = Date.now();

function subscribe(listener: () => void): () => void {
  currentTick = Date.now();
  listeners.add(listener);
  if (timer === null) {
    timer = window.setInterval(() => {
      currentTick = Date.now();
      for (const listener of listeners) {
        listener();
      }
    }, TICK_MS);
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0 && timer !== null) {
      window.clearInterval(timer);
      timer = null;
    }
  };
}

function noopSubscribe(): () => void {
  return () => {};
}

export function useNow(intervalMs: number = 60_000, enabled: boolean = true): number {
  const bucket = useSyncExternalStore(
    enabled ? subscribe : noopSubscribe,
    enabled ? () => Math.floor(currentTick / intervalMs) : () => 0,
    () => 0,
  );
  // Disabled: no subscription; the value refreshes whenever the component
  // re-renders for other reasons (it isn't displayed while disabled anyway).
  return enabled ? bucket * intervalMs : Date.now();
}
