// @vitest-environment jsdom
import type { RunReplay } from "@future-os/thread-projection";
import type { RemoteRunEvent } from "./remoteRunStreams";
import { beforeEach, describe, expect, it } from "vitest";
import { RemoteRunStreams } from "./remoteRunStreams";

/**
 * The replies a remote conversation draws from its runs' logs.
 *
 * What is asserted here is what a reader sees on another machine's conversation
 * while the host is answering — the text as it arrives, the tool rows that resolve
 * in place, the run's own clock — and the two ways a reply becomes a row: rebuilt
 * from the run's log when this client has read it, folded into the record's copy
 * when it has only seen pushes. The projection itself is the shared projector's
 * contract and is tested there; this is about which lane fed it, what that makes
 * the client able to prove, and the bubble that comes back.
 */

let sequence = 0;

// Each test's frames are numbered from zero, so "contiguous" means what it says
// and a cursor assertion can name an index.
beforeEach(() => {
  sequence = 0;
});

/** One pushed event, as the host's body spells it: numbered from zero. */
function event(runId: string, type: string, data: unknown, overrides: Partial<RemoteRunEvent> = {}): RemoteRunEvent {
  const idx = sequence;
  sequence += 1;
  return {
    data: JSON.stringify(data),
    eventId: `${runId}:${idx}`,
    idx,
    runId,
    timestamp: new Date(1_700_000_000_000 + idx * 1_000).toISOString(),
    type,
    ...overrides,
  };
}

/** A run's log as a read returned it: a tail of raw deltas. */
function tail(runId: string, frames: [string, unknown][], options: Partial<RunReplay> = {}): RunReplay {
  return {
    events: frames.map(([type, data], index) => ({
      type,
      data: JSON.stringify(data),
      runId,
      idx: index,
    })),
    completePrefix: true,
    ...options,
  };
}

/** A run's log as a whole-run read returned it: one folded projection. */
function projection(runId: string, frames: [string, unknown][], cursor = frames.length - 1): RunReplay {
  return {
    events: [],
    watermark: cursor,
    projection: {
      runId,
      cursor,
      events: frames.map(([type, data], index) => ({
        type,
        data: JSON.stringify(data),
        runId,
        idx: index,
      })),
    },
    completePrefix: true,
  };
}

