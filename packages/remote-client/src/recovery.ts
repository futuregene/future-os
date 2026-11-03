import type { TimelineItem } from "./types";

/**
 * Recovery predicate: the *latest* reply did not finish cleanly, so it can be
 * retried or continued.
 *
 * Deliberately reason-agnostic. A run can stop for many reasons — the model
 * failed, the user stopped it, the stream was truncated, the app restarted and
 * startup convergence settled the run `cancelled` — and every one of them
 * leaves a reply the reader may want to pick up from. The decision belongs to
 * whoever taps the button; nothing here (and nothing on the desktop, which only
 * re-sends a prompt) has to agree with the reason first. The earlier reading —
 * failed runs only, and never a user stop — hid the one affordance a restart
 * leaves you needing.
 *
 * Still required: a settled reply (`streaming` off), a run to continue, and the
 * latest turn. Recovering an older exchange would append its continuation to
 * the end of the conversation instead of where the reply stopped.
 */
export function canRecoverMessage(item: TimelineItem, latestAssistantId: string | null): boolean {
  return (
    item.kind === "message" &&
    item.role === "assistant" &&
    item.id === latestAssistantId &&
    item.streaming !== true &&
    (item.failed === true || item.stopped === true || item.truncated === true) &&
    typeof item.runId === "string" &&
    item.runId.length > 0
  );
}
