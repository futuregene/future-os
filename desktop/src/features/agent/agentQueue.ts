import { useCallback, useEffect, useRef, useState } from "react";
import { invokeCommand } from "../../integrations/tauri/invoke";

/**
 * One file a waiting message carries, as the queue reports it.
 *
 * `path` is this machine's own path (the Agent runs here), which is also the
 * identity a later re-send names.
 */
export interface AgentQueueAttachment {
  path: string;
  /** "image" | "file". */
  kind: string;
  name: string;
  /** Byte size, or 0 when the Agent did not report one. */
  size: number;
}

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
   * The files the message carries.
   *
   * `undefined` means this queue does not report them (an Agent older than the
   * field): the panel then says an edit sends the words alone rather than
   * claiming a message carries none. A message with no files reports `[]`.
   */
  attachments?: AgentQueueAttachment[];
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
  recentTerminalAcks: QueueAck[];
}

/**
 * What became of a row that left the queue without running.
 *
 * The Agent reports those with the queue itself (`recentTerminalAcks`): folded
 * into the turn ahead (`merged`), taken back by a reader (`cancelled`), replaced
 * by a send that names `supersede_session` (`superseded`), or gone with a deleted
 * conversation (`session_deleted`). Reading only the waiting rows lets one
 * disappear with nobody told.
 */
export interface QueueAck {
  runId: string;
  state: string;
  reason: string;
}

/** The queue of one conversation: what waits, and what just left it. */
export interface AgentQueueSnapshot {
  rows: AgentQueueRow[];
  acks: QueueAck[];
}

/** One sentence about a row that will not run, for the composer to show. */
export interface AgentQueueNotice {
  reason: string;
  count: number;
}

/**
 * How long a notice stays on screen.
 *
 * The acks stay in the Agent's window, so a notice left alone would repeat itself
 * on every read. It is news, not state: it says its piece once and goes.
 */
export const QUEUE_NOTICE_MS = 12_000;

/** The i18n suffix a reason is drawn as, or `other` for one with no sentence yet. */
export function agentQueueNoticeReason(
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

/** Whether a row is parked rather than about to run by itself. */
export function isParkedRow(row: AgentQueueRow): boolean {
  return row.state.startsWith("stale");
}

/** One reported file, or null when it has no path to address or re-send. */
function asAttachment(raw: unknown): AgentQueueAttachment | null {
  if (!raw || typeof raw !== "object")
    return null;
  const attachment = raw as Record<string, unknown>;
  if (typeof attachment.path !== "string" || !attachment.path)
    return null;
  return {
    kind: typeof attachment.kind === "string" ? attachment.kind : "",
    name: typeof attachment.name === "string" && attachment.name
      ? attachment.name
      : attachment.path.split(/[\\/]/).pop() || attachment.path,
    path: attachment.path,
    size: typeof attachment.size === "number" && attachment.size > 0 ? attachment.size : 0,
  };
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
    attachments: Array.isArray(row.attachments)
      ? row.attachments.map(asAttachment).filter(
          (attachment): attachment is AgentQueueAttachment => attachment !== null,
        )
      : undefined,
  };
}

