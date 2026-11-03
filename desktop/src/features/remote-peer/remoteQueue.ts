import { useCallback, useEffect, useRef, useState } from "react";
import { requestRemotePeer } from "./remotePeerClient";

/**
 * The queue of a conversation on a paired computer, read from *that host's*
 * Agent instead of remembered here.
 *
 * This is the client-role twin of `features/agent/agentQueue`: the same rows, the
 * same commands, one machine away. A message written while a reply streams is
 * sent at once with a `busy_policy` and held by the *session* (`enqueue_
 * coalescing`), so it exists on the machine that will run it — visible to every
 * client looking at that conversation, and still there after this window is
 * gone. Anything held in this app's memory or localStorage could only be a guess
 * about that queue, and would be lost with the window it lived in.
 *
 * Reading is the host's `get_queue` (its `get_state`'s `queuedRuns`,
 * normalised — `host/src/agent_bridge/queue.rs`); removing and starting one early
 * go through `cancel_queued_run` / `release_queued_run`, addressed by run id.
 * There is no "edit in place": changing a waiting message is a cancel and a
 * re-send, which is what the composer does.
 */

/**
 * The policy a send made behind a running reply names.
 *
 * `enqueue_coalescing` rather than `enqueue_if_busy`: the session folds the
 * consecutive prompts queued this way into one turn (blank-line joined), which is
 * the "send the queue as one message" behaviour the clients used to implement
 * themselves. The local GUI path names the same value (`useSendMessage.ts`).
 */
export const QUEUED_SEND_POLICY = "enqueue_coalescing";

/** One message waiting behind a conversation's run, as the host reports it. */
export interface RemoteQueueRow {
  runId: string;
  /** 1-based place in the queue, as the Agent reports it. */
  position: number;
  /** When the Agent accepted it. */
  acceptedAt: string;
  /** The message as the Agent holds it (empty for an attachment-only send). */
  text: string;
  /**
   * `queued` runs at the turn boundary. `stale…` waited too long: the Agent
   * parked it rather than spend credits on a message the conversation moved past,
   * and it now starts only when a person says so.
   */
  state: string;
}

/** Whether a row is parked rather than about to run by itself. */
export function isParkedRemoteQueueRow(row: RemoteQueueRow): boolean {
  return row.state.startsWith("stale");
}

function asRow(raw: unknown): RemoteQueueRow | null {
  if (!raw || typeof raw !== "object")
    return null;
  const row = raw as Record<string, unknown>;
  // A row without a run id is one this client could neither remove nor start, so
  // it is dropped rather than drawn as a message with no way back out.
  if (typeof row.runId !== "string" || !row.runId)
    return null;
  return {
    runId: row.runId,
    position: typeof row.position === "number" ? row.position : 0,
    acceptedAt: typeof row.acceptedAt === "string" ? row.acceptedAt : "",
    text: typeof row.text === "string" ? row.text : "",
    state: typeof row.state === "string" && row.state ? row.state : "queued",
  };
}

/** The waiting rows of a host's `get_queue` reply. */
export function parseRemoteQueueRows(raw: unknown): RemoteQueueRow[] {
  const rows = Array.isArray(raw)
    ? raw
    : Array.isArray((raw as Record<string, unknown> | null)?.queued)
      ? (raw as { queued: unknown[] }).queued
      : [];
  return rows
    .map(asRow)
    .filter((row): row is RemoteQueueRow => row !== null);
}

/**
 * The rows waiting in this conversation's queue on that host.
 *
 * Sent on the conversation's own lane, like `get_state` and `abort`: a queue
 * belongs to one of the host's sessions, and the host resolves the lane's subject
 * itself. An empty session id is a draft, which has nothing queued — the host
 * answers an empty queue for it, and asking costs a round trip for a conversation
 * that does not exist yet, so the hook below skips it.
 */
export async function getRemoteQueue(
  desktopId: string,
  sessionId: string,
): Promise<RemoteQueueRow[]> {
  const raw = await requestRemotePeer<Record<string, unknown> | null>(
    desktopId,
    { type: "get_queue", sessionId },
    sessionId,
  );
  return parseRemoteQueueRows(raw?.queued);
}

/** Take one waiting message back out, on the host that holds it. */
export async function cancelRemoteQueuedRun(
  desktopId: string,
  sessionId: string,
  runId: string,
): Promise<void> {
  await requestRemotePeer(desktopId, { type: "cancel_queued_run", sessionId, runId }, sessionId);
}

/**
 * Start a parked (or any waiting) row now, because the reader asked for it.
 *
 * "Now" is still the host's decision: the row starts at the next turn boundary,
 * which for a parked row is as soon as the session is free.
 */
export async function releaseRemoteQueuedRun(
  desktopId: string,
  sessionId: string,
  runId: string,
): Promise<void> {
  await requestRemotePeer(desktopId, { type: "release_queued_run", sessionId, runId }, sessionId);
}

/**
 * How often the panel re-reads the queue while there is something to read.
 *
 * The host announces a prompt's acceptance and its start, but not "this row
 * changed" — a queue has no event of its own — so a live panel polls. It polls
 * only while that conversation is busy or the panel is showing rows; an idle
 * conversation with an empty queue costs nothing, and the next send refreshes
 * explicitly.
 */
const QUEUE_POLL_MS = 2000;

export interface RemoteQueueState {
  rows: RemoteQueueRow[];
  /** Re-read now (after a send, a removal, an edit or a start). */
  refresh: () => void;
}

/**
 * Subscribe to one conversation's queue on one host.
 *
 * Keyed by desktop *and* session, like the addresses the rest of this feature
 * uses: the same session id on two machines is two different conversations, and
 * one host's rows must never be drawn in the other's panel. A late answer for a
 * conversation this view already left is dropped rather than landing in the next
 * one's panel.
 *
 * `live` keeps the poll running while a run is in flight; rows already waiting
 * keep it running by themselves, because a queue that is not empty can change (a
 * row starts, a row is parked) without this view doing anything.
 */
export function useRemoteQueue(
  desktopId: string,
  sessionId: string,
  live: boolean,
): RemoteQueueState {
  const [rows, setRows] = useState<RemoteQueueRow[]>([]);
  const [revision, setRevision] = useState(0);
  // The conversation the answer must belong to, updated in an effect: the ref
  // only has to be right by the time an answer arrives, and an answer always
  // arrives after the render that switched conversations committed.
  const addressRef = useRef(`${desktopId}:${sessionId}`);
  useEffect(() => {
    addressRef.current = `${desktopId}:${sessionId}`;
  }, [desktopId, sessionId]);
  const polling = live || rows.length > 0;

  const refresh = useCallback(() => setRevision(value => value + 1), []);

  useEffect(() => {
    // A draft has no conversation on the host to hold a queue, so there is
    // nothing to ask about and nothing to show.
    if (!sessionId) {
      setRows([]);
      return;
    }
    const address = `${desktopId}:${sessionId}`;
    let cancelled = false;
    const read = async () => {
      try {
        const next = await getRemoteQueue(desktopId, sessionId);
        if (!cancelled && addressRef.current === address)
          setRows(next);
      }
      catch {
        // A queue that cannot be read is not an error to show: a link that is
        // down has its own notice above the composer. The panel keeps what it
        // has, and the next tick tries again.
      }
    };
    void read();
    if (!polling) {
      return () => {
        cancelled = true;
      };
    }
    const timer = setInterval(() => void read(), QUEUE_POLL_MS);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [desktopId, polling, revision, sessionId]);

  return { refresh, rows };
}
