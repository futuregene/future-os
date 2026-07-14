/**
 * CDP event router with per-sessionId dispatch.
 *
 * Events are keyed by `${sessionId ?? "browser"}::${method}`.
 * This ensures that Page.loadEventFired on tab A doesn't wake
 * a navigation waiter on tab B.
 */
export type CdpEventHandler = (params: unknown) => void;

export class CdpEventRouter {
  /** Key: `${sessionId ?? "browser"}::${method}` */
  private handlers = new Map<string, Set<CdpEventHandler>>();

  /**
   * Register a handler for a (sessionId, method) pair.
   * Returns unsubscribe function.
   */
  add(
    sessionId: string | undefined,
    method: string,
    handler: CdpEventHandler,
  ): () => void {
    const key = eventKey(sessionId, method);
    let set = this.handlers.get(key);
    if (!set) {
      set = new Set();
      this.handlers.set(key, set);
    }
    set.add(handler);
    return () => {
      set?.delete(handler);
      if (set && set.size === 0) this.handlers.delete(key);
    };
  }

  /**
   * Dispatch an incoming CDP event.
   */
  dispatch(
    sessionId: string | undefined,
    method: string,
    params: unknown,
  ): void {
    // First try the specific sessionId key
    const specificKey = eventKey(sessionId, method);
    const specific = this.handlers.get(specificKey);
    if (specific) {
      for (const handler of specific) {
        try { handler(params); } catch { /* don't let one handler break others */ }
      }
    }

    // Also try the wildcard ("all sessions for this method") key
    const wildcardKey = eventKey(undefined, method);
    const wildcard = this.handlers.get(wildcardKey);
    if (wildcard) {
      for (const handler of wildcard) {
        try { handler(params); } catch { /* don't let one handler break others */ }
      }
    }
  }

  /**
   * Remove all handlers for a specific session.
   * Called when a target is detached or destroyed.
   */
  clearSession(sessionId: string): void {
    for (const [key, _set] of this.handlers) {
      if (key.startsWith(`${sessionId}::`)) {
        this.handlers.delete(key);
      }
    }
  }

  /** Remove ALL handlers. Called on disconnect. */
  clear(): void {
    this.handlers.clear();
  }
}

function eventKey(sessionId: string | undefined, method: string): string {
  return `${sessionId ?? "browser"}::${method}`;
}
