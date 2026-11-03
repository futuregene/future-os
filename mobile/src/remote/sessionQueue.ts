import { useCallback, useEffect, useRef, useState } from "react";
import type { RemoteClient } from "./client";

/**
 * The conversation's queue, read from the desktop that owns it.
 *
 * The phone used to hold its own queue: a message written while a reply streamed
 * waited in memory (and in AsyncStorage) until this screen saw the run end, and
 * then went out joined with the rest. That copy could only ever be a guess about
 * a conversation that lives on a paired machine — a queue is the session's, and
 * the session has one list whichever device filled it.
 *
 * So a send while a run is in flight goes to the desktop at once with a busy
 * policy (`enqueue_coalescing`), the session holds it on a durable receipt, and
 * the rows below are the desktop's own answer (`get_queue`: `get_state`'s
 * `queuedRuns`, normalised by the host). Cancelling one and starting a parked one
 * early are the Agent's commands too, addressed by run id.
 */

/** One message the session holds behind its running turn. */
export interface SessionQueueRow {
  runId: string;
  /** 1-based place in the queue, as the session reports it. */
  position: number;
  /** When the session accepted it. */
  acceptedAt: string;
  /** The message as the session holds it (empty for an attachment-only send). */
  text: string;
  /**
   * `queued` runs at the turn boundary. `stale…` waited too long: the session
   * parks it and waits for a person to start it, rather than spending credits on
   * a message the conversation has moved past.
   */
  state: string;
}

/** Whether a row is parked rather than about to run by itself. */
export function isParkedSessionQueueRow(row: SessionQueueRow): boolean {
  return row.state.startsWith("stale");
}

function asRow(raw: unknown): SessionQueueRow | null {
  if (!raw || typeof raw !== "object")
    return null;
  const row = raw as Record<string, unknown>;
  if (typeof row.runId !== "string" || !row.runId)
    return null;
  return {
    acceptedAt: typeof row.acceptedAt === "string" ? row.acceptedAt : "",
    position: typeof row.position === "number" ? row.position : 0,
    runId: row.runId,
    state: typeof row.state === "string" && row.state ? row.state : "queued",
    text: typeof row.text === "string" ? row.text : "",
  };
}

/**
 * The rows waiting in this conversation's queue on the desktop.
 *
 * An empty session id (a draft that has not been sent yet) has no queue on the
 * desktop, and asking about one would answer for "whatever the desktop calls
 * this". The panel reads empty, which is the truth.
 */
export async function fetchSessionQueue(
  client: RemoteClient,
  sessionId: string,
): Promise<SessionQueueRow[]> {
  if (!sessionId.trim())
    return [];
  const reply = await client.requestRetry<{ queued?: unknown }>(
    { type: "get_queue", sessionId },
    sessionId,
  );
  const queued = reply.data?.queued;
  if (!Array.isArray(queued))
    return [];
  return queued
    .map(asRow)
    .filter((row): row is SessionQueueRow => row !== null);
}

/** Take one waiting message back out, before it leaves. */
export async function cancelSessionQueuedRun(
  client: RemoteClient,
  sessionId: string,
  runId: string,
): Promise<void> {
  await client.requestRetry<null>(
    { type: "cancel_queued_run", runId, sessionId },
    sessionId,
  );
}

/** Start a parked (or any waiting) row now, because the reader asked for it. */
export async function releaseSessionQueuedRun(
  client: RemoteClient,
  sessionId: string,
  runId: string,
): Promise<void> {
  await client.requestRetry<null>(
    { type: "release_queued_run", runId, sessionId },
    sessionId,
  );
}

/**
 * How often the panel re-reads a queue that has something in it.
 *
 * A queue has no event of its own — the desktop announces a run starting and
 * ending, never "this waiting row changed" — so a panel that is showing rows (or
 * a run that can still fill one) polls. An idle conversation with an empty queue
 * asks nothing at all, and a send refreshes explicitly.
 */
export const QUEUE_POLL_MS = 2000;

/** Whether two reads describe the same queue, so a poll can leave the screen alone. */
function sameRows(left: readonly SessionQueueRow[], right: readonly SessionQueueRow[]): boolean {
  if (left.length !== right.length)
    return false;
  return left.every((row, index) => {
    const other = right[index];
    return other !== undefined
      && row.runId === other.runId
      && row.state === other.state
      && row.position === other.position
      && row.text === other.text;
  });
}

export interface SessionQueueViewOptions {
  /**
   * Read this conversation's rows from the desktop (the context's `getQueue`).
   * Held in a ref: it is re-read on every poll, and a caller that rebuilds the
   * function each render must not restart the timer.
   */
  read: () => Promise<SessionQueueRow[]>;
  /** The conversation whose queue this is. Empty means "no conversation". */
  sessionId: string;
  /** A run is in flight or a send is on its way: worth reading again. */
  live: boolean;
  /** The desktop is reachable and this phone drives it. */
  enabled: boolean;
}

export interface SessionQueueView {
  rows: SessionQueueRow[];
  /** Re-read now: after a cancel, a release, or a send. */
  refresh: () => void;
}

/**
 * Subscribe a view to one conversation's queue on the desktop.
 *
 * The rows are kept with the session they were read for, so a read that lands
 * after the reader moved on cannot draw the previous conversation's queue under
 * the next one's composer.
 */
export function useSessionQueue({
  read,
  sessionId,
  live,
  enabled,
}: SessionQueueViewOptions): SessionQueueView {
  const [state, setState] = useState<{ sessionId: string; rows: SessionQueueRow[] }>({
    rows: [],
    sessionId: "",
  });
  const [revision, setRevision] = useState(0);
  const readRef = useRef(read);
  useEffect(() => {
    readRef.current = read;
  }, [read]);
  // What is on screen, readable from a poll that resolves between renders. Kept
  // beside the state so an answer that says what is already there schedules no
  // update at all: a 2 s poll of a queue that is not moving must not re-render
  // the screen (and a phone that is only watching costs nothing).
  const latestRef = useRef(state);
  const rows = state.sessionId === sessionId ? state.rows : [];
  const polling = enabled && !!sessionId && (live || rows.length > 0);
  const refresh = useCallback(() => setRevision(value => value + 1), []);

  const readOnce = useCallback(async () => {
    try {
      const next = await readRef.current();
      // Kept with the session it was read for, so a read that lands after the
      // reader moved on cannot draw the previous queue under this composer.
      const current = latestRef.current;
      if (current.sessionId === sessionId && sameRows(current.rows, next))
        return;
      const updated = { rows: next, sessionId };
      latestRef.current = updated;
      setState(updated);
    }
    catch {
      // A queue that cannot be read is not an error to show: the panel keeps
      // what it has, and the next tick tries again.
    }
  }, [sessionId]);

  // One read when the conversation, its reachability, the run's own state or the
  // refresh counter changes. The timer is a separate effect, so the read that
  // makes `polling` true does not immediately read again.
  useEffect(() => {
    if (!enabled || !sessionId)
      return;
    void readOnce();
  }, [enabled, live, readOnce, revision, sessionId]);

  useEffect(() => {
    if (!enabled || !sessionId || !polling)
      return;
    const timer = setInterval(() => void readOnce(), QUEUE_POLL_MS);
    return () => clearInterval(timer);
  }, [enabled, polling, readOnce, sessionId]);

  return { refresh, rows };
}
