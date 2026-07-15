/**
 * Typed application event bus. A thin wrapper over `window` CustomEvents so the
 * event names and payloads are checked at the call sites (emit and subscribe)
 * instead of being stringly-typed. The transport is unchanged — these are still
 * DOM CustomEvents on `window` — so cross-component, no-prop-drilling signaling
 * works exactly as before.
 */
export interface FutureEventMap {
  "inspect-run": { runId: string };
  /** Open the Runs panel and select a specific tool call by id. */
  "inspect-tool": { runId: string; toolId: string };
  "inspect-artifact": { artifactId: string };
  "open-review": { reviewId: string };
  "review-updated": { threadId: string };
  "recover-run": {
    action: "continue" | "retry";
    runId: string;
    triggerMessageId?: string | null;
  };
  "toast": { message: string; tone?: "error" | "info" };
  // Attach a workspace file to the composer as an `@`-mention pill. `path` is
  // workspace-relative (the form the mention pill stores); emitted by the file
  // tree, consumed by the active thread's Composer.
  "attach-file-to-context": { path: string; name: string };
  /** Emitted when the agent completes a write/edit/bash tool — the file tree
   * should re-read so newly created or modified files appear. */
  "file-tree-refresh": void;
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
