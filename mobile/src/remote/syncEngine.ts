/**
 * Per-session serial sync engine — the single writer for a session's timeline.
 *
 * The timeline cache + per-run cursors are logically ONE atomic unit (the
 * cursor's meaning is "the cache provably contains ≤ high-water"). The audit's
 * root cause A was that seven paths mutated them separately and raced. This
 * module collapses every mutation into a small set of *sync instructions*, fed
 * through one serial lane per session. Cursor advance and timeline commit
 * happen inside the same instruction, so they cannot interleave.
 *
 * The live event stream is a *tickle*, not a data source (root cause B):
 * at-most-once pushes can drop any suffix, including agent_end. The durable
 * replay via get_events_since is the truth, and **reconcile is the only
 * backfill path**. Every integrity trigger — a gap, an incomplete prefix, a
 * truncated run, a snapshot streaming-flip, an open/switch, a reconnect —
 * converges to the same reconcile instruction.
 *
 * Reconcile has two shapes:
 *   - FULL  (since = -1): the run's prefix is not provably complete, or the
 *     run was replaced (run_snapshot). Rebuilds the timeline from durable
 *     history, strips the run's partial items, and replays the whole run. This
 *     is the anti-H3/H5 path: a mid-run join or a reconnect that lost the
 *     prefix converges here.
 *   - TAIL  (since = highWater): the prefix is complete. Folds the dropped
 *     tail (gap fill, reconnect while streaming, snapshot-flip) onto the live
 *     snapshot without disturbing the prefix.
 *
 * Invariant (module doc + tests): at any instant a session's timeline equals
 * settled history + the active run's replay over [0, highWater] when its prefix
 * is complete. Any input that contradicts this converges to a reconcile.
 *
 * The engine is framework-agnostic: the provider feeds instructions via
 * `event`/`reconcile`/`mutate` and receives committed timelines via
 * `subscribe`. Enqueue calls are synchronous so the NATS callback keeps its
 * tight path; network and history work ride the lane's own promise chain.
 */

import {
  applyStreamEvent,
  emptyTimeline,
  normalizeReplayEvents,
  stripRunItems,
  timelineFromProjection,
  upsertTruncationNotice,
  type ReplayEventWire,
  type TimelineState,
} from "./eventReducer";
import {
  advanceCursor,
  cursorHighWater,
  isPrefixComplete,
  newCursor,
  nextEvent,
  type RunCursor,
} from "./runCursor";
import type { StreamEvent, RemoteSessionState } from "./types";

/** Paginated replay pages, merged by the caller (P0 H2 pagination). */
export interface ReplayResult {
  events: ReplayEventWire[];
  projection?: { run_id?: string; cursor?: number; events?: ReplayEventWire[] } | null;
  truncated?: boolean;
}

/**
 * Everything a sync instruction may need to touch the desktop. The provider
 * wires these to its live client; the lane never reaches for a module global,
 * so a session's queue stays correct across client generations.
 */
export interface SyncDeps {
  requestGetState(sessionId: string): Promise<RemoteSessionState>;
  requestHistory(sessionId: string): Promise<TimelineState>;
  fetchReplay(sessionId: string, runId: string, sinceIdx: number): Promise<ReplayResult>;
}

export type ReconcileReason =
  "gap" | "prefix" | "truncated" | "snapshot-flip" | "open" | "reconnect" | "resend";

/** A committed timeline + its cursor, delivered to subscribers. */
export interface Commit {
  sessionId: string;
  timeline: TimelineState;
  cursor: RunCursor;
}

/** A synchronous timeline mutation (approval decision, optimistic bubble…). */
type Mutator = (timeline: TimelineState) => TimelineState;

type Op = { kind: "event"; event: StreamEvent } | { kind: "mutate"; apply: Mutator };

interface ReconcileRequest {
  reason: ReconcileReason;
  runId?: string;
}

interface SessionLane {
  sessionId: string;
  chain: Promise<void>;
  cursor: RunCursor;
  timeline: TimelineState | null;
  ops: Op[];
  replayQueue: ReconcileRequest[];
  established: boolean;
}

const MAX_REPLAY_QUEUE = 6;

export class SyncEngine {
  private lanes = new Map<string, SessionLane>();
  private subscribers = new Set<(commit: Commit) => void>();
  private deps: SyncDeps;

