import { emptyTimeline, timelineFromEntries } from "../timeline";
import { SyncEngine, type ReplayResult } from "../syncEngine";
import type { HistoryEntry, StreamEvent } from "../types";

/** Deterministic run id generator. */
let seq = 0;
function resetSeq(): void {
  seq = 0;
}
function nextRunId(): string {
  seq += 1;
  return `run-${seq}`;
}

/**
 * One event, as the *normalized* read returns it: the run is named, not optional.
 *
 * The helpers below build the fixtures for `fetchReplay`, which stands in for the
 * shared read's result (`RunReplay`) rather than for a raw wire page — so a fixture
 * that says which run each event belongs to is saying what the read does.
 */
type ReplayedEvent = StreamEvent & { runId: string };
function evt(type: string, run: string, idx: number, data = "{}"): ReplayedEvent {
  return { type, runId: run, idx, data };
}
function agentStart(run: string, idx = 0): ReplayedEvent {
  return evt("agent_start", run, idx);
}
function textChunk(run: string, idx: number, text: string): ReplayedEvent {
  return evt("text_chunk", run, idx, JSON.stringify({ text }));
}
function agentEnd(run: string, idx: number): ReplayedEvent {
  return evt("agent_end", run, idx);
}

/** A run's durable journal — the truth the replay reads. */
class Journal {
  events: StreamEvent[] = [];
  add(event: StreamEvent): void {
    this.events.push(event);
  }
  since(run: string, from: number): StreamEvent[] {
    return this.events.filter((e) =>
      e.runId === run && e.idx != null && from === -1 ? true : (e.idx ?? -1) > from,
    );
  }
  /** The newest index this run's journal holds — the number a read's watermark names. */
  highest(run: string): number | undefined {
    return this.events.filter((e) => e.runId === run && e.idx != null).at(-1)?.idx;
  }
}

class Harness {
  journal = new Journal();
  activeRunId = "";
  isCompacting = false;
  history: ReturnType<typeof emptyTimeline> = emptyTimeline();
  /** Optional folded projection; when set, fetchReplay returns it instead. */
  projection: StreamEvent[] | null = null;
  /** A projection whose cursor is missing or nonsensical, as a legacy host sends. */
  projectionCursor: number | undefined = undefined;
  replayFailures = 0;
  /** Mark every replay reply as byte-truncated by the desktop. */
  truncateReplay = false;
  /** Drives `isSessionVisible`; hidden lanes keep their cache but defer work. */
  visible = true;
  /** Fires between a failed reconcile and its scheduled retry, exactly like the
   * provider's failure hook (a good place to hide the session or drop the lane). */
  onFailure: (() => void) | null = null;
  failures: unknown[] = [];
  /** When set, `requestGetState` waits for it before answering. */
  stateBlocked: Promise<void> | null = null;
  /** When set, the next `fetchReplay` holds its already-computed reply until
   * this resolves (one shot). Lets a test keep a replay in flight while
   * something else happens to the lane — the race a resend creates. */
  replayBlocked: Promise<void> | null = null;
  /** Every get_state request, so a test can assert a reconcile never asked. */
  stateCalls = 0;
  /**
   * Whether the connected Desktop agreed to a feed that omits source indices
   * (the client declared `lean_events_v1`). Such a hole is by design, so the
   * lane must apply across it rather than reconcile a range that will never
   * arrive.
   */
  feedOmitsIndices = false;
  /** Every replay the engine asked for, so a test can assert that a settled
   * run was not re-read from the journal. */
  replayCalls: { run: string; since: number }[] = [];
  timeline: Record<string, ReturnType<typeof emptyTimeline>> = {};
  engine: SyncEngine;

  constructor(activeRunId = "") {
    this.activeRunId = activeRunId;
    this.engine = new SyncEngine({
      isSessionVisible: (sessionId) => sessionId === "" || this.visible,
      requestGetState: async () => {
        this.stateCalls += 1;
        if (this.stateBlocked) await this.stateBlocked;
        const state: { activeRun?: { runId: string }; isCompacting: boolean } = { isCompacting: this.isCompacting };
        if (this.activeRunId) state.activeRun = { runId: this.activeRunId };
        return state;
      },
      requestHistory: async () => this.history,
      onFailure: () => {
        this.failures.push(this.failures.length);
        this.onFailure?.();
      },
      feedOmitsIndices: () => this.feedOmitsIndices,
      fetchReplay: async (_sessionId, run, since) => {
        this.replayCalls.push({ run, since });
        if (this.replayFailures > 0) {
          this.replayFailures -= 1;
          throw new Error("temporary replay failure");
        }
        // The *normalized* result the shared read returns: the wire's spellings
        // (snake_case `run_id`, a payload that is not always a string) are
        // resolved before the lane sees them, and what is left here is the shape
        // the projection consumes.
        const events = this.journal
          .since(run, since)
          .map((e) => ({ type: e.type, data: e.data, runId: e.runId ?? run, idx: e.idx }));
        const watermark = this.journal.highest(run);
        // The reply is fixed from here on; hold it in flight if asked.
        if (this.replayBlocked) {
          const blocked = this.replayBlocked;
          this.replayBlocked = null;
          await blocked;
        }
        if (this.projection) {
          // A folded projection names no run per event (`runId: ""` is what the
          // shared read leaves for an envelope that did not) — exactly the shape
          // that reproduced the ghost item.
          const wire = this.projection.map((e) => ({ type: e.type, data: e.data, runId: "", idx: e.idx }));
          const result: ReplayResult = {
            events: [],
            projection: {
              runId: run,
              cursor: this.projectionCursor ?? wire.length - 1,
              events: wire,
            },
            completePrefix: true,
            ...(this.truncateReplay ? { truncated: true } : {}),
          };
          return result;
        }
        const result: ReplayResult = {
          events,
          // What the shared read concludes: a whole-run read whose events are
          // contiguous from zero through its watermark holds the run's beginning;
          // a tail read never claims it.
          completePrefix: since === -1 && !this.truncateReplay
            && events.every((event, index) => event.idx === index)
            && (watermark === undefined || watermark === events.length - 1),
          ...(this.truncateReplay ? { truncated: true } : {}),
        };
        return result;
      },
    });
    this.engine.subscribe((commit) => {
      this.timeline[commit.sessionId] = commit.timeline;
    });
  }

  /** Set which run get_state reports as active (the running run). */
  active(run: string): void {
    this.activeRunId = run;
  }

  /** Wait for the lane to drain. */
  async settle(): Promise<void> {
    await new Promise((resolve) => setTimeout(resolve, 100));
  }

  /** Timeline committed for a session (thrown if the lane never established). */
  timelineOf(sessionId: string): ReturnType<typeof emptyTimeline> {
    const timeline = this.timeline[sessionId];
    if (!timeline) throw new Error(`no committed timeline for ${sessionId}`);
    return timeline;
  }

  textOf(sessionId: string): string {
    return (this.timeline[sessionId]?.items ?? [])
      .filter((item) => item.kind === "message")
      .map((item) => (item.kind === "message" ? item.text : ""))
      .join("");
  }
}

