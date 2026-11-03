import { useCallback, useEffect, useRef, useState } from "react";
import { invokeCommand } from "../../integrations/tauri/invoke";

/**
 * The conversation's queue, read from the Agent instead of remembered here.
 *
 * A message written while a reply streams is sent at once and held by the
 * session (`handleQueuedSend`, `enqueue_coalescing`), so the rows below are the
 * Agent's own: the same list every client on the conversation sees, in the
 * Agent's order, and still there after this window is gone. Anything the GUI
 * kept locally could only be a guess about that.
 *
 * Reading is `session_queue` (the Host's `get_queue`: `get_state`'s
 * `queuedRuns`, normalised); removing and starting one early go through the
 * Agent's own commands, addressed by run id.
 */
export interface AgentQueueRow {
  runId: string;
  /** 1-based place in the queue, as the Agent reports it. */
  position: number;
  /** When the Agent accepted it. */
  acceptedAt: string;
  /** The message as the Agent holds it (empty for an attachment-only send). */
  text: string;
  /**
   * `queued` runs at the turn boundary. `stale…` waited too long — the Agent
   * parks it and waits for a person to start it rather than spending credits on
   * a message the conversation has moved past.
   */
  state: string;
}

interface QueueSnapshot {
  activeRunId: string | null;
  queued: AgentQueueRow[];
  recentTerminalAcks: { runId: string; state: string; reason: string }[];
}

/** Whether a row is parked rather than about to run by itself. */
export function isParkedRow(row: AgentQueueRow): boolean {
  return row.state.startsWith("stale");
}

function asRow(raw: unknown): AgentQueueRow | null {
  if (!raw || typeof raw !== "object")
    return null;
  const row = raw as Record<string, unknown>;
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

/** The rows waiting in this conversation's queue on the Agent. */
export async function fetchAgentQueue(threadId: string): Promise<AgentQueueRow[]> {
  const snapshot = await invokeCommand<Partial<QueueSnapshot>>("session_queue", { threadId });
  const queued = Array.isArray(snapshot?.queued) ? snapshot.queued : [];
  return queued
    .map(asRow)
    .filter((row): row is AgentQueueRow => row !== null);
}

/** Take one waiting message back out, before it leaves. */
export function cancelAgentQueuedRun(runId: string): Promise<void> {
  return invokeCommand<void>("cancel_queued_run", { runId });
}

/** Start a parked (or any waiting) row now, because the reader asked for it. */
export function releaseAgentQueuedRun(runId: string): Promise<void> {
  return invokeCommand<void>("release_queued_run", { runId });
}

/**
 * How often the panel re-reads the queue while there is something to read.
 *
 * The Agent announces a prompt's acceptance and its start, but not "this row
 * changed" — a queue has no event of its own — so a live panel polls. It only
 * polls while the conversation is busy or the panel is showing rows; an idle
 * conversation with an empty queue costs nothing.
 */
const QUEUE_POLL_MS = 2000;

export interface AgentQueueState {
  rows: AgentQueueRow[];
  /** Re-read now (after a send, a removal, or a start). */
  refresh: () => void;
}

/**
 * Subscribe to one conversation's Agent-side queue.
 *
 * `live` keeps the poll running while a run is in flight; rows already waiting
 * keep it running by themselves, because a queue that is not empty can change
 * (a row starts, a row is parked) without this view doing anything. With nothing
 * waiting and nothing streaming there is nothing to poll for, and the next send
 * refreshes explicitly.
 */
export function useAgentQueue(threadId: string | null, live: boolean): AgentQueueState {
  const [rows, setRows] = useState<AgentQueueRow[]>([]);
  const [revision, setRevision] = useState(0);
  // A late answer for a conversation this view already left must not land in the
  // next one's panel. Updated in an effect, not during render: the ref only has
  // to be right by the time an answer arrives, and an answer always arrives after
  // the render that switched conversations committed.
  const threadRef = useRef(threadId);
  useEffect(() => {
    threadRef.current = threadId;
  }, [threadId]);
  const polling = live || rows.length > 0;

  const refresh = useCallback(() => setRevision(value => value + 1), []);

  useEffect(() => {
    if (!threadId) {
      setRows([]);
      return;
    }
    let cancelled = false;
    const read = async () => {
      try {
        const next = await fetchAgentQueue(threadId);
        if (!cancelled && threadRef.current === threadId)
          setRows(next);
      }
      catch {
        // A queue that cannot be read is not an error to show: the panel simply
        // keeps what it has, and the next tick tries again.
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
  }, [polling, revision, threadId]);

  return { refresh, rows };
}
