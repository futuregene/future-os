import { requestRemotePeer } from "./remotePeerClient";

/**
 * One reasoning block's text, read from the host when its row is opened.
 *
 * This client declares the trimmed lane (`lean_events_v1`), which sends a
 * reasoning row's boundaries and not its body — the reply's largest payload that
 * nobody has opened yet. `ThinkingBlock` therefore draws the row from the
 * boundary alone and asks for the text the first time a reader expands it, which
 * keeps opening a conversation as cheap as it was and pays only for the rows
 * someone actually read.
 *
 * The identity is the block's **index among the run's reasoning blocks** (the
 * order the reply renders them), because that is the only thing a client that
 * was sent boundaries alone can address: it has no entry id or content key for
 * the block. The host resolves it against the run's entries.
 *
 * A `null` answer means the block is not readable — a run still writing it, or
 * an index the run does not have. That is not an error: the row says so rather
 * than pretending the reasoning was empty.
 */

interface ReasoningBody {
  text?: string | null;
}

/**
 * One fetched body per `(desktop, session, run, index)`.
 *
 * Cached because a transcript re-renders on every streaming push: a row that
 * refetched its reasoning each time would turn reading a conversation into a
 * request storm. The value is a *promise* so two rows asking at once make one
 * request, and so a miss or a failure is remembered rather than retried on every
 * render — the row can be collapsed and reopened without another round trip.
 */
const bodies = new Map<string, Promise<string | null>>();

function cacheKey(desktopId: string, sessionId: string, runId: string, index: number): string {
  return `${desktopId}::${sessionId}::${runId}::${index}`;
}

export function fetchRemoteReasoning(input: {
  desktopId: string;
  sessionId: string;
  runId: string;
  index: number;
  /**
   * Whether the reply has stopped. Only a settled block's read is cached: one
   * still being written keeps growing, so caching it would freeze a prefix and
   * show it as the whole thought.
   */
  settled?: boolean;
}): Promise<string | null> {
  const key = cacheKey(input.desktopId, input.sessionId, input.runId, input.index);
  if (input.settled !== false) {
    const cached = bodies.get(key);
    if (cached)
      return cached;
  }
  const pending = requestRemotePeer<ReasoningBody>(
    input.desktopId,
    {
      type: "get_reasoning",
      sessionId: input.sessionId,
      runId: input.runId,
      offset: input.index,
    },
    input.sessionId,
  ).then((data) => {
    const text = typeof data?.text === "string" ? data.text : "";
    return text.trim().length > 0 ? text : null;
  }).catch(() => {
    // A failure is remembered as "not readable": the row says so, and a
    // transcript that retried a failed read per render would be worse than one
    // that shows less.
    return null;
  });
  if (input.settled !== false)
    bodies.set(key, pending);
  return pending;
}

/** Drop a conversation's cached bodies, so a fresh open re-reads them. */
export function forgetRemoteReasonings(desktopId: string, sessionId: string): void {
  const prefix = `${desktopId}::${sessionId}::`;
  for (const key of [...bodies.keys()]) {
    if (key.startsWith(prefix))
      bodies.delete(key);
  }
}
