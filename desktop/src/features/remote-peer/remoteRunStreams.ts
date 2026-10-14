import type {
  AgentMessage,
  AssistantRunProjection,
  RunCursor,
  RunCursorEntry,
  RunEvent,
  RunProjector,
  RunReplay,
} from "@future-os/thread-projection";
import type { RemoteRunEvent } from "./remotePeerClient";
import { advanceCursor, createRunProjector, newCursor, nextEvent } from "@future-os/thread-projection";
import { replayEventsToRunEvents } from "./remoteRunReplay";

export type { RemoteRunEvent };

/**
 * The replies a remote conversation is drawing from its runs' own logs.
 *
 * A host publishes its agent's events — the same vocabulary a local run
 * produces: `text_chunk`, `thinking_delta`, `tool_start`, `tool_end`,
 * `agent_end` — and this folds them into the reply the transcript draws, through
 * the *shared* projector (`createRunProjector`). That part is the point: the
 * reply a reader watches arrive on another machine is assembled by the same
 * rules as the one on this machine, down to how a tool row is folded and when a
 * retry is announced.
 *
 * What is *not* shared is where the events come from. Locally they are read out
 * of the run's persisted log, one page per tick, keyed by a run row in this
 * machine's store. Here there are two sources, and the difference between them
 * is what this class is about:
 *
 * - **Pushes** (`apply`) are the live lane: at-most-once, unordered under jitter,
 *   and — because this client declares the trimmed lane (`lean_events_v1`) —
 *   deliberately missing slices. They make a reply *appear as it is written*.
 * - **The run's log** (`restore`) is the read lane: durable, ordered, and the
 *   only source that can cover a run from its beginning. It is what a client
 *   that joined a run already in flight has for the part it never saw.
 *
 * A run this client rebuilt from its log is *authoritative* (`fromReplay`): the
 * transcript's own partial copy of that run is hidden (`stripRunCopies`) and the
 * reply is drawn from here instead. A run it only ever saw pushes for is not:
 * there the transcript's copy is the beginning and these events are the end, so
 * the view folds the two together (`foldLiveReply`). Both are "one row per
 * reply"; they differ in what this client can prove.
 *
 * The cursor (`RunCursor`) is the bookkeeping that makes the two lanes agree: the
 * highest index applied in order, and whether the run's beginning is provably
 * present. It is the shared one, so "what a cursor means" has a single definition
 * across both remote clients.
 */

/** How many runs' projections are kept. One per run, dropped oldest-first. */
const MAX_LIVE_RUNS = 4;

/** One live run of this conversation. */
interface LiveRun {
  runId: string;
  /** Incremental fold over the run's events, from whichever lane fed it. */
  projector: RunProjector;
  /** The projection was rebuilt from the run's log rather than from pushes. */
  fromReplay: boolean;
  /**
   * The run's terminal frame has arrived.
   *
   * The reply is complete, but the host's own record of it may not be readable
   * yet — the agent persists the last model call as the run ends. Until that
   * record is read back (or the conversation is reopened), this projection is
   * the most complete copy of the reply anyone has.
   */
  settled: boolean;
  /**
   * The host's record of this run has been read back, so this copy is no longer
   * drawn.
   *
   * The projection stays for the frames that may still arrive (see
   * `markRecorded`); it simply stops being a source of rows.
   */
  recorded: boolean;
  /**
   * When the run started, for the elapsed-time readout.
   *
   * Taken from the run's own `agent_start` when the log was read (the run's
   * clock), and only otherwise from the moment this window first heard of it —
   * a client that joins a reply in flight receives no `agent_start`, so without
   * the log its timer can only count from its own arrival.
   */
  startedAt: number | null;
}

export interface RemoteLiveReply {
  message: AgentMessage;
  /** The message was rebuilt from the run's log: the transcript's copy is hidden. */
  fromReplay: boolean;
}

/** What `apply` decided about one pushed event. */
export type PushVerdict = "applied" | "duplicate" | "gap" | "overlap" | "untracked";