describe("SyncEngine", () => {
  test("live queued standalone compaction reaches the UI through the production batch lane", async () => {
    const h = new Harness("r");
    h.journal.add(agentStart("r"));
    h.journal.add(textChunk("r", 1, "reply"));
    try {
      await h.engine.open("s");
      await h.settle();
      h.active("");
      const end = agentEnd("r", 2);
      h.journal.add(end);
      h.engine.event("s", end);
      await h.settle();
      const started = evt("compaction_started", "r", 3, JSON.stringify({ operation_id: "cmp", phase: "standalone", trigger: "manual" }));
      h.journal.add(started);
      h.engine.event("s", started);
      await h.settle();
      expect(h.timelineOf("s").items.at(-1)).toMatchObject({ streaming: false, segments: [{ status: "running" }] });
      expect(h.timelineOf("s").streaming).toBe(false);
      const committed = evt("compaction_committed", "r", 4, JSON.stringify({ operation_id: "cmp", checkpoint_id: "cp", phase: "standalone", trigger: "manual" }));
      h.journal.add(committed);
      h.engine.event("s", committed);
      await h.settle();
      expect(h.timelineOf("s").items.at(-1)).toMatchObject({ id: "m_cp", streaming: false, segments: [{ status: "completed" }] });
      expect(h.timelineOf("s").streaming).toBe(false);
      expect(h.timelineOf("s").compacting).toBe(false);
      // The compaction frames are session fan-out stamped with the settled
      // run's identity: they must apply without ever re-reading that run.
      expect(h.replayCalls.filter(call => call.run === "r")).toHaveLength(1);
    } finally { h.engine.clear(); }
  });

  test("a resend that lands while the replay is in flight discards that pass instead of committing it", async () => {
    // `resend` is a fresh-prefix reason, so enqueueing it bumps the lane's
    // baseline version synchronously even though the lane is already
    // reconciling. The pass in flight captured that version before its awaits,
    // so it has to notice the bump and abandon its replay rather than commit a
    // snapshot that predates the resend. Production reaches this through the
    // compaction poll, which reconciles with exactly this reason.
    const textOf = (timeline: ReturnType<typeof emptyTimeline>): string =>
      timeline.items
        .filter(item => item.kind === "message")
        .map(item => (item as { text?: string }).text ?? "")
        .join("|");
    const h = new Harness("r");
    h.journal.add(agentStart("r"));
    h.journal.add(textChunk("r", 1, "first"));
    let release!: () => void;
    h.replayBlocked = new Promise<void>((resolve) => { release = resolve; });
    const committedText: string[] = [];
    h.engine.subscribe((commit) => { committedText.push(textOf(commit.timeline)); });
    try {
      const opened = h.engine.open("s");
      await h.settle();
      // The replay for run "r" is parked, its reply already computed from the
      // journal as it stood *before* the resend.
      expect(h.replayCalls).toEqual([{ run: "r", since: -1 }]);
      const beforeResend = committedText.length;
      h.journal.add(textChunk("r", 2, "second"));
      h.engine.reconcile("s", "resend");
      release();
      await opened.catch(() => undefined);
      await h.settle();
      // Nothing may be committed from the abandoned reply: a snapshot the
      // resend already invalidated must never reach the UI.
      expect(committedText.slice(beforeResend)).toEqual([]);
      // The abandoned pass arms a 500ms retry backoff, after which the resend
      // runs (settle() only waits 100ms, so wait past the backoff explicitly).
      await new Promise((resolve) => setTimeout(resolve, 700));
      expect(h.replayCalls.length).toBe(2);
      const after = committedText.slice(beforeResend);
      expect(after.length).toBeGreaterThan(0);
      // Every snapshot that did reach the UI reflects the resend.
      for (const text of after) expect(text).toContain("second");
      expect(textOf(h.timelineOf("s"))).toBe("firstsecond");
    } finally { h.engine.clear(); }
  });

  test("a compaction frame with no cursor for its stamped run is not a gap", async () => {
    // The lane never witnessed run "r" (fresh open, or its cursor was evicted
    // by newer runs). The broadcaster still stamps between-runs compaction
    // with it; dropping those frames would lose the compaction's only signal.
    const h = new Harness("r");
    h.journal.add(agentStart("r"));
    h.journal.add(textChunk("r", 1, "reply"));
    try {
      await h.engine.open("s");
      await h.settle();
      const callsBefore = h.replayCalls.length;
      h.active("");
      h.engine.event("s", evt("compaction_started", "r", 3, JSON.stringify({ operation_id: "cmp", phase: "standalone", trigger: "manual" })));
      await h.settle();
      expect(h.timelineOf("s").items.at(-1)).toMatchObject({ segments: [{ status: "running" }] });
      expect(h.timelineOf("s").compacting).toBe(true);
      h.engine.event("s", evt("compaction_committed", "r", 4, JSON.stringify({ operation_id: "cmp", checkpoint_id: "cp", phase: "standalone", trigger: "manual" })));
      await h.settle();
      expect(h.timelineOf("s").items.at(-1)).toMatchObject({ id: "m_cp", segments: [{ status: "completed" }] });
      expect(h.timelineOf("s").compacting).toBe(false);
      // No gap replay for a run the lane never tracked — nothing was lost.
      expect(h.replayCalls.length).toBe(callsBefore);
    } finally { h.engine.clear(); }
  });

  test("a history refresh clears a settled compaction start-alias left by a lost terminal", async () => {
    // The terminal frame never arrived, but a late queued start folded into
    // the durable checkpoint (see the projection test): the settled divider
    // still wears its `compaction:<op>` id. The next history merge must drop
    // that alias — the durable `m_<checkpoint>` row is the same marker.
    // History is paged by user exchange, so the window needs a prompt to
    // cover both the reply and the checkpoint.
    const h = new Harness("r");
    h.journal.add(agentStart("r"));
    h.journal.add(textChunk("r", 1, "reply"));
    try {
      await h.engine.open("s");
      await h.settle();
      h.active("");
      h.history = timelineFromEntries([
        { id: "u", kind: "user", role: "user", createdAtMs: 0, runId: "r", blocks: [{ kind: "text", text: "question" }] },
        { id: "a", kind: "assistant", role: "assistant", createdAtMs: 1, runId: "r", blocks: [{ kind: "text", text: "reply" }] },
        { id: "cp-entry", kind: "compaction", role: "system", createdAtMs: 2, blocks: [],
          checkpoint: { checkpointId: "cp", trigger: "manual", phase: "standalone" } },
      ]);
      h.engine.event("s", evt("compaction_started", "r", 3, JSON.stringify({ operation_id: "cmp", phase: "standalone", trigger: "manual" })));
      await h.settle();
      // The start beat the history preview to the commit: a running
      // placeholder sits below where the durable divider will land, and its
      // terminal frame is never coming.
      expect(h.timelineOf("s").items.at(-1)).toMatchObject({ id: "compaction:cmp", segments: [{ status: "running" }] });
      h.engine.reconcile("s", "resend");
      await h.settle();
      const dividers = h.timelineOf("s").items.filter(item =>
        item.kind === "message" && item.segments?.some(segment => segment.kind === "compaction"));
      expect(dividers).toEqual([expect.objectContaining({ id: "m_cp" })]);
    } finally { h.engine.clear(); }
  });

  test.each(["compaction_committed", "compaction_failed", "compaction_unchanged"])("restores compaction on open and releases it on %s", async terminal => {
    const h = new Harness();
    h.isCompacting = true;
    try {
      await h.engine.open("s");
      await h.settle();
      expect(h.timelineOf("s").compacting).toBe(true);
      expect(h.timelineOf("s").streaming).toBe(false);
      h.isCompacting = false;
      h.engine.event("s", { type: terminal, data: JSON.stringify({ operation_id: "cmp", trigger: "manual" }) });
      await h.settle();
      expect(h.timelineOf("s").compacting).toBe(false);
      h.engine.event("s", { type: "compaction_started", data: JSON.stringify({ operation_id: "cmp2", trigger: "manual" }) });
      await h.settle();
      expect(h.timelineOf("s").compacting).toBe(true);
      // A missed terminal must not leave sending disabled after reconnect.
      h.engine.reconcile("s", "reconnect");
      await h.settle();
      expect(h.timelineOf("s").compacting).toBe(false);
    } finally { h.engine.clear(); }
  });

  test.each(["open", "reconnect"] as const)("%s clears a run that finished while hidden and restores its footer", async reason => {
    const h = new Harness("r");
    h.journal.add(agentStart("r"));
    h.journal.add(textChunk("r", 1, "partial"));
    try {
      await h.engine.open("s");
      await h.settle();
      expect(h.engine.streamingFor("s")).toBe(true);
      h.active("");
      h.history = timelineFromEntries([
        { id: "u", kind: "user", role: "user", createdAtMs: 1, runId: "r", blocks: [{ kind: "text", text: "question" }] },
        { id: "a", kind: "assistant", role: "assistant", createdAtMs: 2, runId: "r", blocks: [{ kind: "text", text: "full reply" }], run: { durationMs: 4200, status: "completed" }, usage: { outputTokens: 32 } },
      ]);
      h.journal.events = []; // Finished replay may have been pruned.
      h.engine.restart("s", reason);
      await h.settle();
      expect(h.engine.streamingFor("s")).toBe(false);
      expect(h.timelineOf("s").items.filter(item => item.kind === "message" && item.role === "assistant"))
        .toEqual([expect.objectContaining({ text: "full reply", durationMs: 4200, outputTokens: 32 })]);
      expect(h.timelineOf("s").items.some(item => item.kind === "message" && item.streaming)).toBe(false);
    } finally { h.engine.clear(); }
  });

  test("idle reopen recovers the cached run's missed terminal event before history catches up", async () => {
    const h = new Harness("r");
    h.journal.add(agentStart("r"));
    h.journal.add(textChunk("r", 1, "reply"));
    try {
      await h.engine.open("s");
      await h.settle();
      h.active("");
      h.journal.add(evt("agent_end", "r", 2, JSON.stringify({ duration_ms: 5100, usage: { output_tokens: 30 } })));
      h.engine.restart("s", "open");
      await h.settle();
      expect(h.engine.streamingFor("s")).toBe(false);
      expect(h.timelineOf("s").items).toEqual([expect.objectContaining({ text: "reply", streaming: false, durationMs: 5100, outputTokens: 30 })]);
    } finally { h.engine.clear(); }
  });

  test("warm history keeps known terminal stats until entry metadata catches up", async () => {
    const h = new Harness("r");
    h.journal.add(agentStart("r"));
    h.journal.add(textChunk("r", 1, "reply"));
    h.journal.add(evt("agent_end", "r", 2, JSON.stringify({ duration_ms: 5100, usage: { output_tokens: 30 } })));
    try {
      await h.engine.open("s");
      await h.settle();
      h.active("");
      h.history.items = [{ id: "a", kind: "message", role: "assistant", runId: "r", text: "reply" }];
      h.engine.restart("s", "open");
      await h.settle();
      expect(h.timelineOf("s").items).toEqual([expect.objectContaining({ id: "a", durationMs: 5100, outputTokens: 30 })]);
    } finally { h.engine.clear(); }
  });

  test("a projection snapshot retains its accumulator for the following live tail", async () => {
    const h = new Harness("r");
    h.projection = [agentStart("r"), textChunk("r", 1, "prefix")];
    try {
      h.engine.reconcile("s", "open");
      await h.settle();
      expect(h.textOf("s")).toBe("prefix");
      h.projection = null;
      h.engine.event("s", textChunk("r", 2, " tail"));
      await h.settle();
      expect(h.textOf("s")).toBe("prefix tail");
    } finally { h.engine.clear(); }
  });

  test("restarting during a cooperative replay sees only committed cursors and drops old work", async () => {
    jest.useFakeTimers();
    let during = -2;
    let calls = 0;
    const engine = new SyncEngine({
      requestGetState: async () => ({ activeRun: { runId: "r" } }),
      requestHistory: async () => emptyTimeline(),
      fetchReplay: async () => {
        calls++;
        if (calls === 1) {
          setTimeout(() => {
            during = engine.cursorFor("s").get("r")?.highWater ?? -1;
            engine.restart("s", "reconnect");
          }, 0);
          return { events: [agentStart("r"), ...Array.from({ length: 10_000 }, (_, i) => textChunk("r", i + 1, "old"))].map(ev => ({ ...ev })), completePrefix: true };
        }
        return { events: [agentStart("r"), textChunk("r", 1, "replacement")].map(ev => ({ ...ev })), completePrefix: true };
      },
    });
    try {
      engine.reconcile("s", "open");
      await jest.runAllTimersAsync();
      expect(during).toBe(-1);
      expect(engine.cursorFor("s").get("r")?.highWater).toBe(1);
      expect(engine.timelineFor("s")?.items.find(item => item.kind === "message"))
        .toMatchObject({ text: "replacement" });
      expect(calls).toBe(2);
    } finally { engine.clear(); jest.useRealTimers(); }
  });


  test("publishes cold history before slow replay without claiming a complete prefix", async () => {
    jest.useFakeTimers();
    const history = emptyTimeline();
    history.items = [{ kind: "message", id: "prompt", role: "user", text: "hello", runId: "r" }];
    const replay = jest.fn(async () => {
      await new Promise(resolve => setTimeout(resolve, 16_800));
      return { events: [agentStart("r"), textChunk("r", 1, "reply"), agentEnd("r", 2)].map(event => ({ ...event })), completePrefix: true };
    });
    const engine = new SyncEngine({
      requestGetState: async () => ({ activeRun: { runId: "r" } }),
      requestHistory: async () => history,
      fetchReplay: replay,
    });
    const commits = jest.fn();
    engine.subscribe(commits);
    try {
      engine.reconcile("s", "open");
      await jest.advanceTimersByTimeAsync(15_000);
      expect(commits).toHaveBeenCalledTimes(1);
      expect(engine.timelineFor("s")?.items).toEqual(history.items);
      expect(engine.streamingFor("s")).toBe(true);
      expect(engine.cursorFor("s").size).toBe(0);
      await jest.advanceTimersByTimeAsync(1_801);
      expect(engine.timelineFor("s")?.items.filter(item => item.id === "prompt")).toHaveLength(1);
      expect(engine.streamingFor("s")).toBe(false);
      expect(engine.cursorFor("s").size).toBe(1);
    } finally {
      engine.clear();
      jest.useRealTimers();
    }
  });

  test("keeps early history readable after replay failure and retries the full prefix", async () => {
    jest.useFakeTimers();
    const h = new Harness("r");
    h.history.items = [{ kind: "message", id: "prompt", role: "user", text: "hello", runId: "r" }];
    h.replayFailures = 1;
    h.journal.add(agentStart("r"));
    h.journal.add(textChunk("r", 1, "reply"));
    try {
      h.engine.reconcile("s", "open");
      await jest.advanceTimersByTimeAsync(10);
      expect(h.textOf("s")).toBe("hello");
      expect(h.engine.cursorFor("s").size).toBe(0);
      await jest.advanceTimersByTimeAsync(600);
      expect(h.textOf("s")).toBe("helloreply");
    } finally {
      h.engine.clear();
      jest.useRealTimers();
    }
  });

  beforeEach(() => {
    resetSeq();
  });

  test("an empty successful replay retains the gap tail and retries without a hot loop", async () => {
    jest.useFakeTimers();
    const run = nextRunId();
    const h = new Harness(run);
    const replay = jest.spyOn(h.journal, "since");
    try {
      h.journal.add(agentStart(run));
      h.journal.add(textChunk(run, 1, "a"));
      h.engine.reconcile("s1", "open");
      await jest.advanceTimersByTimeAsync(20);
      h.engine.event("s1", textChunk(run, 3, "c"));
      h.engine.event("s1", agentEnd(run, 4));
      h.engine.mutate("s1", timeline => ({ ...timeline, durableItemIds: new Set(["retained-mutation"]) }));
      await jest.advanceTimersByTimeAsync(20);
      expect(h.textOf("s1")).toBe("a");
      const callsAfterGap = replay.mock.calls.length;
      await jest.advanceTimersByTimeAsync(400);
      expect(replay).toHaveBeenCalledTimes(callsAfterGap);
      h.journal.add(textChunk(run, 2, "b"));
      h.journal.add(textChunk(run, 3, "c"));
      h.journal.add(agentEnd(run, 4));
      await jest.advanceTimersByTimeAsync(200);
      expect(h.textOf("s1")).toBe("abc");
      expect(h.timelineOf("s1").streaming).toBe(false);
      expect(h.timelineOf("s1").durableItemIds?.has("retained-mutation")).toBe(true);
    } finally {
      h.engine.clear();
      jest.useRealTimers();
    }
  });

  test("clear drops pairing state but preserves the Provider commit subscription", async () => {
    const h = new Harness();
    const commits = jest.fn();
    h.engine.subscribe(commits);

    h.engine.mutate("", () => ({ ...emptyTimeline(), streaming: true }));
    await h.settle();
    h.engine.clear();
    h.engine.mutate("", () => ({ ...emptyTimeline(), streaming: false }));
    await h.settle();

    expect(commits).toHaveBeenCalledTimes(2);
    expect(commits.mock.calls[1][0].timeline.streaming).toBe(false);
  });

  test("clear rejects a late commit from the previous pairing generation", async () => {
    let releaseState: ((state: { activeRun?: { runId: string } }) => void) | undefined;
    const state = new Promise<{ activeRun?: { runId: string } }>((resolve) => {
      releaseState = resolve;
    });
    const engine = new SyncEngine({
      requestGetState: async () => state,
      requestHistory: async () => emptyTimeline(),
      fetchReplay: async () => ({ events: [], completePrefix: false }),
    });
    const commits = jest.fn();
    engine.subscribe(commits);

    engine.event("old-session", agentStart("old-run"));
    await new Promise((resolve) => setTimeout(resolve, 0));
    engine.clear();
    releaseState?.({ activeRun: { runId: "old-run" } });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(commits).not.toHaveBeenCalled();

    engine.mutate("new-session", () => emptyTimeline());
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(commits).toHaveBeenCalledTimes(1);
    expect(commits.mock.calls[0][0].sessionId).toBe("new-session");
  });

  test("restart bypasses a request still pending on the previous connection", async () => {
    let releaseOldState: (() => void) | undefined;
    const oldState = new Promise<{ activeRun?: { runId: string } }>((resolve) => {
      releaseOldState = () => resolve({});
    });
    let stateReads = 0;
    const engine = new SyncEngine({
      requestGetState: async () => {
        stateReads += 1;
        return stateReads === 1 ? oldState : {};
      },
      requestHistory: async () => ({
        ...emptyTimeline(),
        items: [{ id: "history", kind: "message", role: "user", text: "restored" }],
      }),
      fetchReplay: async () => ({ events: [], completePrefix: false }),
    });
    const commits = jest.fn();
    engine.subscribe(commits);

    engine.reconcile("s1", "open");
    await new Promise((resolve) => setTimeout(resolve, 0));
    engine.restart("s1", "reconnect");
    await new Promise((resolve) => setTimeout(resolve, 20));

    expect(stateReads).toBe(2);
    expect(commits).toHaveBeenCalledTimes(1);
    expect(commits.mock.calls[0][0].timeline.items[0]).toMatchObject({ text: "restored" });

    releaseOldState?.();
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(commits).toHaveBeenCalledTimes(1);
  });

  test("reports the failed stage and scheduled retry for diagnostics", async () => {
    const onFailure = jest.fn();
    const engine = new SyncEngine({
      requestGetState: async () => ({ activeRun: { runId: "run-1" } }),
      requestHistory: async () => emptyTimeline(),
      fetchReplay: async () => {
        throw new Error("replay unavailable");
      },
      onFailure,
    });

    engine.reconcile("s1", "open");
    await new Promise((resolve) => setTimeout(resolve, 20));

    expect(onFailure).toHaveBeenCalledWith(
      expect.objectContaining({
        sessionId: "s1",
        runId: "run-1",
        reason: "open",
        stage: "replay",
        attempt: 1,
        retryInMs: 500,
      }),
    );
    engine.clear();
  });

  test("mid-run join replays the prefix from -1 (H3)", async () => {
    const run = nextRunId();
    const h = new Harness(run);
    h.journal.add(agentStart(run, 0));
    h.journal.add(textChunk(run, 1, "hello"));
    h.journal.add(textChunk(run, 2, " world"));
    h.journal.add(agentEnd(run, 3));

    // First contact is mid-run (idx 2). The open reconcile must fetch the
    // whole run from the journal, not just the live tail.
    h.engine.event("s1", textChunk(run, 2, " world"));
    await h.settle();

    expect(h.textOf("s1")).toBe("hello world");
    expect(h.timelineOf("s1").streaming).toBe(false);
  });

  test("reconnect full-reconciles so a settled run keeps its full text (H5)", async () => {
    const run = nextRunId();
    const h = new Harness(run);
    h.journal.add(agentStart(run, 0));
    h.journal.add(textChunk(run, 1, "first half"));
    h.journal.add(textChunk(run, 2, " + second half"));
    h.journal.add(agentEnd(run, 3));

    // First contact, mid-run — the prefix reconcile heals the head.
    h.engine.event("s1", textChunk(run, 1, "first half"));
    await h.settle();
    expect(h.timelineOf("s1").streaming).toBe(false);

    // A reconnect refreshes durable history and replays the run even when its
    // previously observed prefix was complete.
    h.engine.reconcileAll("reconnect");
    await h.settle();
    expect(h.textOf("s1")).toBe("first half + second half");
    expect(h.timelineOf("s1").streaming).toBe(false);
  });

  // The regression that pinned the phone behind a sync notice: the lean trim
  // omits source indices by design, so a lane that trims nothing must be the
  // only one that reads a jump as loss.
  test("a lane that omits indices applies across the hole instead of replaying it", async () => {
    const run = nextRunId();
    const h = new Harness(run);
    try {
      h.engine.event("s1", agentStart(run, 0));
      h.engine.event("s1", textChunk(run, 1, "a"));
      await h.settle();
      expect(h.textOf("s1")).toBe("a");

      h.feedOmitsIndices = true;
      const before = h.replayCalls.length;
      // The frames at idx 2..8 were dropped by the trim (reasoning and argument
      // deltas), so the next survivor arrives above the cursor.
      h.engine.event("s1", textChunk(run, 9, "z"));
      await h.settle();

      expect(h.textOf("s1")).toBe("az");
      expect(h.replayCalls.length).toBe(before);
    } finally {
      h.engine.clear();
    }
  });

  test("the same jump on a lane that trims nothing is still a gap", async () => {
    const run = nextRunId();
    const h = new Harness(run);
    try {
      h.engine.event("s1", agentStart(run, 0));
      h.engine.event("s1", textChunk(run, 1, "a"));
      await h.settle();

      const before = h.replayCalls.length;
      h.engine.event("s1", textChunk(run, 9, "z"));
      await h.settle();

      expect(h.replayCalls.length).toBeGreaterThan(before);
    } finally {
      // `npm test` runs jest with `--runInBand`, so a lane left holding its
      // retry timer keeps the process from exiting and hangs CI instead of
      // failing it: a gap verdict schedules one, and this test provokes one on
      // purpose.
      h.engine.clear();
    }
  });

  test("gap during live streaming fills the hole from the journal (M4)", async () => {
    const run = nextRunId();
    const h = new Harness(run);
    // Establish the run's prefix first.
    h.journal.add(agentStart(run, 0));
    h.journal.add(textChunk(run, 1, "a"));
    h.engine.event("s1", agentStart(run, 0));
    h.engine.event("s1", textChunk(run, 1, "a"));
    await h.settle();
    expect(h.textOf("s1")).toBe("a");

    // The journal grows to the full run; the live relay drops b (idx 2).
    h.journal.add(textChunk(run, 2, "b"));
    h.journal.add(textChunk(run, 3, "c"));
    h.journal.add(agentEnd(run, 4));
    h.engine.event("s1", textChunk(run, 3, "c"));
    await h.settle();

    expect(h.textOf("s1")).toBe("abc");
  });

  test("coalesces established live deltas into one frame commit", async () => {
    const run = nextRunId();
    const h = new Harness(run);
    h.engine.event("s1", agentStart(run, 0));
    await h.settle();

    const commits = jest.fn();
    h.engine.subscribe(commits);
    h.engine.event("s1", textChunk(run, 1, "a"));
    // Model separate NATS deliveries while keeping both inside one frame.
    await Promise.resolve();
    h.engine.event("s1", textChunk(run, 2, "b"));
    await h.settle();

    expect(commits).toHaveBeenCalledTimes(1);
    expect(h.textOf("s1")).toBe("ab");
  });

  test("a failed gap replay preserves queued events and retries automatically", async () => {
    const run = nextRunId();
    const h = new Harness(run);
    h.journal.add(agentStart(run, 0));
    h.journal.add(textChunk(run, 1, "a"));
    h.engine.event("s1", agentStart(run, 0));
    h.engine.event("s1", textChunk(run, 1, "a"));
    await h.settle();

    h.journal.add(textChunk(run, 2, "b"));
    h.journal.add(textChunk(run, 3, "c"));
    h.journal.add(agentEnd(run, 4));
    h.replayFailures = 1;
    h.engine.event("s1", textChunk(run, 3, "c"));
    h.engine.event("s1", agentEnd(run, 4));

    // The first reconcile fails. The lane-owned retry fires after 500ms and
    // must retain the gap event plus the terminal event queued behind it.
    await new Promise((resolve) => setTimeout(resolve, 650));
    expect(h.textOf("s1")).toBe("abc");
    expect(h.timelineOf("s1").streaming).toBe(false);
  });

  test("agent_end drops are healed by a snapshot-flip reconcile (M11)", async () => {
    const run = nextRunId();
    const h = new Harness(run);
    // Establish the run's prefix first.
    h.journal.add(agentStart(run, 0));
    h.journal.add(textChunk(run, 1, "full reply"));
    h.engine.event("s1", agentStart(run, 0));
    h.engine.event("s1", textChunk(run, 1, "full reply"));
    await h.settle();
    expect(h.timelineOf("s1").streaming).toBe(true);

    // The journal grows to include the end; the live relay dropped it
    // (at-most-once). The client is still "generating".
    h.journal.add(agentEnd(run, 2));
    h.engine.event("s1", textChunk(run, 1, "full reply")); // dedup; no-op
    await h.settle();
    expect(h.timelineOf("s1").streaming).toBe(true);

    // The desktop's sessions snapshot flips streaming → a reconcile on the
    // run recovers the dropped end from the journal.
    h.engine.reconcile("s1", "snapshot-flip", run);
    await h.settle();
    expect(h.timelineOf("s1").streaming).toBe(false);
    expect(h.textOf("s1")).toBe("full reply");
  });

  test("a reconcile that finds no active run clears a stale generating flag and adopts the fence", async () => {
    // The desktop reports no active run (the process restarted, or the run was
    // evicted from state) while the phone still shows this run as generating.
    // Nothing will ever send the terminal frame, so the reconcile has to drop
    // the flag itself or the composer stays locked forever.
    const run = nextRunId();
    const h = new Harness("");
    h.journal.add(agentStart(run, 0));
    h.engine.event("s1", agentStart(run, 0));
    await h.settle();
    expect(h.timelineOf("s1").streaming).toBe(true);

    // The same state carries the authoritative compaction fence, which the
    // replayed timeline disagrees with: the snapshot wins.
    h.isCompacting = true;
    h.engine.reconcile("s1", "snapshot-flip", run);
    await h.settle();

    expect(h.timelineOf("s1").streaming).toBe(false);
    expect(h.timelineOf("s1").compacting).toBe(true);
  });

  test("out-of-order first delivery still converges to the full text", async () => {
    const run = nextRunId();
    const h = new Harness(run);
    h.journal.add(agentStart(run, 0));
    h.journal.add(textChunk(run, 1, "x"));
    h.journal.add(textChunk(run, 2, "y"));

    // The live stream delivers idx 2 before idx 1 (out of order).
    h.engine.event("s1", textChunk(run, 2, "y"));
    await h.settle();
    // The prefix reconcile recovered 0..1 from the journal.
    h.engine.event("s1", textChunk(run, 1, "x"));
    await h.settle();

    expect(h.textOf("s1")).toBe("xy");
  });

  test("duplicate live events are dropped (dedup)", async () => {
    const run = nextRunId();
    const h = new Harness(run);
    h.journal.add(agentStart(run, 0));
    h.engine.event("s1", textChunk(run, 1, "once"));
    await h.settle();
    h.engine.event("s1", textChunk(run, 1, "once"));
    await h.settle();
    expect(h.textOf("s1")).toBe("once");
  });

  test("mutate applies inside the lane without racing live events", async () => {
    const run = nextRunId();
    const h = new Harness(run);
    h.journal.add(agentStart(run, 0));

    h.engine.event("s1", agentStart(run, 0));
    h.engine.mutate("s1", (tl) => ({
      ...tl,
      items: [
        ...tl.items,
        { id: "opt", kind: "notice" as const, tone: "neutral" as const, text: "sent" },
      ],
    }));
    h.engine.event("s1", textChunk(run, 1, "streamed"));
    await h.settle();

    expect(h.textOf("s1")).toContain("streamed");
    expect(
      h.timelineOf("s1").items.some((item) => item.kind === "notice" && item.text === "sent"),
    ).toBe(true);
  });

  test("a mutation parked behind a reconcile survives the restart a foreground return triggers", async () => {
    const run = nextRunId();
    const h = new Harness(run);
    h.journal.add(agentStart(run, 0));
    h.journal.add(textChunk(run, 1, "reply"));
    try {
      // The opening reconcile is still in flight (a slow desktop) when a caller
      // parks its mutation behind it — the older-history page, an approval
      // decision.
      let releaseState: (() => void) | undefined;
      h.stateBlocked = new Promise<void>((resolve) => { releaseState = resolve; });
      void h.engine.open("s");
      await new Promise((resolve) => setTimeout(resolve, 0));
      const seen: string[] = [];
      h.engine.mutate("s", (timeline) => {
        seen.push(timeline.items.flatMap((item) => (item.kind === "message" ? [item.text] : [])).join(""));
        return {
          ...timeline,
          items: [...timeline.items, { id: "page", kind: "notice" as const, tone: "neutral" as const, text: "older page" }],
        };
      });
      // Backgrounded: the lane may publish nothing, and the reconcile in flight
      // bails out without ever draining the queue that holds the mutation.
      h.visible = false;
      releaseState?.();
      await new Promise((resolve) => setTimeout(resolve, 0));
      expect(h.engine.timelineFor("s")).toBeNull();
      expect(h.engine.canCommit("s")).toBe(false);
      // Foreground again: the provider restarts every real lane.
      h.visible = true;
      h.engine.restart("s", "reconnect");
      await h.settle();
      // Applied once, over the successor's freshly replayed baseline — not the
      // empty timeline the lane held when the mutation was queued.
      expect(seen).toEqual(["reply"]);
      expect(h.timelineOf("s").items.at(-1)).toMatchObject({ id: "page" });
    } finally { h.engine.clear(); }
  });

  test("a mutation the live op budget deferred survives the restart that follows", async () => {
    jest.useFakeTimers();
    const run = nextRunId();
    const h = new Harness(run);
    try {
      h.journal.add(agentStart(run, 0));
      h.journal.add(textChunk(run, 1, "reply"));
      h.engine.reconcile("s", "open");
      await jest.runAllTimersAsync();
      expect(h.textOf("s")).toBe("reply");
      // The drain's first op spends the whole op budget (a slow reducer, as a
      // native-bridge stall would), so it yields with the rest of the batch —
      // the mutation below included — still queued.
      h.engine.mutate("s", (timeline) => {
        jest.setSystemTime(Date.now() + 100);
        return timeline;
      });
      h.engine.mutate("s", (timeline) => ({
        ...timeline,
        items: [...timeline.items, { id: "decision", kind: "notice" as const, tone: "neutral" as const, text: "approved" }],
      }));
      // Let that first drain run, but not the chained step that would drain the
      // deferred tail before the lane is replaced.
      await Promise.resolve();
      expect(h.timelineOf("s").items.some((item) => item.id === "decision")).toBe(false);
      h.engine.restart("s", "reconnect");
      await jest.runAllTimersAsync();
      expect(h.timelineOf("s").items.at(-1)).toMatchObject({ id: "decision" });
      expect(h.textOf("s")).toBe("reply");
    } finally { h.engine.clear(); jest.useRealTimers(); }
  });

  test("projection replay without run_id does not leave a streaming ghost (regression)", async () => {
    const run = nextRunId();
    const h = new Harness(run);
    // Desktop folds the settled run into a projection whose events omit run_id.
    h.projection = [agentStart(run, 0), textChunk(run, 1, "folded reply"), agentEnd(run, 2)];
    h.engine.reconcile("s1", "resend", run);
    await h.settle();
    expect(h.textOf("s1")).toBe("folded reply");
    expect(h.timelineOf("s1").streaming).toBe(false);
    const assistantItems = h
      .timelineOf("s1")
      .items.filter((item) => item.kind === "message" && item.role === "assistant");
    expect(assistantItems).toHaveLength(1);
    expect(assistantItems[0]).toMatchObject({ runId: run, streaming: false });
  });

  test("open reconcile on an idle session loads durable history (no active run)", async () => {
    const h = new Harness(); // no active run, no live events
    h.history = {
      ...emptyTimeline(),
      items: [
        { id: "h1", kind: "message", role: "user", text: "Hello", runId: "r1" },
        { id: "h2", kind: "message", role: "assistant", text: "Hi there" },
      ],
    };
    // Entering the session enqueues an "open" reconcile; with no active run
    // there is nothing to tail-replay, so the timeline must come from history.
    h.engine.reconcile("s1", "open");
    await h.settle();
    expect(h.textOf("s1")).toBe("HelloHi there");
  });

  test("first live contact on an idle-established session keeps live tail", async () => {
    const h = new Harness();
    h.history = {
      ...emptyTimeline(),
      items: [{ id: "h1", kind: "message", role: "user", text: "old" }],
    };
    const run = nextRunId();
    h.journal.add(agentStart(run, 0));
    h.journal.add(textChunk(run, 1, " new"));
    h.engine.event("s1", agentStart(run, 0));
    h.engine.event("s1", textChunk(run, 1, " new"));
    await h.settle();
    expect(h.textOf("s1")).toBe("old new");
  });

  test("user_message live mirror is applied (optimistic bubble lands)", async () => {
    const run = nextRunId();
    const h = new Harness(run);
    const user = evt("user_message", run, 5, JSON.stringify({ text: "hi there" }));
    h.engine.event("s1", user);
    await h.settle();

    const userItems = h
      .timelineOf("s1")
      .items.filter((item) => item.kind === "message" && item.role === "user");
    expect(userItems).toHaveLength(1);
    expect(userItems[0]).toMatchObject({ text: "hi there" });
  });

  test("query accessors return empty defaults before a lane establishes", () => {
    const h = new Harness();
    expect(h.engine.timelineFor("nope")).toBeNull();
    expect(h.engine.cursorFor("nope").size).toBe(0);
    expect(h.engine.streamingFor("nope")).toBe(false);
    h.engine.clear();
    expect(h.engine.timelineFor("nope")).toBeNull();
  });

  test("untracked events (no runId) apply directly", async () => {
    const h = new Harness();
    h.engine.event("s1", { type: "user_message", data: JSON.stringify({ text: "hi" }) });
    await h.settle();
    expect(h.textOf("s1")).toBe("hi");
  });

  test("a run settling in a batch is not re-read when it arrived whole", async () => {
    const run = nextRunId();
    const h = new Harness(run);
    h.journal.add(agentStart(run, 0));
    h.journal.add(textChunk(run, 1, "reply"));

    h.engine.event("s1", agentStart(run, 0));
    h.engine.event("s1", textChunk(run, 1, "reply"));
    await h.settle();
    expect(h.timelineOf("s1").streaming).toBe(true);

    const reads = h.replayCalls.length;
    h.journal.add(agentEnd(run, 2));
    h.engine.event("s1", agentEnd(run, 2));
    await h.settle();
    expect(h.timelineOf("s1").streaming).toBe(false);
    expect(h.textOf("s1")).toBe("reply");
    // The terminal arrived over a contiguous prefix, so the run is whole here:
    // re-reading it would download what is on screen and show the sync notice
    // for it. `agent_end` is a finished run's last event.
    expect(h.replayCalls.length).toBe(reads);
  });

  test("a settle still reconciles when the prefix is incomplete", async () => {
    const run = nextRunId();
    const h = new Harness(run);
    // The journal's head is no longer readable (only 0 and 5 survive), so a
    // replay cannot establish a contiguous prefix even though the run is live.
    h.journal.add(agentStart(run, 0));
    h.journal.add(textChunk(run, 5, "tail"));

    h.engine.event("s1", textChunk(run, 5, "tail"));
    await h.settle();
    expect(h.timelineOf("s1").streaming).toBe(true);
    expect(h.engine.runCompleteLocally("s1", run)).toBe(false);

    const reads = h.replayCalls.length;
    h.journal.add(agentEnd(run, 6));
    h.engine.event("s1", agentEnd(run, 6));
    await h.settle();
    expect(h.timelineOf("s1").streaming).toBe(false);
    // The terminal proves the end, not the beginning: the run is still read
    // back so the unreadable prefix can be recovered.
    expect(h.replayCalls.length).toBeGreaterThan(reads);
  });

  test("a dropped terminal is still healed by the catalog's snapshot flip (M11)", async () => {
    const run = nextRunId();
    const h = new Harness(run);
    h.journal.add(agentStart(run, 0));
    h.journal.add(textChunk(run, 1, "full reply"));

    h.engine.event("s1", agentStart(run, 0));
    h.engine.event("s1", textChunk(run, 1, "full reply"));
    await h.settle();

    // The journal grows to include the end; the live relay dropped it, so this
    // client still believes the run is generating — the state that must keep
    // healing rather than being skipped as complete.
    h.journal.add(agentEnd(run, 2));
    expect(h.engine.runCompleteLocally("s1", run)).toBe(false);
    h.engine.reconcile("s1", "snapshot-flip", run);
    await h.settle();
    expect(h.timelineOf("s1").streaming).toBe(false);
    expect(h.textOf("s1")).toBe("full reply");
    expect(h.engine.runCompleteLocally("s1", run)).toBe(true);
  });

  /**
   * A projection with no usable cursor is refused rather than mis-cursored.
   *
   * A legacy host can answer a folded projection without a boundary, and the
   * cursor is what the next read resumes from: guessing it from the events would
   * make this client claim to hold a range it was never given.
   */
  test("a projection without a usable cursor is refused, and the prefix is not built from it", async () => {
    const run = nextRunId();
    const h = new Harness(run);
    h.projectionCursor = -1;
    h.projection = [agentStart(run, 0), textChunk(run, 1, "folded"), agentEnd(run, 2)];
    try {
      h.engine.reconcile("s1", "resend", run);
      await h.settle();
      // Nothing from that projection was applied, and the lane is retrying from
      // durable state rather than carrying a cursor nobody can justify.
      expect(h.textOf("s1")).not.toContain("folded");
      expect(h.engine.cursorFor("s1").get(run)).toBeUndefined();
    }
    finally {
      // A refused reply arms the lane's retry timer, and this test is not running
      // on fake timers: leaving it armed keeps the process alive after the run.
      h.engine.clear();
    }
  });

  test("restartAll rebuilds every real lane from durable state", async () => {
    const run = nextRunId();
    const h = new Harness(run);
    h.journal.add(agentStart(run, 0));
    h.journal.add(textChunk(run, 1, "hello"));
    h.journal.add(agentEnd(run, 2));

    h.engine.event("s1", textChunk(run, 1, "hello"));
    await h.settle();
    expect(h.textOf("s1")).toBe("hello");

    // A reconnect rebuilds every real lane (the draft lane "" is skipped).
    h.engine.restartAll("reconnect");
    await h.settle();
    expect(h.textOf("s1")).toBe("hello");
    expect(h.timelineOf("s1").streaming).toBe(false);
  });

  test("a step during retry backoff defers to the pending retry timer", async () => {
    const run = nextRunId();
    const h = new Harness(run);
    h.journal.add(agentStart(run, 0));
    h.journal.add(textChunk(run, 1, "a"));
    h.engine.event("s1", agentStart(run, 0));
    h.engine.event("s1", textChunk(run, 1, "a"));
    await h.settle();

    h.journal.add(textChunk(run, 2, "b"));
    h.journal.add(textChunk(run, 3, "c"));
    h.journal.add(agentEnd(run, 4));
    h.replayFailures = 1;
    h.engine.event("s1", textChunk(run, 3, "c"));
    h.engine.event("s1", agentEnd(run, 4));
    await h.settle();

    // The first reconcile failed and armed a retry timer. A mutation arriving
    // inside the backoff window re-runs step(), which must defer to the pending
    // timer (retryNotBefore > now) instead of re-entering the reconcile.
    h.engine.mutate("s1", (tl) => tl);
    await new Promise((resolve) => setTimeout(resolve, 650));
    expect(h.textOf("s1")).toBe("abc");
    expect(h.timelineOf("s1").streaming).toBe(false);
  });

  test.each(["open", "reconnect"] as const)(
    "%s preserves turn order when cached replies become durable during another run",
    async reason => {
      const h = new Harness("r1");
      const firstPrompt: HistoryEntry = {
        id: "u1", kind: "user", role: "user", createdAtMs: 1, runId: "r1",
        blocks: [{ kind: "text", text: "first question" }],
      };
      h.history = timelineFromEntries([firstPrompt]);
      h.journal.add(agentStart("r1"));
      h.journal.add(textChunk("r1", 1, "first reply"));
      h.journal.add(agentEnd("r1", 2));
      try {
        await h.engine.open("s");
        await h.settle();
        expect(h.timelineOf("s").items.map(item => item.id)).toEqual(["m_u1", "assistant:r1"]);

        // While the phone is away, r1 is persisted with its entry id and a
        // second prompt starts. Reopening must not append the cached r1 reply
        // after u2 just because its live id differs from the durable entry id.
        h.active("r2");
        h.history = timelineFromEntries([
          firstPrompt,
          {
            id: "a1", kind: "assistant", role: "assistant", createdAtMs: 2, runId: "r1",
            blocks: [{ kind: "text", text: "first reply" }],
          },
          {
            id: "u2", kind: "user", role: "user", createdAtMs: 3, runId: "r2",
            blocks: [{ kind: "text", text: "second question" }],
            metadata: { attachments: [{ path: "/photo.jpg", name: "photo.jpg", kind: "image" }] },
          },
        ]);
        // The replay endpoint returns only the requested run.
        h.journal.events = [agentStart("r2"), textChunk("r2", 1, "second reply")];
        for (let reopen = 0; reopen < 2; reopen += 1) {
          h.engine.restart("s", reason);
          await h.settle();
          expect(h.timelineOf("s").items.map(item => item.id)).toEqual([
            "m_u1", "m_a1", "m_u2", "assistant:r2",
          ]);
          expect(h.timelineOf("s").items[2]).toMatchObject({
            role: "user", text: "second question", attachments: [{ name: "photo.jpg" }],
          });
          expect(h.timelineOf("s").streaming).toBe(true);
        }
      } finally {
        h.engine.clear();
      }
    },
  );

  test("history dedup requires both role and run identity, not matching text", async () => {
    const h = new Harness();
    h.history.items = [
      { id: "u1", kind: "message", role: "user", text: "same", runId: "r1" },
      { id: "a1", kind: "message", role: "assistant", text: "same", runId: "r1" },
      { id: "u2", kind: "message", role: "user", text: "same", runId: "r2" },
    ];
    try {
      h.engine.mutate("s", live => ({
        ...live,
        items: [{ id: "assistant:r2", kind: "message", role: "assistant", text: "same", runId: "r2" }],
      }));
      await h.settle();
      await h.engine.open("s");
      await h.settle();
      expect(h.timelineOf("s").items.map(item => item.id)).toEqual([
        "u1", "a1", "u2", "assistant:r2",
      ]);
    } finally {
      h.engine.clear();
    }
  });

  test("full reconcile drops a live user mirror duplicating a durable prompt", async () => {
    const h = new Harness();
    h.history = {
      ...emptyTimeline(),
      items: [
        { id: "h1", kind: "message", role: "user", text: "Hello", runId: "r1" },
        { id: "h2", kind: "message", role: "assistant", text: "Hi" },
      ],
    };
    h.engine.reconcile("s1", "open");
    await h.settle();
    expect(h.timelineOf("s1").items.map((i) => i.id)).toEqual(["h1", "h2"]);

    h.engine.mutate("s1", (tl) => ({
      ...tl,
      items: [
        ...tl.items,
        { id: "local-dup", kind: "message", role: "user", text: "Hello", runId: "r1" },
        { id: "notice-1", kind: "notice", tone: "neutral", text: "kept" },
      ],
    }));
    await h.settle();

    h.engine.reconcile("s1", "resend");
    await h.settle();

    expect(h.timelineOf("s1").items.map((i) => i.id)).toEqual(["h1", "h2", "notice-1"]);
  });
  test("live queue overflow converges from the durable journal without losing text", async () => {
    jest.useFakeTimers();
    const run = nextRunId();
    const h = new Harness(run);
    try {
      h.journal.add(agentStart(run));
      h.engine.event("s1", agentStart(run));
      for (let idx = 1; idx <= 4200; idx += 1) {
        const event = textChunk(run, idx, "a");
        h.journal.add(event);
        h.engine.event("s1", event);
      }
      // Replay intentionally crosses task boundaries now; completion, not a
      // 20ms wall-clock sleep, is the relevant assertion boundary.
      await jest.runAllTimersAsync();
      expect(h.textOf("s1")).toBe("a".repeat(4200));
    } finally { h.engine.clear(); jest.useRealTimers(); }
  });
});

