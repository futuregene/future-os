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

/**
 * What became of a row that left the queue without running.
 *
 * A row can go without this phone asking: the session folds a queued prompt into
 * the turn ahead of it (`merged`), the reader takes it back (`cancelled`), a send
 * that names `supersede_session` replaces it (`superseded`), or the conversation
 * is deleted with it still waiting (`session_deleted`). The desktop reports each
 * of those for a while after it happens. A screen that reads only the waiting
 * rows sees one disappear and cannot tell a merge from a deletion, so the reader
 * is left believing a message went out that never ran.
 */
export interface SessionQueueAck {
  runId: string;
  /** `terminal` for every ack the desktop keeps. */
  state: string;
  /** `merged` | `cancelled` | `superseded` | `session_deleted`, or anything new. */
  reason: string;
}

/** The queue of one conversation as the desktop reports it. */
export interface SessionQueueSnapshot {
  rows: SessionQueueRow[];
  acks: SessionQueueAck[];
}

/** One sentence about a row that will not run, for the panel to show. */
export interface SessionQueueNotice {
  reason: string;
  count: number;
}

/**
 * How long a notice stays on screen.
 *
 * The acks stay in the desktop's window, so a notice left alone would repeat
 * itself on every read. It is news, not state: it says its piece once and goes.
 */
export const QUEUE_NOTICE_MS = 12_000;

/** The i18n suffix a reason is drawn as, or `other` for one with no sentence yet. */
export function sessionQueueNoticeReason(
  reason: string,
): "merged" | "cancelled" | "superseded" | "session_deleted" | "other" {
  switch (reason) {
    case "merged":
    case "cancelled":
    case "superseded":
    case "session_deleted":
      return reason;
    default:
      return "other";
  }
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
 * The rows waiting in this conversation's queue on the desktop, and the reasons
 * rows left it.
 *
 * An empty session id (a draft that has not been sent yet) has no queue on the
 * desktop, and asking about one would answer for "whatever the desktop calls
 * this". The panel reads empty, which is the truth.
 */
export async function fetchSessionQueue(
  client: RemoteClient,
  sessionId: string,
): Promise<SessionQueueSnapshot> {
  if (!sessionId.trim())
    return { acks: [], rows: [] };
  const reply = await client.requestRetry<{ queued?: unknown; recentTerminalAcks?: unknown }>(
    { type: "get_queue", sessionId },
    sessionId,
  );
  const queued = reply.data?.queued;
  const acks = reply.data?.recentTerminalAcks;
  return {
    acks: Array.isArray(acks)
      ? acks.map(asAck).filter((ack): ack is SessionQueueAck => ack !== null)
      : [],
    rows: Array.isArray(queued)
      ? queued.map(asRow).filter((row): row is SessionQueueRow => row !== null)
      : [],
  };
}

/** One ack as the desktop reports it, or null when it cannot be explained. */
function asAck(raw: unknown): SessionQueueAck | null {
  if (!raw || typeof raw !== "object")
    return null;
  const ack = raw as Record<string, unknown>;
  // An ack with no run id or no reason cannot be told to the reader, so it is
  // not announced as "something happened".
  if (typeof ack.runId !== "string" || !ack.runId)
    return null;
  if (typeof ack.reason !== "string" || !ack.reason)
    return null;
  return {
    reason: ack.reason,
    runId: ack.runId,
    state: typeof ack.state === "string" && ack.state ? ack.state : "terminal",
  };
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
   * Read this conversation's rows (and what recently left it) from the desktop
   * (the context's `getQueue`). Held in a ref: it is re-read on every poll, and a
   * caller that rebuilds the function each render must not restart the timer.
   */
  read: () => Promise<SessionQueueSnapshot>;
  /** The conversation whose queue this is. Empty means "no conversation". */
  sessionId: string;
  /** A run is in flight or a send is on its way: worth reading again. */
  live: boolean;
  /** The desktop is reachable and this phone drives it. */
  enabled: boolean;
}

export interface SessionQueueView {
  rows: SessionQueueRow[];
  /** A row that left the queue without running, in one sentence; null when none has. */
  notice: SessionQueueNotice | null;
  /** Re-read now: after a cancel, a release, or a send. */
  refresh: () => void;
  /**
   * This screen removed `runId` itself, so its ack is not news.
   *
   * The bin is here: a notice that repeats the reader's own tap would read as a
   * fault. Rows removed anywhere else — the desktop's window, another phone, the
   * session folding a burst into one turn, a deleted conversation — are
   * announced.
   */
  forget: (runId: string) => void;
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
  const [notice, setNotice] = useState<SessionQueueNotice | null>(null);
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
  // Acks already told, and acks this screen caused itself: identity, not state.
  const toldRef = useRef(new Set<string>());
  const forgottenRef = useRef(new Set<string>());
  // The conversation whose first answer has been read: the desktop's window
  // holds acks from before this screen opened, and those are not news.
  const primedRef = useRef<string | null>(null);
  const rows = state.sessionId === sessionId ? state.rows : [];
  const polling = enabled && !!sessionId && (live || rows.length > 0);
  const refresh = useCallback(() => setRevision(value => value + 1), []);
  const forget = useCallback((runId: string) => {
    forgottenRef.current.add(runId);
  }, []);

  // A notice is news: it says its piece and goes, so an ack still inside the
  // desktop's window cannot keep the sentence on screen.
  useEffect(() => {
    if (!notice)
      return;
    const timer = setTimeout(() => setNotice(null), QUEUE_NOTICE_MS);
    return () => clearTimeout(timer);
  }, [notice]);

  const readOnce = useCallback(async () => {
    try {
      const next = await readRef.current();
      // Kept with the session it was read for, so a read that lands after the
      // reader moved on cannot draw the previous queue under this composer.
      const current = latestRef.current;
      if (current.sessionId !== sessionId || !sameRows(current.rows, next.rows)) {
        const updated = { rows: next.rows, sessionId };
        latestRef.current = updated;
        setState(updated);
      }
      const fresh = next.acks.filter(ack =>
        !forgottenRef.current.has(ack.runId) && !toldRef.current.has(`${ack.runId}:${ack.reason}`));
      for (const ack of next.acks)
        toldRef.current.add(`${ack.runId}:${ack.reason}`);
      if (primedRef.current !== sessionId) {
        // Everything in the desktop's window belongs to before this screen.
        primedRef.current = sessionId;
        setNotice(null);
        return;
      }
      if (fresh.length > 0) {
        setNotice({
          count: fresh.length,
          reason: fresh[fresh.length - 1]!.reason,
        });
      }
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

  return { forget, notice, refresh, rows };
}
