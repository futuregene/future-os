/**
 * Typed application event bus. A thin wrapper over `window` CustomEvents so the
 * event names and payloads are checked at the call sites (emit and subscribe)
 * instead of being stringly-typed. The transport is unchanged — these are still
 * DOM CustomEvents on `window` — so cross-component, no-prop-drilling signaling
 * works exactly as before.
 */
export interface FutureEventMap {
  "inspect-run": { runId: string };
  "inspect-artifact": { artifactId: string };
  "open-review": { reviewId: string };
  "review-updated": { threadId: string };
  "open-research-resource": { resourceId: string };
  "recover-run": {
    action: "continue" | "retry";
    runId: string;
    triggerMessageId?: string | null;
  };
  "toast": { message: string; tone?: "error" | "info" };
}

type FutureEventName = keyof FutureEventMap;

function channel(type: FutureEventName): string {
  return `futureos:${type}`;
}

export function emitFutureEvent<K extends FutureEventName>(type: K, detail: FutureEventMap[K]): void {
  window.dispatchEvent(new CustomEvent(channel(type), { detail }));
}

/**
 * Subscribe to a typed event. Returns an unsubscribe function suitable for
 * returning directly from a `useEffect`.
 */
export function onFutureEvent<K extends FutureEventName>(
  type: K,
  handler: (detail: FutureEventMap[K]) => void,
): () => void {
  const listener = (event: Event) => {
    handler((event as CustomEvent<FutureEventMap[K]>).detail);
  };
  window.addEventListener(channel(type), listener);
  return () => {
    window.removeEventListener(channel(type), listener);
  };
}