  constructor(deps: SyncDeps) {
    this.deps = deps;
  }

  subscribe(fn: (commit: Commit) => void): () => void {
    this.subscribers.add(fn);
    return () => this.subscribers.delete(fn);
  }

  /** Enqueue a live event for the session. Never throws. */
  event(sessionId: string, event: StreamEvent): void {
    const lane = this.laneFor(sessionId);
    // First contact for a real session: establish the timeline from durable
    // history + a full replay so a mid-run join gets its prefix (H3), not just
    // the live tail.
    if (!lane.established && lane.sessionId !== "") {
      this.enqueueReplay(lane, { reason: "open" });
    }
    lane.ops.push({ kind: "event", event });
    this.loop(lane);
  }

  /** Enqueue a reconcile. Repeats of the same reason+run are folded. */
  reconcile(sessionId: string, reason: ReconcileReason, runId?: string): void {
    this.enqueueReplay(this.laneFor(sessionId), { reason, runId });
  }

  /** Reconcile every real lane (reconnect recovery). Lanes that never
   * established (their first history load failed while the backend was down)
   * are included — this is exactly the case that must self-heal on recovery;
   * the draft lane ("") has no desktop state and is skipped by runReconcile. */
  reconcileAll(reason: ReconcileReason): void {
    for (const lane of this.lanes.values()) {
      if (lane.sessionId !== "") this.enqueueReplay(lane, { reason });
    }
  }

  /** Apply a synchronous timeline mutation inside the lane. Never throws. */
  mutate(sessionId: string, apply: Mutator): void {
    const lane = this.laneFor(sessionId);
    lane.ops.push({ kind: "mutate", apply });
    this.loop(lane);
  }

  /** Current committed timeline, or null before the lane establishes. */
  timelineFor(sessionId: string): TimelineState | null {
    return this.lanes.get(sessionId)?.timeline ?? null;
  }

  cursorFor(sessionId: string): RunCursor {
    return this.lanes.get(sessionId)?.cursor ?? newCursor();
  }

  /** The committed snapshot's streaming flag — the send guard's source. */
  streamingFor(sessionId: string): boolean {
    return this.lanes.get(sessionId)?.timeline?.streaming ?? false;
  }

  /** Drop all lanes (unpair / credentials cleared). */
  clear(): void {
    this.lanes.clear();
  }

  private laneFor(sessionId: string): SessionLane {
    let lane = this.lanes.get(sessionId);
    if (!lane) {
      lane = {
        sessionId,
        chain: Promise.resolve(),
        cursor: newCursor(),
        timeline: null,
        ops: [],
        replayQueue: [],
        established: false,
      };
      this.lanes.set(sessionId, lane);
    }
    return lane;
  }

  private enqueueReplay(lane: SessionLane, request: ReconcileRequest): void {
    const duplicate = lane.replayQueue.some(
      existing => existing.reason === request.reason && existing.runId === request.runId,
    );
    if (duplicate) return;
    if (lane.replayQueue.length >= MAX_REPLAY_QUEUE) return;
    lane.replayQueue.push(request);
    this.loop(lane);
  }

  /** Serial loop — reconciles first, then the queued ops, atomically. */
  private loop(lane: SessionLane): void {
    lane.chain = lane.chain
      .then(() => this.step(lane))
      .catch(() => {
        // A failed step must never kill the lane chain.
      });
  }

  private async step(lane: SessionLane): Promise<void> {
    let justReconciledRun: string | null = null;
    const request = lane.replayQueue.shift();
    if (request) {
      justReconciledRun = request.runId ?? null;
      await this.runReconcile(lane, request);
    }
    this.applyOps(lane, justReconciledRun);
  }

  private async runReconcile(lane: SessionLane, request: ReconcileRequest): Promise<void> {
    if (lane.sessionId === "") return; // the draft lane has no desktop state.
    try {
      const state = await this.deps.requestGetState(lane.sessionId);
      const activeRunId = state.activeRun?.runId ?? "";
      // The reconcile target is the requested run (a snapshot-flip run that
      // just settled is no longer active) else the active run.
      const targetRunId = request.runId || activeRunId;

      const full = this.isFullReplay(lane, targetRunId, request);
      if (full) {
        await this.fullReconcile(lane, targetRunId);
      } else {
        await this.tailReconcile(lane, targetRunId);
      }
      lane.established = true;
    } catch {
      // Network/desktop failure mid-reconcile — keep the last committed
      // snapshot. If the lane never established, the next event re-triggers.
    }
  }

