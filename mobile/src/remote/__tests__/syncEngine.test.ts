import { emptyTimeline } from "../eventReducer";
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
    return this.events.filter(e => e.runId === run && e.idx != null && from === -1 ? true : (e.idx ?? -1) > from);
  }
}

class Harness {
  journal = new Journal();
  activeRunId = "";
  history: ReturnType<typeof emptyTimeline> = emptyTimeline();
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
        // The replay path delivers the agent's snake_case wire shape; the
        // engine normalizes it back to StreamEvent.
        const events = this.journal
          .since(run, since)
          .map(e => ({ type: e.type, data: e.data, run_id: e.runId, idx: e.idx }));
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
    expect(h.timelineOf("s1").items.some(item => item.kind === "notice" && item.text === "sent")).toBe(
      true,
    );
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

    const userItems = h.timelineOf("s1").items.filter(
      item => item.kind === "message" && item.role === "user",
    );
    expect(userItems).toHaveLength(1);
    expect(userItems[0]).toMatchObject({ text: "hi there" });
  });
});
