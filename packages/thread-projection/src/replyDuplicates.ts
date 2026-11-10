import type { AgentMessage } from "./model";

/**
 * Collapse a turn's reply to the one bubble it is.
 *
 * A reply can be on screen more than once. The agent persists a partial
 * snapshot after each model call, so a reload *during* the run projects that
 * snapshot into the array as a settled-looking reply; the live bubble writing
 * the same run is a second copy, appended beside it. Reload again and a third
 * appears. Measured on the desktop as one answer drawn three times — "49s ·
 * 2,557 tokens", then the same text under "✓ 13 · 思考 11 次", then a third —
 * while the stored transcript held it once and a full reload rendered it once.
 *
 * Two rules, both keyed on identity rather than text:
 *
 * - one reply per run: a later bubble carrying a run replaces an earlier one
 *   carrying the same run (the live bubble owns the reply, the snapshot merely
 *   described it);
 * - a snapshot with no run at all belongs to the reply directly below it: the
 *   stamps are withheld from in-flight runs (`AgentMessage.inFlightRunId`), so
 *   an unidentified reply immediately above an identified one, with no prompt
 *   between them, can only be that same reply written earlier.
 *
 * Nothing else moves: an older exchange's replies and every user message stay
 * exactly where they are.
 */
export function collapseReplyDuplicates<
  T extends { id: string; role?: string; runId?: string | null; inFlightRunId?: string | null },
>(messages: T[]): T[] {
  const out: T[] = [];
  for (const message of messages) {
    const run = message.role === "assistant" ? (message.runId ?? message.inFlightRunId ?? null) : null;
    if (message.role === "assistant") {
      // The reply directly above this one, while nothing but replies separate
      // them (a prompt between them means a new turn, and that stops the walk).
      let above = out.length - 1;
      while (above >= 0 && out[above]!.role === "assistant") {
        const previous = out[above]!;
        const previousRun = previous.runId ?? previous.inFlightRunId ?? null;
        // One reply per run: the newcomer replaces the copy already drawn.
        if (run && previousRun === run) {
          out.splice(above, 1);
          above -= 1;
          continue;
        }
        // A snapshot of a run this client cannot name, sitting directly above
        // the reply that does name it.
        if (run && !previousRun) {
          out.splice(above, 1);
          above -= 1;
          continue;
        }
        break;
      }
    }
    out.push(message);
  }
  return out.length === messages.length ? messages : out;
}

/** Convenience for the live writers: the array with each reply drawn once. */
export function withoutReplyDuplicates(messages: AgentMessage[]): AgentMessage[] {
  return collapseReplyDuplicates(messages);
}
