/**
 * "Which conversation is waiting for me?" — the panel's attention queue.
 *
 * The question is the one a person watching several conversations actually asks,
 * and it has a narrow answer on purpose: a conversation is **not** interesting
 * because it is busy. Only three states need a human:
 *
 * - **`approval`** — a run is blocked on a decision. It is the strictest: nothing
 *   moves until someone answers, and the approval queue includes a run that reports
 *   `waiting_approval` even before the row's count catches up.
 * - **`failed`** — the run stopped with an error. Nobody resumes it by waiting.
 * - **`finished`** — a run ended while the reader was looking elsewhere (the rail's
 *   unread mark). Worth a visit; nothing is blocked.
 *
 * Everything else — queued, running, cancelled, and an idle conversation that was
 * read when it finished — is `null`. A queue that included "running" would be the
 * whole panel, and a queue that is the whole panel is not a queue.
 *
 * Pure, so the rule is one table of cases rather than something to observe by
 * watching a grid.
 */

export type VibeAttentionReason = "approval" | "failed" | "finished";

/** What the rule reads; a subset of what a cell already knows. */
export interface VibeAttentionInput {
  /** Pending approvals for this conversation (the rail's own count). */
  approvals: number;
  /** The latest run's status, or `undefined` when it has never run. */
  runStatus?: string | null;
  /** The rail's "finished while you were not looking" mark. */
  unread: boolean;
}

/** The reason this conversation needs a human, or `null`. */
export function attentionOf({ approvals, runStatus, unread }: VibeAttentionInput): VibeAttentionReason | null {
  // Approval first: it is the only state that blocks the agent, and a failed run
  // with a pending approval still needs the approval answered to move at all.
  if (approvals > 0 || runStatus === "waiting_approval")
    return "approval";
  if (runStatus === "failed")
    return "failed";
  if (unread && runStatus === "completed")
    return "finished";
  return null;
}

/**
 * The next conversation needing attention, after `fromKey` in panel order,
 * wrapping — or `null` when nothing does.
 *
 * Order is the panel's order, not the order things went wrong: the ring comes back
 * to where the reader is sitting, so "next" always means "the next cell clockwise
 * from me", which is the only thing a direction-key-driven panel can promise.
 */
export function nextAttention(
  keys: readonly string[],
  reasons: ReadonlyMap<string, VibeAttentionReason>,
  fromKey: string | null,
): string | null {
  const waiting = keys.filter(key => reasons.has(key));
  if (waiting.length === 0)
    return null;
  if (!fromKey)
    return waiting[0]!;
  const from = keys.indexOf(fromKey);
  if (from === -1)
    return waiting[0]!;
  for (let offset = 1; offset <= keys.length; offset++) {
    const candidate = keys[(from + offset) % keys.length]!;
    if (reasons.has(candidate))
      return candidate;
  }
  return waiting[0]!;
}