/**
 * Lane lifecycle guards: what a lane does when it is hidden, replaced, or
 * evicted while work is queued, and the two integrity cases that recover from
 * the durable journal instead of the live lane.
 */
describe("lane lifecycle guards", () => {
  test("a single oversized live frame is dropped and recovered from the journal", async () => {
    const run = nextRunId();
    const h = new Harness(run);
    try {
      h.journal.add(agentStart(run, 0));
      // The journal holds the truth; the live frame below carries a different
      // text of a size no NATS reply could have delivered.
      h.journal.add(textChunk(run, 1, "durable"));
      h.engine.reconcile("s1", "open");
      await h.settle();
      const oversized = textChunk(run, 2, "x".repeat(4 * 1024 * 1024));
      h.engine.event("s1", oversized);
      await h.settle();
      // The frame is dropped without buffering 8 MiB of it, and its content is
      // still readable — the durable replay is what carries it.
      expect(h.textOf("s1")).toBe("durable");
      expect(h.engine.timelineFor("s1")!.streaming).toBe(true);
    } finally { h.engine.clear(); }
  });

  test("a run id is required to call a cached run complete", async () => {
    const run = nextRunId();
    const h = new Harness(run);
    try {
      h.journal.add(agentStart(run, 0));
      h.journal.add(agentEnd(run, 1));
      h.engine.reconcile("s1", "open");
      await h.settle();
      expect(h.engine.runCompleteLocally("s1", run)).toBe(true);
      // No run named: there is nothing to be complete, and a lane that happens
      // to hold a whole run must not answer yes for it.
      expect(h.engine.runCompleteLocally("s1", "")).toBe(false);
    } finally { h.engine.clear(); }
  });

  test("a truncated replay marks the timeline so the hole is visible", async () => {
    const run = nextRunId();
    const h = new Harness(run);
    try {
      h.journal.add(agentStart(run, 0));
      h.journal.add(textChunk(run, 1, "partial"));
      h.truncateReplay = true;
      h.engine.reconcile("s1", "open");
      await h.settle();
      // Silently rendering a prefix would look like the run only said "partial".
      expect(h.timelineOf("s1").items.some(
        (item) => item.kind === "notice" && item.text === "truncated",
      )).toBe(true);
      expect(h.textOf("s1")).toBe("partial");
    } finally { h.engine.clear(); }
  });

  test("the draft lane reconciles nothing", async () => {
    const h = new Harness();
    try {
      h.engine.reconcile("", "open");
      await h.settle();
      // The optimistic draft has no desktop state: asking for one would show a
      // session the desktop never opened, and would hold a lane open for it.
      expect(h.stateCalls).toBe(0);
      expect(h.replayCalls).toHaveLength(0);
      expect(h.engine.timelineFor("")).toBeNull();
    } finally { h.engine.clear(); }
  });

  test("a reconcile with no run to tail does not spend a replay", async () => {
    const h = new Harness();
    try {
      h.history = {
        ...emptyTimeline(),
        items: [{ id: "h1", kind: "message", role: "user", text: "hello" }],
      };
      h.engine.reconcile("s1", "open");
      await h.settle();
      const replays = h.replayCalls.length;
      // The session is settled and idle, and this reconcile names no run: there
      // is no tail to fold, so nothing may be fetched.
      h.engine.reconcile("s1", "snapshot-flip");
      await h.settle();
      expect(h.replayCalls).toHaveLength(replays);
      expect(h.textOf("s1")).toBe("hello");
    } finally { h.engine.clear(); }
  });

  test("the queued reconcile instruction list is bounded", async () => {
    const run = nextRunId();
    const h = new Harness(run);
    try {
      h.journal.add(agentStart(run, 0));
      h.journal.add(textChunk(run, 1, "one"));
      // Hold the first request open so every later instruction queues behind it
      // instead of draining.
      let unblock!: () => void;
      h.stateBlocked = new Promise<void>(resolve => { unblock = resolve; });
      h.engine.reconcile("s1", "open");
      await h.settle();
      for (let index = 0; index < 40; index += 1) {
        h.engine.reconcile("s1", "gap", `run-${index}`);
      }
      const stateCalls = h.stateCalls;
      unblock();
      h.stateBlocked = null;
      await h.settle();
      await h.settle();
      // A desktop that keeps announcing gaps must not grow the queue without
      // bound: the lane drains its capped backlog and stops, rather than
      // fetching once per announcement it ever received.
      expect(h.replayCalls.length).toBeLessThanOrEqual(7);
      expect(h.stateCalls).toBeLessThanOrEqual(stateCalls + 6);
      expect(h.engine.timelineFor("s1")).not.toBeNull();
    } finally { h.engine.clear(); }
  });

  test("a lane hidden while a retry is armed neither fires it nor retries again", async () => {
    jest.useFakeTimers();
    const run = nextRunId();
    const h = new Harness(run);
    try {
      h.journal.add(agentStart(run, 0));
      h.engine.reconcile("s1", "open");
      await jest.runAllTimersAsync();
      h.replayFailures = 1;
      h.engine.reconcile("s1", "resend");
      // One tick is enough for the failing request to complete and for its
      // retry to be armed (the first backoff is hundreds of milliseconds, so it
      // is still pending below).
      await jest.advanceTimersByTimeAsync(1);
      expect(h.failures.length).toBeGreaterThan(0);
      // The user left the conversation while the retry was pending.
      h.visible = false;
      const replays = h.replayCalls.length;
      await jest.advanceTimersByTimeAsync(60_000);
      // A hidden session has no visible timeline to repair: the armed retry and
      // any later backoff must both end without touching the desktop again.
      expect(h.replayCalls).toHaveLength(replays);
      expect(h.failures).toHaveLength(1);
    } finally { h.engine.clear(); jest.useRealTimers(); }
  });

  test("a failure that is reported as the session is hidden does not arm a retry", async () => {
    jest.useFakeTimers();
    const run = nextRunId();
    const h = new Harness(run);
    try {
      h.journal.add(agentStart(run, 0));
      h.engine.reconcile("s1", "open");
      await jest.runAllTimersAsync();
      h.replayFailures = 1;
      // Hiding the session from inside the failure hook is what the provider
      // does when navigation happens mid-request: the lane is already gone from
      // the visible set when the retry would be armed.
      h.onFailure = () => { h.visible = false; };
      h.engine.reconcile("s1", "resend");
      await jest.advanceTimersByTimeAsync(1);
      expect(h.failures).toHaveLength(1);
      const afterFailure = h.replayCalls.length;
      // Neither the instruction that just failed nor a backoff timer may spend
      // another request on a session that is no longer on screen.
      await jest.advanceTimersByTimeAsync(200_000);
      expect(h.replayCalls).toHaveLength(afterFailure);
      expect(h.failures).toHaveLength(1);
    } finally { h.engine.clear(); jest.useRealTimers(); }
  });

  test("a clock jump past a pending retry cancels it instead of replaying twice", async () => {
    jest.useFakeTimers();
    const run = nextRunId();
    const h = new Harness(run);
    try {
      h.journal.add(agentStart(run, 0));
      h.journal.add(textChunk(run, 1, "text"));
      h.engine.reconcile("s1", "open");
      await jest.runAllTimersAsync();
      h.replayFailures = 1;
      h.engine.reconcile("s1", "resend");
      await jest.advanceTimersByTimeAsync(1);
      expect(h.failures).toHaveLength(1);
      // The device's wall clock jumps past the retry's due time (a suspended
      // JS VM, or a manual clock change) without the timer callback running.
      jest.setSystemTime(Date.now() + 120_000);
      const replays = h.replayCalls.length;
      h.engine.reconcile("s1", "gap", run);
      await jest.advanceTimersByTimeAsync(0);
      const settled = h.replayCalls.length;
      expect(settled).toBeGreaterThan(replays);
      // The now-overdue timer must not fire a second reconcile on top of the
      // one that just succeeded.
      await jest.advanceTimersByTimeAsync(200_000);
      expect(h.replayCalls).toHaveLength(settled);
    } finally { h.engine.clear(); jest.useRealTimers(); }
  });

  test("clearing the engine cancels a live flush that is still waiting", async () => {
    jest.useFakeTimers();
    const run = nextRunId();
    const h = new Harness(run);
    try {
      h.journal.add(agentStart(run, 0));
      h.journal.add(textChunk(run, 1, "first"));
      h.engine.reconcile("s1", "open");
      await jest.runAllTimersAsync();
      expect(h.textOf("s1")).toBe("first");
      // A token queues a deferred frame commit, then the user unpairs before
      // the display frame elapses.
      h.engine.event("s1", textChunk(run, 2, " late"));
      h.engine.clear();
      await jest.advanceTimersByTimeAsync(1_000);
      // The deferred frame belongs to the discarded pairing: it must not commit
      // into a lane the engine has already dropped.
      expect(h.engine.timelineFor("s1")).toBeNull();
      expect(h.textOf("s1")).toBe("first");
    } finally { h.engine.clear(); jest.useRealTimers(); }
  });

  test("evicting a lane cancels its pending retry", async () => {
    jest.useFakeTimers();
    const run = nextRunId();
    const h = new Harness(run);
    try {
      h.journal.add(agentStart(run, 0));
      h.engine.reconcile("s1", "open");
      await jest.runAllTimersAsync();
      h.replayFailures = 1;
      h.engine.reconcile("s1", "resend");
      await jest.advanceTimersByTimeAsync(1);
      expect(h.failures).toHaveLength(1);
      const replays = h.replayCalls.length;
      // The cache budget forces this lane out while its retry is still armed.
      h.engine.pruneCache("other", 0, 0);
      expect(h.engine.timelineFor("s1")).toBeNull();
      await jest.advanceTimersByTimeAsync(120_000);
      // An evicted lane must not keep probing the desktop from the background.
      expect(h.replayCalls).toHaveLength(replays);
    } finally { h.engine.clear(); jest.useRealTimers(); }
  });
});
