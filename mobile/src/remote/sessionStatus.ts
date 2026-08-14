import type { RemoteSession } from "./types";

/**
 * Effective run status for a session row — mirrors the desktop sidebar
 * (`ThreadListItem.effectiveRunStatus`): the local run status wins when it
 * reports running/queued, but a session the agent reports as streaming with
 * no local run row (e.g. a prompt started by the TUI/CLI/another machine)
 * still reads as running.
 */
export function effectiveRunStatus(
  status: string | undefined,
  streaming: boolean | undefined,
): string | undefined {
  if (status === "running" || status === "queued") return status;
  if (streaming) return "running";
  return status;
}

const RUNNING_STATUSES = new Set(["running", "queued", "waiting_approval"]);
const FINISHED_STATUSES = new Set(["completed", "failed"]);

/**
 * Sessions whose run transitioned running→finished since the last snapshot.
 * The session the user is currently viewing is excluded — a run completing in
 * it isn't "unread", the unread dot exists to pull the eye to other sessions.
 */
export function detectFinished(
  prevStatus: Record<string, string | undefined>,
  sessions: { sessionId: string; status?: string }[],
  selectedSessionId?: string,
): { finished: string[]; next: Record<string, string | undefined> } {
  const finished: string[] = [];
  const next: Record<string, string | undefined> = {};
  for (const s of sessions) {
    const before = prevStatus[s.sessionId];
    if (
      before !== undefined &&
      RUNNING_STATUSES.has(before) &&
      s.status &&
      FINISHED_STATUSES.has(s.status) &&
      s.sessionId !== selectedSessionId
    ) {
      finished.push(s.sessionId);
    }
    next[s.sessionId] = s.status;
  }
  return { finished, next };
}

/**
 * Stable pinned-first reorder for optimistic pin/unpin. Pinned sessions move
 * to the top in their original relative order; everything else keeps the
 * incoming (desktop-sorted) order. Matches the desktop sidebar's `pinned DESC`
 * grouping, which the pushed snapshot also converges on.
 */
export function sortPinnedFirst(list: RemoteSession[]): RemoteSession[] {
  const pinned = list.filter(session => session.pinned);
  const rest = list.filter(session => !session.pinned);
  return [...pinned, ...rest];
}