export class RemoteRunStreams {
  private readonly runs = new Map<string, LiveRun>();
  private readonly cursor: RunCursor = newCursor();

  /**
   * Fold one pushed event in.
   *
   * `omitIndices` says whether a hole between indices is *by design*: this client
   * declares the trimmed lane, whose feed drops whole event families
   * (`thinking_delta`, tool argument deltas), so a jump is expected and must not
   * be read as loss. A host that does not trim sends one event per index, and
   * there a jump is a real gap — which is what the verdict reports, for the
   * caller to reconcile from the run's log.
   */
  apply(event: RemoteRunEvent, options?: { omitIndices?: boolean }): PushVerdict {
    const verdict = nextEvent(
      this.cursor,
      event.runId,
      event.idx,
      event.coalescedCount,
      { omitIndices: options?.omitIndices ?? false },
    );
    if (verdict.kind === "untracked") {
      // A frame that names no run or no index is not part of a reply's log.
      return "untracked";
    }
    if (verdict.kind !== "apply") {
      // A duplicate would double a fragment; a gap or an overlap is a claim
      // about content this client is not holding — the caller reads the run's
      // log rather than appending something it cannot order.
      return verdict.kind === "dup" ? "duplicate" : verdict.kind;
    }
    const run = this.runFor(event.runId);
    run.projector.append(toRunEvent(event));
    return "applied";
  }

  /**
   * Make this run's reply agree with what its log says.
   *
   * Two shapes arrive here, and the difference is what the read could prove:
   *
   * - **A whole run** (a snapshot bootstrap, or a read from index 0): the
   *   projection is replaced. This is the one operation that makes a run
   *   authoritative* (`fromReplay`), and the one that lets the transcript's own
   *   partial copy of the run be hidden.
   * - **A tail** (a read from where this client got to): the events are folded
   *   into the projection that is already there, which is what a gap the pushes
   *   left looks like. The projector skips what it has already applied, so a
   *   re-delivered frame is not drawn twice.
   *
   * Replacing rather than feeding on a whole run is deliberate: the log *is* the
   * run, and a projector that had been folding pushes would otherwise count the
   * same events twice.
   */
  restore(runId: string, replay: RunReplay): void {
    const events = replay.projection?.events ?? replay.events;
    const wholeRun = replay.completePrefix || (replay.projection?.events?.length ?? 0) > 0;
    const runEvents = replayEventsToRunEvents(runId, events);
    const previous = this.runs.get(runId);

    if (wholeRun) {
      const projector = createRunProjector();
      if (runEvents.length > 0)
        projector.ingest(runEvents);
      const projection = projector.snapshot();
      this.runs.delete(runId);
      this.runs.set(runId, {
        runId,
        projector,
        fromReplay: true,
        // A rebuilt run keeps the marks it already had: a replay that lands just
        // as the run ends — or after its record was read — is the same reply, not
        // a new one.
        settled: previous?.settled ?? false,
        recorded: previous?.recorded ?? false,
        // The run's own start when the log carries it; otherwise whatever this
        // window had already been counting from.
        startedAt: projection.startedAtMs ?? previous?.startedAt ?? Date.now(),
      });
    }
    else if (runEvents.length > 0) {
      // A tail fills a hole in a reply this lane is already drawing: it extends
      // the projection rather than becoming authoritative, because the beginning
      // of the run is still only in the record.
      this.runFor(runId).projector.ingest(runEvents);
    }
    this.advance(runId, replay, events.map(event => event.idx).filter((idx): idx is number => idx !== undefined));
    this.evict();
  }

