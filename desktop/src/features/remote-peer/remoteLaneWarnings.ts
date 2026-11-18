/**
 * What the remote lane did with a run, said out loud.
 *
 * A remote reply can stop growing — or be drawn from the host's record while the
 * host is still writing it — through several doors, and every one of them used to
 * be silent: a fragment dropped when its terminal frame arrived, a content lane
 * that failed and took the frames still in it, a run log the host refused or a
 * read that errored, a record that declared a run complete while the host still
 * had it executing. From the outside all four look the same — "my answer is
 * truncated" or "this conversation looks finished but it isn't" — and the reader
 * has no way to say which happened, so the report arrives without the one fact
 * that decides where the defect is.
 *
 * This is that fact, as one console line per event. Not a UI: the reader is not
 * asked to diagnose their own transcript, and a notice in the transcript would
 * have to explain "the run's beginning could not be read" in a place that is
 * about the answer. What it is for is the developer who gets the report — the
 * same role `warnLivePreviewFallback` plays for this app's own runs, with the
 * same shape: a bracketed prefix, one warning per run and reason, and the error
 * passed through so its stack survives.
 *
 * One warning per (conversation, reason, run): a link that flap-restarts the lane
 * must not print a line per frame, and the second identical failure inside one
 * visit adds nothing to the first.
 */

/** One thing the lane did that the reader cannot see from the transcript. */
export interface RemoteLaneWarning {
  /** The run this is about. Absent for a lane-wide event (a reset, a refusal). */
  runId?: string | null;
  /** The error that ended a read, when there was one. */
  error?: unknown;
}

const warned = new Set<string>();

/**
 * An upper bound on that memory.
 *
 * It is per conversation and per run, so reaching it takes a session of hundreds
 * of runs; when it does, forgetting everything costs at most one repeated line
 * per reason still in play.
 */
const WARNED_LIMIT = 512;

/**
 * Say what happened to a run's live reply.
 *
 * `desktopId` and `sessionId` are in the message on purpose: a window can have
 * several remote conversations open, and a line that names none of them is not
 * much use in a log of the report.
 */
export function warnRemoteLane(
  desktopId: string,
  sessionId: string,
  reason: string,
  warning: RemoteLaneWarning = {},
): void {
  const key = `${desktopId}\u0000${sessionId}\u0000${reason}\u0000${warning.runId ?? ""}`;
  if (warned.has(key))
    return;
  if (warned.size >= WARNED_LIMIT)
    warned.clear();
  warned.add(key);
  const run = warning.runId ? ` for run ${warning.runId}` : "";
  const detail = warning.error === undefined ? "" : ` (${describe(warning.error)})`;
  const message = `[remote] ${reason}${run} — ${sessionId}@${desktopId}${detail}`;
  // The value goes through as well when it is an error: the text above is what a
  // reader greps for, and the object is what carries its stack.
  if (warning.error instanceof Error)
    console.warn(message, warning.error);
  else
    console.warn(message);
}

/** Forget what was already said about one conversation (it is being dropped). */
export function forgetRemoteLaneWarnings(desktopId: string, sessionId: string): void {
  const prefix = `${desktopId}\u0000${sessionId}\u0000`;
  for (const key of warned) {
    if (key.startsWith(prefix))
      warned.delete(key);
  }
}

/** An error (or anything else thrown) as one line of text. */
function describe(value: unknown): string {
  if (value instanceof Error)
    return value.message;
  if (typeof value === "string")
    return value;
  try {
    return JSON.stringify(value) ?? String(value);
  }
  catch {
    return String(value);
  }
}
