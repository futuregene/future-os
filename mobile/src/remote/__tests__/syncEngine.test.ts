import { emptyTimeline } from "../timeline";
import { SyncEngine, type ReplayResult } from "../syncEngine";
import type { StreamEvent } from "../types";

/** Deterministic run id generator. */
let seq = 0;
function resetSeq(): void {
  seq = 0;
}
function nextRunId(): string {
  seq += 1;
  return `run-${seq}`;
}

function evt(type: string, run: string, idx: number, data = "{}"): StreamEvent {
  return { type, runId: run, idx, data };
}
function agentStart(run: string, idx = 0): StreamEvent {
  return evt("agent_start", run, idx);
}
function textChunk(run: string, idx: number, text: string): StreamEvent {
  return evt("text_chunk", run, idx, JSON.stringify({ text }));
}
function agentEnd(run: string, idx: number): StreamEvent {
  return evt("agent_end", run, idx);
}

/** A run's durable journal — the truth the replay reads. */
class Journal {
  events: StreamEvent[] = [];
  add(event: StreamEvent): void {
    this.events.push(event);
  }
  since(run: string, from: number): StreamEvent[] {
    return this.events.filter(e =>
      e.runId === run && e.idx != null && from === -1 ? true : (e.idx ?? -1) > from,
    );
  }
}

class Harness {
  journal = new Journal();
  activeRunId = "";
  history: ReturnType<typeof emptyTimeline> = emptyTimeline();
  /** Optional folded projection; when set, fetchReplay returns it instead. */
  projection: StreamEvent[] | null = null;
  /** Omit the folded projection's explicit cursor (derive it from event idx). */
  omitProjectionCursor = false;
  /** Emit replay events with snake_case run_id (legacy desktop wire). */
  snakeCaseReplay = false;
  replayFailures = 0;
  timeline: Record<string, ReturnType<typeof emptyTimeline>> = {};
  engine: SyncEngine;

  constructor(activeRunId = "") {
    this.activeRunId = activeRunId;
    this.engine = new SyncEngine({
      requestGetState: async () => {
        const state: { activeRun?: { runId: string } } = {};
        if (this.activeRunId) state.activeRun = { runId: this.activeRunId };
        return state;
      },
      requestHistory: async () => this.history,
      fetchReplay: async (_sessionId, run, since) => {
        if (this.replayFailures > 0) {
          this.replayFailures -= 1;
          throw new Error("temporary replay failure");
        }
        // The desktop RPC serializes replay events with camelCase runId — the
        // real wire shape that reproduced the missing-runId ghost.
        const runKey = this.snakeCaseReplay ? "run_id" : "runId";
        const events = this.journal
          .since(run, since)
          .map(e => ({ type: e.type, data: e.data, [runKey]: e.runId, idx: e.idx }));
        if (this.projection) {
          // Folded projections carry NO run_id per event (whole-run coalesced
          // deltas) — exactly the wire shape that reproduced the ghost item.
          const wire = this.projection.map(e => ({ type: e.type, data: e.data, idx: e.idx }));
          const projection = this.omitProjectionCursor
            ? { run_id: run, events: wire }
            : { run_id: run, cursor: wire.length - 1, events: wire };
          const result: ReplayResult = {
            events: [],
            projection,
          };
          return result;
        }
        const result: ReplayResult = { events };
        return result;
      },
    });
    this.engine.subscribe(commit => {
      this.timeline[commit.sessionId] = commit.timeline;
    });
  }

  /** Set which run get_state reports as active (the running run). */
  active(run: string): void {
    this.activeRunId = run;
  }

  /** Wait for the lane to drain. */
  async settle(): Promise<void> {
    await new Promise(resolve => setTimeout(resolve, 20));
  }

  /** Timeline committed for a session (thrown if the lane never established). */
  timelineOf(sessionId: string): ReturnType<typeof emptyTimeline> {
    const timeline = this.timeline[sessionId];
    if (!timeline) throw new Error(`no committed timeline for ${sessionId}`);
    return timeline;
  }

  textOf(sessionId: string): string {
    return (this.timeline[sessionId]?.items ?? [])
      .filter(item => item.kind === "message")
      .map(item => (item.kind === "message" ? item.text : ""))
      .join("");
  }
}