describe("remoteRunStreams", () => {
  it("grows the reply as pushed text arrives", () => {
    const streams = new RemoteRunStreams();

    expect(streams.apply(event("r1", "agent_start", {}))).toBe("applied");
    expect(streams.replies()).toEqual([]);
    streams.apply(event("r1", "text_chunk", { text: "Hel" }));
    const [first] = streams.replies();
    expect(first!.message.content).toBe("Hel");
    expect(first!.message.role).toBe("assistant");
    expect(first!.message.status).toBe("streaming");
    expect(first!.message.runId).toBe("r1");
    // Nothing has been read from the run's log, so the record's own copy of this
    // run is still the beginning of the reply: the view folds the two.
    expect(first!.fromReplay).toBe(false);

    streams.apply(event("r1", "text_chunk", { text: "lo" }));
    const [second] = streams.replies();
    expect(second!.message.content).toBe("Hello");
    // The same bubble, because it is the same run: a new id would unmount and
    // remount the row on every delta, which is what makes a transcript flicker.
    expect(second!.message.id).toBe(first!.message.id);
    expect(first!.message.id).toBe("stream_r1");
  });

  /**
   * The bubble carries what this app's own live reply carries.
   *
   * The transcript is the same component, so a field left off here is a
   * difference the reader sees: no inline tool row, no elapsed timer, no
   * reasoning block.
   */
  it("carries the fields the local live bubble carries", () => {
    const streams = new RemoteRunStreams();
    streams.apply(event("r1", "agent_start", {}));
    streams.apply(event("r1", "thinking_start", {}));
    streams.apply(event("r1", "thinking_delta", { text: "let me look" }));
    streams.apply(event("r1", "tool_start", {
      tool_id: "call_1",
      tool_name: "read",
      tool_args: { path: "analysis/effect-size.R" },
    }));
    streams.apply(event("r1", "text_chunk", { text: "reading it" }));

    const [reply] = streams.replies();
    const message = reply!.message;
    expect(message.authorKey).toBe("author.researchCopilot");
    expect(message.status).toBe("streaming");
    expect(message.thinkingActive).toBe(false);
    expect(message.segments?.map(segment => segment.kind)).toEqual(["thinking", "activity", "text"]);
    expect(message.createdAt).toMatch(/^\d{4}-\d{2}-\d{2}T/);
  });

  /**
   * A run that has produced nothing renderable is not drawn.
   *
   * `agent_start` is the run's first event and carries no content: an empty
   * bubble with an author line reads as a broken reply. A reasoning row *is*
   * renderable — it is the only indication that the model is working — and it
   * arrives with `thinkingActive`, which is what the footer's hint reads.
   */
  it("draws nothing until the run has a row to show", () => {
    const streams = new RemoteRunStreams();
    streams.apply(event("r1", "agent_start", {}));
    expect(streams.replies()).toEqual([]);
    streams.apply(event("r1", "thinking_start", {}));
    const [reply] = streams.replies();
    expect(reply!.message.segments?.map(segment => segment.kind)).toEqual(["thinking"]);
    expect(reply!.message.thinkingActive).toBe(true);
    // Both frames were applied: nothing to draw is not the same as nothing seen.
    expect(streams.cursorFor("r1")?.highWater).toBe(1);
  });

  /**
   * The run's own clock, from the log.
   *
   * A client that joins a reply already in flight receives no `agent_start` on
   * the push lane — it is the run's first event — so its timer can only count
   * from its own arrival. The log carries the start the run itself reported, and
   * reading it is the only way the elapsed time a reader sees is the run's.
   */
  it("times the reply from the run's own start once its log has been read", () => {
    const startedAt = 1_700_000_000_000 - 8 * 60_000;
    const streams = new RemoteRunStreams();
    // Pushes alone: the clock starts when this window first heard of the run.
    streams.apply(event("r1", "text_chunk", { text: "half a minute in" }));
    const before = streams.replies()[0]!.message.runStartedAt!;
    expect(before).toBeGreaterThan(startedAt);

    streams.restore("r1", tail("r1", [
      ["agent_start", { started_at_ms: startedAt }],
      ["text_chunk", { text: "half a minute in" }],
    ]));

    const [reply] = streams.replies();
    expect(reply!.fromReplay).toBe(true);
    expect(reply!.message.runStartedAt).toBe(startedAt);
  });

  /**
   * A rebuilt run is authoritative: the transcript's copy of it is hidden.
   *
   * This is what a conversation opened mid-run needs — the page holds a partial
   * copy of the reply and the pushes hold the rest, and the log holds all of it.
   */
  it("rebuilds a run from its log and marks it authoritative", () => {
    const streams = new RemoteRunStreams();
    streams.restore("r1", projection("r1", [
      ["agent_start", { started_at_ms: 1_700_000_000_000 }],
      ["text_chunk", { text: "the whole " }],
      ["text_chunk", { text: "reply" }],
    ]));

    const [reply] = streams.replies();
    expect(reply!.fromReplay).toBe(true);
    expect(reply!.message.content).toBe("the whole reply");
    expect([...streams.replayedRuns()]).toEqual(["r1"]);
    // The cursor covers the run from its beginning, which is what makes a later
    // read a tail.
    expect(streams.cursorFor("r1")).toEqual({ highWater: 2, prefixComplete: true });
  });

  /** A rebuild replaces the projection: the same events are not counted twice. */
  it("does not fold the log on top of what the pushes had built", () => {
    const streams = new RemoteRunStreams();
    streams.apply(event("r1", "text_chunk", { text: "pushed" }));
    streams.restore("r1", projection("r1", [
      ["agent_start", { started_at_ms: 1_700_000_000_000 }],
      ["text_chunk", { text: "the " }],
      ["text_chunk", { text: "log" }],
    ]));
    expect(streams.replies()[0]!.message.content).toBe("the log");
  });

  /** Pushes after a rebuild continue it in place, without a second row. */
  it("continues a rebuilt reply with the frames that follow it", () => {
    const streams = new RemoteRunStreams();
    streams.restore("r1", tail("r1", [
      ["agent_start", { started_at_ms: 1_700_000_000_000 }],
      ["text_chunk", { text: "one " }],
    ]));
    streams.apply(event("r1", "text_chunk", { text: "two" }, { idx: 2 }));
    const [reply] = streams.replies();
    expect(reply!.message.content).toBe("one two");
    expect(streams.replies()).toHaveLength(1);
  });

  /**
   * A jump in the indices, read two ways.
   *
   * A host that trims its feed omits whole event families, so a jump is expected
   * — and there is nothing to recover, which is why the caller says so. A host
   * that does not trim sends one event per index, so the same jump is frames this
   * client is missing, and the caller reads the run's log.
   */
  it("reports a jump as loss unless the caller says the feed omits indices", () => {
    const streams = new RemoteRunStreams();
    streams.apply(event("r1", "text_chunk", { text: "one" }, { idx: 0 }));
    expect(streams.apply(event("r1", "text_chunk", { text: "three" }, { idx: 2 }))).toBe("gap");
    // The frame was not applied: a gap means the client cannot order what it has.
    expect(streams.replies()[0]!.message.content).toBe("one");
    expect(streams.cursorFor("r1")?.highWater).toBe(0);

    expect(streams.apply(event("r1", "text_chunk", { text: "three" }, { idx: 2 }), { omitIndices: true }))
      .toBe("applied");
    expect(streams.replies()[0]!.message.content).toBe("onethree");
    expect(streams.cursorFor("r1")?.highWater).toBe(2);
  });

  /** An event the projector has already folded is not folded again. */
  it("drops a re-delivered frame instead of drawing it twice", () => {
    const streams = new RemoteRunStreams();
    streams.apply(event("r1", "text_chunk", { text: "once" }, { idx: 0 }));
    expect(streams.apply(event("r1", "text_chunk", { text: "once" }, { idx: 0 }))).toBe("duplicate");
    expect(streams.replies()[0]!.message.content).toBe("once");
  });

  /**
   * A settled run keeps its projection until the host's record can be read.
   *
   * The record trails the terminal frame — the agent writes the last model call
   * as the run ends — so a client that dropped its copy here would blink the reply
   * out of the transcript. It is drawn as a settled reply, with the run's own
   * duration rather than a live timer.
   */
  it("draws a settled reply from its log until the record is readable", () => {
    const streams = new RemoteRunStreams();
    streams.restore("r1", projection("r1", [
      ["agent_start", { started_at_ms: 1_700_000_000_000 }],
      ["text_chunk", { text: "the answer" }],
      ["agent_end", { duration_ms: 42_000 }],
    ]));

    streams.settle("r1");

    const [reply] = streams.replies();
    expect(reply!.message.status).toBe("complete");
    expect(reply!.message.durationMs).toBe(42_000);
    // Still drawn, and still standing in for the record's copy of that run.
    expect([...streams.replayedRuns()]).toEqual(["r1"]);
    expect(streams.knownRuns()).toEqual(["r1"]);

    // The record arrived: the projection stops being a source of rows, but keeps
    // folding the frames a run can still emit after its terminal one.
    streams.markRecorded("r1");
    expect(streams.replies()).toEqual([]);
    expect(streams.replayedRuns().size).toBe(0);
    expect(streams.knownRuns()).toEqual([]);
    streams.apply(event("r1", "usage", { output_tokens: 12 }, { idx: 3 }));
    expect(streams.replies()).toEqual([]);
    // Its cursor still says how far the log was read, so a later read is a tail.
    expect(streams.cursorFor("r1")?.highWater).toBe(3);
  });

  /**
   * A reply built from pushes alone is dropped when the run ends.
   *
   * That copy is a *fragment*: the reply's beginning is in the record, not in it.
   * The record holds the whole reply once the run is over, so keeping this one
   * would draw the same words twice — once from each lane.
   */
  it("drops a push-only reply when the run ends", () => {
    const streams = new RemoteRunStreams();
    streams.apply(event("r1", "text_chunk", { text: "the tail of it" }));
    expect(streams.replies()).toHaveLength(1);

    streams.settle("r1");

    expect(streams.replies()).toEqual([]);
    expect(streams.replayedRuns().size).toBe(0);
    // Its cursor stays: the next read of that run is still a tail from where the
    // pushes got to.
    expect(streams.cursorFor("r1")?.highWater).toBe(0);
  });

  /** A settled reply's duration falls back to the clock this lane was holding. */
  it("falls back to the watched clock when the log carries no duration", () => {
    const startedAt = Date.now() - 30_000;
    const streams = new RemoteRunStreams();
    streams.restore("r1", projection("r1", [
      ["agent_start", { started_at_ms: startedAt }],
      ["text_chunk", { text: "no duration here" }],
    ]));
    streams.settle("r1");
    // The run reported no duration, so the reply's own start against now is the
    // elapsed time — which is what the reader was watching on the live timer.
    const elapsed = streams.replies()[0]!.message.durationMs!;
    expect(elapsed).toBeGreaterThanOrEqual(30_000);
    expect(elapsed).toBeLessThan(60_000);
  });

  /** Two runs are two rows, in the order the reader met them. */
  it("keeps a settled reply while the next run is being written", () => {
    const streams = new RemoteRunStreams();
    streams.restore("r1", projection("r1", [["agent_start", {}], ["text_chunk", { text: "first" }]]));
    streams.settle("r1");
    streams.restore("r2", projection("r2", [["agent_start", {}], ["text_chunk", { text: "second" }]]));

    const replies = streams.replies();
    expect(replies.map(reply => reply.message.content)).toEqual(["first", "second"]);
    expect(replies.every(reply => reply.fromReplay)).toBe(true);
  });

  /**
   * A link that came back: what the pushes built is no longer evidence.
   *
   * A projection rebuilt from a run's *log* is a read, and a read does not go
   * stale because a socket did — it stays, with its cursor.
   */
  it("restarts the push lane without discarding what was read from a log", () => {
    const streams = new RemoteRunStreams();
    streams.restore("r1", projection("r1", [["agent_start", {}], ["text_chunk", { text: "from the log" }]]));
    streams.apply(event("r2", "text_chunk", { text: "from pushes" }, { idx: 0 }));

    streams.restart();

    const replies = streams.replies();
    expect(replies.map(reply => [reply.message.content, reply.fromReplay]))
      .toEqual([["from the log", true]]);
    expect(streams.cursorFor("r1")).toEqual({ highWater: 1, prefixComplete: true });
  });

  /** Another conversation is opened: nothing here belongs to it, cursors included. */
  it("forgets everything when another conversation is opened", () => {
    const streams = new RemoteRunStreams();
    streams.restore("r1", projection("r1", [["agent_start", {}], ["text_chunk", { text: "theirs" }]]));
    streams.clear();
    expect(streams.replies()).toEqual([]);
    expect(streams.cursorFor("r1")).toBeUndefined();
  });

  /**
   * Projections are bounded, because deltas are gone once applied; cursors are
   * bounded separately, and outliving their projection is the point.
   *
   * A conversation that has answered many prompts would otherwise keep one
   * accumulator per run for the life of the window, and none of them would ever be
   * read again — the settled reply comes from the host's record. The cursor is a
   * different fact (how far the log was read), so eviction must not take it: it is
   * what makes the next read of that run a tail.
   */
  it("bounds how many runs it keeps, while keeping their cursors", () => {
    const streams = new RemoteRunStreams();
    for (let index = 0; index < 10; index += 1)
      streams.apply(event(`r${index}`, "text_chunk", { text: `reply ${index}` }, { idx: 0 }));

    expect(streams.replies().map(reply => reply.message.runId)).toEqual(["r6", "r7", "r8", "r9"]);
    // Projections are capped at four; cursors at eight — so `r2`'s projection is
    // gone while its position in the log is still known.
    expect(streams.cursorFor("r2")?.highWater).toBe(0);
  });
});
