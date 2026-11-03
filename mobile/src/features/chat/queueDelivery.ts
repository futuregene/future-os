/**
 * What the desktop's refusal to take a queued message means.
 *
 * The phone sends a queue as soon as the run it waited for is over. "Over" is the
 * desktop's own state, and the two reads of it disagree for a moment: the screen's
 * streaming flag comes from the last event or catalogue snapshot it has seen, while
 * the host may still consider the run alive for a beat after an interrupt. Sending
 * into that beat is refused with `session_busy` — which is not "no", it is "not
 * yet", and the queue must wait it out rather than hand the reader a retry for a
 * moment that passes by itself.
 */

/** The refusals that mean the run is still finishing, rather than a refusal. */
export function isTransientQueueRefusal(error: unknown): boolean {
  const code = error instanceof Error ? error.message : "";
  // `session_busy`: the desktop is still inside the run (the usual answer for a
  // send that arrives while an interrupt is settling). `send_busy`: this client's
  // own previous send is still in flight — a race with the interrupt, not a fault.
  return code.includes("session_busy") || code === "send_busy";
}

/** How long to wait before asking again (`QUEUE_RETRY_LIMIT` of these ≈ 7 s). */
export const QUEUE_RETRY_DELAY_MS = 350;

/**
 * How many times to ask again before treating it as a real refusal.
 *
 * An interrupted run settles in well under a second on a healthy pairing; the
 * budget covers a slow link without leaving a queue that retries forever.
 */
export const QUEUE_RETRY_LIMIT = 20;

/**
 * The command id a batch of rows was handed to the desktop under, if it has one.
 *
 * The desktop dedupes a prompt by its command id and by nothing else, so one
 * message must keep one id from the first attempt onward: the queue's own retry
 * and the outbox's recovery are two paths to the same message, and a fresh id on
 * either is a second run of the same words. The first attempt mints it; a row that
 * failed with an error the link recovers from carries it back into the panel.
 *
 * A batch answers only when every row agrees — rows the reader added or edited
 * since are a different message, and the next attempt mints a new id for them.
 */
export function batchCommandId(items: readonly { commandId?: string }[]): string | null {
  const id = items[0]?.commandId;
  if (!id)
    return null;
  return items.every(item => item.commandId === id) ? id : null;
}
