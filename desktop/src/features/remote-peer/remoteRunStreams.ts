import type { AgentMessage, AssistantRunProjection, RunEvent, RunProjector } from "@future-os/thread-projection";
import { createRunProjector } from "@future-os/thread-projection";

/**
 * A remote run's reply, while it is still being written.
 *
 * The host publishes its agent's events — the same vocabulary a local run
 * produces: `text_chunk`, `thinking_delta`, `tool_start`, `tool_end`,
 * `agent_end` — and this folds them into the reply bubble the transcript draws.
 * The projection itself is the shared one (`createRunProjector`), which is the
 * point: the reply a reader watches arrive on another machine is assembled by the
 * same rules as the one on this machine, down to how a tool row is folded and
 * when a retry is announced.
 *
 * What is *not* shared is where the events come from. Locally they are read out
 * of the run's persisted event log, one page per tick, keyed by a run row in this
 * machine's store; here they arrive as pushes, and there is no local run to read
 * — which is why this is a class over pushed events rather than the store-backed
 * cache the local view uses.
 */

/** One pushed event, as the host's own body spells it. */
export interface RemoteRunEvent {
  /** The event's JSON payload, exactly as the host sent it (a JSON string). */
  data: string;
  /** Run-scoped event index; the projector orders by this. */
  idx: number;
  /** The host's event id, which is this event's identity. */
  eventId: string;
  /** The host's run id — what the reply bubble is addressed by. */
  runId: string;
  /** ISO timestamp from the host. */
  timestamp: string;
  /** The host's event type (`text_chunk`, `tool_end`, …). */
  type: string;
}

/**
 * How many runs' projectors are kept.
 *
 * One per run, because a run's projection is incremental — the deltas that built
 * it are gone once applied. A conversation answering one prompt at a time has one
 * live run, so this is only ever about a session that has been busy: entries are
 * dropped oldest-first and a dropped run simply rebuilds from what arrives next.
 */
const MAX_LIVE_RUNS = 4;

export class RemoteRunStreams {
  private readonly projectors = new Map<string, RunProjector>();
  /** When each live run started, for the reply's elapsed-time readout. */
  private readonly startedAt = new Map<string, number>();

  /**
   * Fold one pushed event in, and return the reply to draw.
   *
   * Returns `null` when the run has nothing renderable yet — the host's
   * `agent_start` and its first frames arrive before there is a word to show, and
   * an empty bubble with an author line is worse than none.
   */
  apply(event: RemoteRunEvent): AgentMessage | null {
    const projector = this.projectorFor(event.runId);
    projector.append(toRunEvent(event));
    return bubbleFor(projector.snapshot(), event.runId, this.startedAt.get(event.runId) ?? null);
  }

  /**
   * The run is over: forget its projection.
   *
   * The settled reply is a *record* — the host's journal has it, and the
   * transcript re-reads it — so keeping the projection would only leave a second
   * copy that can disagree with the one being drawn.
   */
  settle(runId: string): void {
    this.projectors.delete(runId);
    this.startedAt.delete(runId);
  }

  /** Another conversation is being opened: nothing here belongs to it. */
  clear(): void {
    this.projectors.clear();
    this.startedAt.clear();
  }

  private projectorFor(runId: string): RunProjector {
    const existing = this.projectors.get(runId);
    if (existing) {
      // Touch: the map's order is the runs' recency, so the bound below evicts
      // the run nobody has heard from.
      this.projectors.delete(runId);
      this.projectors.set(runId, existing);
      return existing;
    }
    const projector = createRunProjector();
    this.projectors.set(runId, projector);
    this.startedAt.set(runId, Date.now());
    while (this.projectors.size > MAX_LIVE_RUNS) {
      const oldest = this.projectors.keys().next().value;
      /* v8 ignore next -- size > MAX (>= 1) guarantees a first key */
      if (oldest === undefined)
        break;
      this.settle(oldest);
    }
    return projector;
  }
}

function toRunEvent(event: RemoteRunEvent): RunEvent {
  const createdAt = Date.parse(event.timestamp);
  return {
    id: event.eventId,
    runId: event.runId,
    eventType: event.type,
    payload: event.data,
    sequence: event.idx,
    // The host's timestamp is ISO; a body without one still has to sort, and
    // "now" is the only honest fallback (arrival order is the order it sent).
    createdAt: Number.isNaN(createdAt) ? Date.now() : createdAt,
  };
}

/**
 * The reply bubble for a projection.
 *
 * The same fields, ids and status this app's own live reply uses, because the
 * transcript is the same component: `MessageBlock` reads `segments` for inline
 * tool activity, `status: "streaming"` for the cursor, and `thinkingActive` for
 * the "thinking…" hint. `stream_<runId>` is the local bubble's id shape, so a
 * local and a remote reply can never collide in one list.
 */
function bubbleFor(
  projection: AssistantRunProjection,
  runId: string,
  runStartedAt: number | null,
): AgentMessage | null {
  if (
    !projection.content.trim()
    && projection.activityItems.length === 0
    && projection.segments.length === 0
    && !projection.reconnecting
  ) {
    return null;
  }
  return {
    id: `stream_${runId}`,
    role: "assistant",
    authorKey: "author.researchCopilot",
    content: projection.content.trim(),
    status: "streaming",
    createdAt: new Date().toISOString(),
    activityItems: projection.activityItems,
    segments: projection.segments,
    thinkingActive: projection.thinkingActive,
    reconnecting: projection.reconnecting,
    outputTokens: projection.outputTokens,
    runStartedAt,
    runId,
  };
}