  /**
   * The run's terminal frame arrived: its reply is complete.
   *
   * What happens next depends on what this client's copy *is*:
   *
   * - **Rebuilt from the run's log**: kept. The record of a run trails its
   *   terminal frame (the agent writes the last model call as the run ends), so a
   *   client that dropped its copy here would blink the reply out of the
   *   transcript. It is drawn until the record can be read back (`markRecorded`).
   * - **Built from pushes alone**: dropped, with its projection. That copy is a
   *   fragment* — the reply's beginning is in the record, not in it — so keeping
   *   it past the end draws the words twice, once from each lane.
   */
  settle(runId: string): void {
    const run = this.runs.get(runId);
    if (!run)
      return;
    if (!run.fromReplay) {
      this.runs.delete(runId);
      return;
    }
    run.settled = true;
  }

  /**
   * The host's record of this run is readable: stop drawing this client's copy.
   *
   * The projection is *kept* rather than dropped, because a run can still emit
   * frames after its terminal one (a trailing usage report, a compaction marker)
   * and a dropped run would be recreated by the next of them — as a push-only run
   * whose reply would then be drawn *beside* the record instead of instead of it.
   * A recorded run keeps folding into the same projector and is simply not drawn.
   */
  markRecorded(runId: string): void {
    const run = this.runs.get(runId);
    if (run)
      run.recorded = true;
  }

  /**
   * Everything this conversation is drawing above its record, oldest first.
   *
   * Ordered by when each run was first heard from, which is the order the reader
   * saw them in: a host answers one prompt at a time, so this is normally one
   * reply — and briefly two when a settled reply is still waiting for its record
   * while the next run has started.
   */
  replies(): RemoteLiveReply[] {
    return [...this.runs.values()]
      .filter(run => !run.recorded)
      .flatMap((run): RemoteLiveReply[] => {
        const message = this.bubble(run);
        return message ? [{ message, fromReplay: run.fromReplay }] : [];
      });
  }

  /** The run ids this lane is drawing — the copies to hide from the record. */
  replayedRuns(): ReadonlySet<string> {
    return new Set(
      [...this.runs.values()].filter(run => !run.recorded && run.fromReplay).map(run => run.runId),
    );
  }

  /** The runs whose record is not readable yet — those the caller may retire. */
  knownRuns(): string[] {
    return [...this.runs.values()].filter(run => !run.recorded).map(run => run.runId);
  }

  /** What this lane knows about a run's log position, or undefined. */
  cursorFor(runId: string): RunCursorEntry | undefined {
    return this.cursor.get(runId);
  }

  /**
   * Forget everything: another conversation is being opened.
   *
   * The cursor goes with the projections. It describes how much of *these* runs
   * this lane holds, and the lane no longer holds any of them.
   */
  clear(): void {
    this.runs.clear();
    this.cursor.clear();
  }

  /**
   * The conversation's push lane restarted (a link came back, or a page moved the
   * boundary): what the pushes built is no longer evidence of anything.
   *
   * Projections built from a run's *log* survive with their cursors — they are
   * reads, and a read does not go stale because a socket did. Projections built
   * from pushes are dropped: the next reconcile refills them from the log.
   */
  restart(): void {
    for (const run of [...this.runs.values()]) {
      if (!run.fromReplay)
        this.runs.delete(run.runId);
    }
  }

  private bubble(run: LiveRun): AgentMessage | null {
    const projection = run.projector.snapshot();
    const startedAt = projection.startedAtMs ?? run.startedAt;
    if (run.settled)
      return settledBubble(projection, run.runId, startedAt);
    return streamingBubble(projection, run.runId, startedAt);
  }

  /** Move the run's cursor to what a log read covered. */
  private advance(runId: string, replay: RunReplay, indices: number[]): void {
    const last = indices.reduce((max, idx) => Math.max(max, idx), -1);
    if (last >= 0)
      advanceCursor(this.cursor, runId, last, replay.completePrefix);
    else if (replay.completePrefix)
      advanceCursor(this.cursor, runId, this.cursorFor(runId)?.highWater ?? -1, true);
  }