  /**
   * Rebuild the timeline from durable history + a whole-run replay. Used when
   * the run's prefix is not provably complete (mid-run join), when the run was
   * replaced (run_snapshot), or on a forced full reason (prefix / resend).
   */
  private async fullReconcile(lane: SessionLane, targetRunId: string): Promise<void> {
    const history = await this.deps.requestHistory(lane.sessionId);
    let base = mergeLiveInto(history, lane.timeline);
    if (targetRunId) {
      base = stripRunItems(base, targetRunId);
      base = await this.replayInto(lane, base, targetRunId, -1);
    }
    lane.timeline = base;
    this.commit(lane);
  }

  /**
   * Fold a run's missing tail onto the live snapshot. The prefix is already
   * complete and stays untouched — this is gap fill, a reconnect while
   * streaming, and the snapshot-flip check for a run that just settled.
   */
  private async tailReconcile(lane: SessionLane, runId: string): Promise<void> {
    if (!runId || !lane.timeline) return;
    const since = cursorHighWater(lane.cursor, runId);
    const base = await this.replayInto(lane, lane.timeline, runId, since);
    lane.timeline = base;
    this.commit(lane);
  }

  /**
   * Fetch a run's events from `since` and fold them into `base`. A FULL fetch
   * (since = -1) rebuilds the run's items from the replay (strip first, then
   * re-apply); a TAIL fetch appends without touching existing items. Both
   * advance the cursor; a full fetch marks the prefix complete.
   */
  private async replayInto(
    lane: SessionLane,
    base: TimelineState,
    runId: string,
    since: number,
  ): Promise<TimelineState> {
    const result = await this.deps.fetchReplay(lane.sessionId, runId, since);
    if (result.projection?.events?.length) {
      let events = normalizeReplayEvents(result.projection.events);
      // A folded projection's run id rides on the envelope, not necessarily on
      // each event. Stamp the replayed run's id onto the events so the
      // projection's agent_start/agent_end match the same `assistant:{runId}`
      // item as the live stream — without this, agent_start builds an
      // `assistant:` ghost that no later agent_end can clear, leaving a
      // permanent run indicator.
      events = events.map(ev => (ev.runId ? ev : { ...ev, runId }));
      const cursorIdx =
        result.projection.cursor ?? events.reduce((max, ev) => Math.max(max, ev.idx ?? -1), -1);
      advanceCursor(lane.cursor, runId, cursorIdx, true);
      const stripped = stripRunItems(base, runId);
      const projected = timelineFromProjection(events);
      const settled = events.some(ev => ev.type === "agent_end");
      return {
        ...stripped,
        items: [...stripped.items, ...projected.items],
        streaming: !settled,
      };
    }
    const events = normalizeReplayEvents(result.events);
    if (events.length === 0) return base;
    let next = base;
    if (since === -1) {
      // Whole-run replay — supersede any partial items of the run.
      next = stripRunItems(base, runId);
    }
    for (const ev of events) {
      advanceCursor(lane.cursor, ev.runId ?? runId, ev.idx ?? -1, since === -1);
      next = applyStreamEvent(next, ev);
    }
    const settled = events.some(ev => ev.type === "agent_end");
    if (result.truncated) next = { ...next, items: upsertTruncationNotice(next.items, runId) };
    // A settled replay overrides the live streaming flag; an empty replay
    // leaves it alone (the run may have ended between the fetch and now).
    return { ...next, streaming: settled ? false : next.streaming };
  }