/** One ack as `session_queue` reports it, or null when it cannot be explained. */
function asAck(raw: unknown): QueueAck | null {
  if (!raw || typeof raw !== "object")
    return null;
  const ack = raw as Record<string, unknown>;
  // An ack with no run id or reason cannot be explained, so it is not announced
  // as "something happened".
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

/** The queue's rows and the reasons rows left it, as `session_queue` answers. */
export async function fetchAgentQueue(threadId: string): Promise<AgentQueueSnapshot> {
  const snapshot = await invokeCommand<Partial<QueueSnapshot>>("session_queue", { threadId });
  const queued = Array.isArray(snapshot?.queued) ? snapshot.queued : [];
  const acks = Array.isArray(snapshot?.recentTerminalAcks) ? snapshot.recentTerminalAcks : [];
  return {
    acks: acks.map(asAck).filter((ack): ack is QueueAck => ack !== null),
    rows: queued.map(asRow).filter((row): row is AgentQueueRow => row !== null),
  };
}

/**
 * Take one waiting message back out, before it leaves.
 *
 * The conversation travels with the run: a row can be in the Agent's queue that
 * this app never recorded a run for — a goal-plane note, a prompt another client
 * sent straight to the Agent — and the conversation is then the only thing that
 * says which session to take it back from.
 */
export function cancelAgentQueuedRun(threadId: string, runId: string): Promise<void> {
  return invokeCommand<void>("cancel_queued_run", { threadId, runId });
}

/** Start a parked (or any waiting) row now, because the reader asked for it. */
export function releaseAgentQueuedRun(threadId: string, runId: string): Promise<void> {
  return invokeCommand<void>("release_queued_run", { threadId, runId });
}

/**
 * Correct one waiting message's words, in the place it already holds.
 *
 * Not a removal and a re-send: the queue is an order the reader built, and a
 * re-send enters at the back — a typo fixed in the second of four messages would
 * have been sent after the fourth. The Agent replaces the accepted prompt's text
 * where it waits; everything else about that prompt stays as it was accepted.
 */
export function editAgentQueuedRun(
  threadId: string,
  runId: string,
  message: string,
): Promise<void> {
  return invokeCommand<void>("edit_queued_run", { threadId, runId, message });
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
  /** A row that left the queue without running, in one sentence; null when none has. */
  notice: AgentQueueNotice | null;
  /** Re-read now (after a send, a removal, or a start). */
  refresh: () => void;
  /**
   * This window removed `runId` itself, so its ack is not news.
   *
   * A notice repeating the reader's own click reads as a fault; rows removed
   * anywhere else — another client, the Agent folding a burst into one turn, a
   * deleted conversation — are announced.
   */
  forget: (runId: string) => void;
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
  const [notice, setNotice] = useState<AgentQueueNotice | null>(null);
  const [revision, setRevision] = useState(0);
  // A late answer for a conversation this view already left must not land in the
  // next one's panel. Updated in an effect, not during render: the ref only has
  // to be right by the time an answer arrives, and an answer always arrives after
  // the render that switched conversations committed.
  const threadRef = useRef(threadId);
  useEffect(() => {
    threadRef.current = threadId;
  }, [threadId]);
  // Acks already told, and acks this window caused itself: identity, not state.
  const toldRef = useRef(new Set<string>());
  const forgottenRef = useRef(new Set<string>());
  // The conversation whose first answer has been read: the Agent's window holds
  // acks from before this view existed, and those are not news.
  const primedRef = useRef<string | null>(null);
  const polling = live || rows.length > 0;

  const refresh = useCallback(() => setRevision(value => value + 1), []);
  const forget = useCallback((runId: string) => {
    forgottenRef.current.add(runId);
  }, []);

  // A notice is news: it says its piece and goes.
  useEffect(() => {
    if (!notice)
      return;
    const timer = setTimeout(setNotice, QUEUE_NOTICE_MS, null);
    return () => clearTimeout(timer);
  }, [notice]);

  useEffect(() => {
    if (!threadId) {
      setRows([]);
      setNotice(null);
      return;
    }
    let cancelled = false;
    const read = async () => {
      try {
        const next = await fetchAgentQueue(threadId);
        if (cancelled || threadRef.current !== threadId)
          return;
        setRows(next.rows);
        const fresh = next.acks.filter(ack =>
          !forgottenRef.current.has(ack.runId) && !toldRef.current.has(`${ack.runId}:${ack.reason}`));
        for (const ack of next.acks)
          toldRef.current.add(`${ack.runId}:${ack.reason}`);
        if (primedRef.current !== threadId) {
          primedRef.current = threadId;
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

  return { forget, notice, refresh, rows };
}