  private runFor(runId: string): LiveRun {
    const existing = this.runs.get(runId);
    if (existing) {
      // Touch: the map's order is the runs' recency, so the bound below evicts
      // the run nobody has heard from.
      this.runs.delete(runId);
      this.runs.set(runId, existing);
      return existing;
    }
    const run: LiveRun = {
      runId,
      projector: createRunProjector(),
      fromReplay: false,
      settled: false,
      recorded: false,
      // The first frame this window sees of a run it has not seen before is when
      // its reply began, as far as pushes can tell. A replay replaces this with
      // the run's own start.
      startedAt: Date.now(),
    };
    this.runs.set(runId, run);
    this.evict();
    return run;
  }

  /**
   * Bound the projections, oldest first.
   *
   * A run's *cursor* is not dropped with its projector: it is a fact about the
   * host's log — how far this client has read — which stays true after the
   * projection is gone, and it is what lets the next read of that run be a tail
   * rather than a whole-run read.
   */
  private evict(): void {
    while (this.runs.size > MAX_LIVE_RUNS) {
      const oldest = this.runs.keys().next().value;
      /* v8 ignore next -- size > MAX (>= 1) guarantees a first key */
      if (oldest === undefined)
        break;
      this.runs.delete(oldest);
    }
  }
}

function toRunEvent(event: RemoteRunEvent): RunEvent {
  const createdAt = Number.isNaN(Date.parse(event.timestamp)) ? Date.now() : Date.parse(event.timestamp);
  return {
    id: event.eventId,
    runId: event.runId,
    eventType: event.type,
    payload: event.data,
    sequence: event.idx,
    // The host's timestamp is ISO; a body without one still has to sort, and
    // "now" is the only honest fallback (arrival order is the order it sent).
    createdAt,
  };
}

/**
 * Nothing renderable yet.
 *
 * The host's `agent_start` and its first frames arrive before there is a word to
 * show, and an empty bubble with an author line is worse than none.
 */
function empty(projection: AssistantRunProjection): boolean {
  return !projection.content.trim()
    && projection.activityItems.length === 0
    && projection.segments.length === 0
    && !projection.reconnecting;
}

/**
 * The reply being written.
 *
 * The same fields, ids and status this app's own live reply uses, because the
 * transcript is the same component: `MessageBlock` reads `segments` for inline
 * tool activity, `status: "streaming"` for the cursor, and `thinkingActive` for
 * the "thinking…" hint. `stream_<runId>` is the local bubble's id shape, so a
 * local and a remote reply can never collide in one list.
 */
function streamingBubble(
  projection: AssistantRunProjection,
  runId: string,
  startedAt: number | null,
): AgentMessage | null {
  if (empty(projection))
    return null;
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
    runStartedAt: startedAt,
    runId,
  };
}

/**
 * The reply a run finished with, drawn from the run's own log.
 *
 * A settled reply's footer shows `durationMs`, not a live timer, so the run's own
 * duration (from its `agent_end`) is what the reader sees — and a run whose
 * duration the log did not carry falls back to the clock this lane was holding,
 * which is the same number to within a tick.
 *
 * `status: "complete"` is the honest reading of a run whose terminal frame
 * arrived. A *failed* run is not distinguishable here (the failure rides the
 * host's record as `run.status`, and this lane sees only the events), which is
 * exactly why this bubble steps aside as soon as that record is readable.
 */
function settledBubble(
  projection: AssistantRunProjection,
  runId: string,
  startedAt: number | null,
): AgentMessage | null {
  if (empty(projection))
    return null;
  const duration = projection.durationMs
    ?? (startedAt === null ? undefined : Math.max(0, Date.now() - startedAt));
  return {
    id: `stream_${runId}`,
    role: "assistant",
    authorKey: "author.researchCopilot",
    content: projection.content.trim(),
    status: "complete",
    createdAt: new Date().toISOString(),
    activityItems: projection.activityItems,
    segments: projection.segments,
    outputTokens: projection.outputTokens,
    ...(duration === undefined ? {} : { durationMs: duration }),
    runId,
  };
}
