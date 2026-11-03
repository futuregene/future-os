/**
 * What a host's refusal to take a queued message means.
 *
 * The client sends a queue as soon as its host reports the run over, but "over" is
 * the host's own state and the two can disagree for a beat: a prompt that arrives
 * while the run is still settling is refused with `session_busy`. That is not "no",
 * it is "not yet", and the queue must ride it out rather than hand the reader a
 * retry for a wait that ends by itself. Same rule as the phone's
 * (`mobile/src/features/chat/queueDelivery.ts`), on the same codes.
 */

/** The refusals that mean the run is still finishing, rather than a refusal. */
export function isTransientQueueRefusal(error: unknown): boolean {
  // A Tauri rejection can be a bare string, not an Error: the host's answer
  // travels as `session_busy: this session is still running; …`.
  const message = error instanceof Error ? error.message : String(error ?? "");
  return message.includes("session_busy") || message.includes("send_busy");
}

/** How long to wait before asking again (`QUEUE_RETRY_LIMIT` of these ≈ 7 s). */
export const QUEUE_RETRY_DELAY_MS = 350;

/**
 * How many times to ask again before treating it as a real refusal.
 *
 * An interrupted run settles well inside this on a healthy link; the budget stops
 * a host that keeps refusing from turning the queue into a retry loop.
 */
export const QUEUE_RETRY_LIMIT = 20;