describe("SyncEngine", () => {
  beforeEach(() => {
    resetSeq();
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
    const state = new Promise<{ activeRun?: { runId: string } }>(resolve => {
      releaseState = resolve;
    });
    const engine = new SyncEngine({
      requestGetState: async () => state,
      requestHistory: async () => emptyTimeline(),
      fetchReplay: async () => ({ events: [] }),
    });
    const commits = jest.fn();
    engine.subscribe(commits);

    engine.event("old-session", agentStart("old-run"));
    await new Promise(resolve => setTimeout(resolve, 0));
    engine.clear();
    releaseState?.({ activeRun: { runId: "old-run" } });
    await new Promise(resolve => setTimeout(resolve, 20));
    expect(commits).not.toHaveBeenCalled();

    engine.mutate("new-session", () => emptyTimeline());
    await new Promise(resolve => setTimeout(resolve, 20));
    expect(commits).toHaveBeenCalledTimes(1);
    expect(commits.mock.calls[0][0].sessionId).toBe("new-session");
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

    // A reconnect re-reconciles; the prefix is now complete so it re-checks
    // the tail only, but the full text must be stable.
    h.engine.reconcileAll("reconnect");
    await h.settle();
    expect(h.textOf("s1")).toBe("first half + second half");
    expect(h.timelineOf("s1").streaming).toBe(false);
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
    await new Promise(resolve => setTimeout(resolve, 650));
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
    h.engine.mutate("s1", tl => ({
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
      h.timelineOf("s1").items.some(item => item.kind === "notice" && item.text === "sent"),
    ).toBe(true);
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
      .items.filter(item => item.kind === "message" && item.role === "assistant");
    expect(assistantItems).toHaveLength(1);
    expect(assistantItems[0]).toMatchObject({ runId: run, streaming: false });
  });

  test("snake_case run_id replay still normalizes (older desktops)", async () => {
    const run = nextRunId();
    const h = new Harness(run);
    h.snakeCaseReplay = true;
    // Legacy replay events carry snake_case run_id; normalizeReplayEvents must
    // keep accepting them.
    h.journal.add(agentStart(run, 0));
    h.journal.add(textChunk(run, 1, "legacy"));
    h.journal.add(agentEnd(run, 2));
    h.engine.event("s1", textChunk(run, 1, "legacy")); // first contact, mid-run
    await h.settle();
    expect(h.textOf("s1")).toBe("legacy");
    expect(h.timelineOf("s1").streaming).toBe(false);
  });

  test("open reconcile on an idle session loads durable history (no active run)", async () => {
    const h = new Harness(); // no active run, no live events
    h.history = {
      ...emptyTimeline(),
      items: [
        { id: "h1", kind: "message", role: "user", text: "Hello" },
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
      .items.filter(item => item.kind === "message" && item.role === "user");
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

  test("a run settling in a batch triggers an internal snapshot-flip reconcile (M11)", async () => {
    const run = nextRunId();
    const h = new Harness(run);
    h.journal.add(agentStart(run, 0));
    h.journal.add(textChunk(run, 1, "reply"));

    h.engine.event("s1", agentStart(run, 0));
    h.engine.event("s1", textChunk(run, 1, "reply"));
    await h.settle();
    expect(h.timelineOf("s1").streaming).toBe(true);

    h.journal.add(agentEnd(run, 2));
    h.engine.event("s1", agentEnd(run, 2));
    await h.settle();
    expect(h.timelineOf("s1").streaming).toBe(false);
    expect(h.textOf("s1")).toBe("reply");
  });

  test("projection replay without an explicit cursor derives it from event idx", async () => {
    const run = nextRunId();
    const h = new Harness(run);
    h.omitProjectionCursor = true;
    h.projection = [agentStart(run, 0), textChunk(run, 1, "folded"), agentEnd(run, 2)];
    h.engine.reconcile("s1", "resend", run);
    await h.settle();
    expect(h.textOf("s1")).toBe("folded");
    expect(h.timelineOf("s1").streaming).toBe(false);
  });

  test("full reconcile drops a live user mirror duplicating a durable prompt", async () => {
    const h = new Harness();
    h.history = {
      ...emptyTimeline(),
      items: [
        { id: "h1", kind: "message", role: "user", text: "Hello" },
        { id: "h2", kind: "message", role: "assistant", text: "Hi" },
      ],
    };
    h.engine.reconcile("s1", "open");
    await h.settle();
    expect(h.timelineOf("s1").items.map(i => i.id)).toEqual(["h1", "h2"]);

    h.engine.mutate("s1", tl => ({
      ...tl,
      items: [
        ...tl.items,
        { id: "local-dup", kind: "message", role: "user", text: "Hello" },
        { id: "notice-1", kind: "notice", tone: "neutral", text: "kept" },
      ],
    }));
    await h.settle();

    h.engine.reconcile("s1", "resend");
    await h.settle();

    expect(h.timelineOf("s1").items.map(i => i.id)).toEqual(["h1", "h2", "notice-1"]);
  });
});