  /**
   * Apply the queued synchronous ops to the snapshot in order. `justReconciledRun`
   * is the run the immediately-preceding reconcile covered — a gap event for it
   * must not be re-enqueued: the reconcile just fetched from its cursor, so a
   * repeat would loop forever when the replay is genuinely empty (the run's
   * durable journal ended where the cursor sits).
   */
  private applyOps(lane: SessionLane, justReconciledRun: string | null): void {
    if (lane.ops.length === 0) return;
    let timeline = lane.timeline ?? emptyTimeline();
    const ops = lane.ops;
    lane.ops = [];
    const beforeStreaming = timeline.streaming;
    let flipRunId: string | undefined;
    let changed = false;

    for (let index = 0; index < ops.length; index += 1) {
      const op = ops[index];
      if (!op) continue;
      if (op.kind === "mutate") {
        const next = op.apply(timeline);
        if (next !== timeline) {
          timeline = next;
          changed = true;
        }
        continue;
      }
      const event = op.event;
      const wasFirst = event.runId != null && !lane.cursor.has(event.runId);
      const verdict = nextEvent(lane.cursor, event.runId, event.idx);
      if (verdict.kind === "dup") continue;
      if (verdict.kind === "gap") {
        if (event.runId && justReconciledRun !== event.runId) {
          // Defer the gap event and everything after it until the gap reconcile
          // fills the hole, then continue applying in order.
          lane.ops.unshift(...ops.slice(index));
          this.enqueueReplay(lane, { reason: "gap", runId: event.runId ?? "" });
        }
        break;
      }
      if (verdict.kind === "apply") {
        advanceCursor(lane.cursor, event.runId!, verdict.idx);
        timeline = applyStreamEvent(timeline, event);
        changed = true;
        if (event.type === "agent_end") flipRunId = event.runId ?? flipRunId;
        // A run whose first seen event has idx > 0 (mid-run join or an
        // out-of-order first delivery) has an unknown prefix (H3) — reconcile
        // it from -1 so the missing prefix is recovered.
        if (wasFirst && (verdict.idx ?? 0) > 0) {
          this.enqueueReplay(lane, { reason: "prefix", runId: event.runId ?? "" });
        }
      } else {
        // untracked — no cursor, apply as-is (dedup lives in the reducer).
        timeline = applyStreamEvent(timeline, event);
        changed = true;
        if (event.type === "agent_end") flipRunId = event.runId ?? flipRunId;
      }
    }

    if (changed) {
      lane.timeline = timeline;
      this.commit(lane);
    }
    // A run settling in this batch may have lost its tail (M11) — reconcile
    // the settled run so the durable journal supersedes the partial replay.
    if (beforeStreaming && !timeline.streaming && flipRunId) {
      this.enqueueReplay(lane, { reason: "snapshot-flip", runId: flipRunId });
    }
  }

  private isFullReplay(lane: SessionLane, runId: string, request: ReconcileRequest): boolean {
    if (request.reason === "prefix" || request.reason === "resend") return true;
    // A lane with no committed timeline has no live baseline to top up — the
    // only way to build it is from durable history (idle sessions have no
    // active run for a tail reconcile to target).
    if (lane.timeline === null) return true;
    return !isPrefixComplete(lane.cursor, runId);
  }

  private commit(lane: SessionLane): void {
    if (!lane.timeline) return;
    for (const fn of this.subscribers) {
      fn({ sessionId: lane.sessionId, timeline: lane.timeline, cursor: lane.cursor });
    }
  }
}

/**
 * Build the full-reconcile base: durable history is the settled truth; fold in
 * the live cache's items that history doesn't carry and that aren't part of
 * the active run (optimistic bubbles, notices, approval cards) so they survive
 * the rebuild. Live user messages that duplicate a history prompt are dropped.
 */
function mergeLiveInto(history: TimelineState, live: TimelineState | null): TimelineState {
  if (!live) return { ...history, streaming: history.streaming };
  const historyIds = new Set(history.items.map(item => item.id));
  const historyUserTexts = new Set(
    history.items
      .filter(item => item.kind === "message" && item.role === "user")
      .map(item => (item.kind === "message" ? item.text : "")),
  );
  // Keep everything durable history doesn't carry: optimistic bubbles, notices,
  // approval cards, and live user mirrors of prompts not yet durable. The
  // active run's *assistant* items are dropped by fullReconcile's stripRunItems
  // (the replay rebuilds them); user bubbles always survive.
  const folded = live.items.filter(item => {
    if (historyIds.has(item.id)) return false;
    if (item.kind === "message" && item.role === "user" && historyUserTexts.has(item.text)) {
      return false;
    }
    return true;
  });
  return {
    ...history,
    items: [...history.items, ...folded],
    streaming: live.streaming,
  };
}
