// @vitest-environment jsdom
import type { RemoteRunEvent } from "./remoteRunStreams";
import { describe, expect, it } from "vitest";
import { RemoteRunStreams } from "./remoteRunStreams";

/**
 * The live reply of a remote run.
 *
 * What is asserted here is what a reader sees on another machine's conversation
 * while the host is answering: the text as it arrives, the tool rows that resolve
 * in place, and the moment the live reply steps aside for the record. The
 * projection itself is the shared projector's contract and is tested there — this
 * is about the events reaching it and the bubble coming back.
 */

let sequence = 0;

/** One pushed event, as the host's body spells it. */
function event(runId: string, type: string, data: unknown, overrides: Partial<RemoteRunEvent> = {}): RemoteRunEvent {
  sequence += 1;
  return {
    data: JSON.stringify(data),
    eventId: `${runId}:${sequence}`,
    idx: sequence,
    runId,
    timestamp: new Date(1_700_000_000_000 + sequence * 1_000).toISOString(),
    type,
    ...overrides,
  };
}

describe("remoteRunStreams", () => {
  it("grows the reply as its text arrives", () => {
    const streams = new RemoteRunStreams();

    expect(streams.apply(event("r1", "agent_start", {}))).toBeNull();
    const first = streams.apply(event("r1", "text_chunk", { text: "Hel" }))!;
    expect(first.content).toBe("Hel");
    expect(first.role).toBe("assistant");
    expect(first.status).toBe("streaming");
    expect(first.runId).toBe("r1");

    const second = streams.apply(event("r1", "text_chunk", { text: "lo" }))!;
    expect(second.content).toBe("Hello");
    // The same bubble, because it is the same run: a new id would unmount and
    // remount the row on every delta, which is what makes a transcript flicker.
    expect(second.id).toBe(first.id);
    expect(first.id).toBe("stream_r1");
  });

  /**
   * The bubble carries what this app's own live reply carries.
   *
   * The transcript is the same component, so a field left off here is a
   * difference the reader sees: no inline tool row, no elapsed timer, no
   * reasoning block. Asserted as a shape rather than a list of fields the fixture
   * happens to set.
   */
  it("carries the fields the local live bubble carries", () => {
    const streams = new RemoteRunStreams();
    streams.apply(event("r1", "thinking_start", {}));
    // Reasoning with nothing visible yet: the footer's "thinking…" hint, which is
    // the state the local transcript shows while a model is still reasoning.
    const reasoning = streams.apply(event("r1", "thinking_delta", { text: "thinking about it" }))!;
    expect(reasoning.thinkingActive).toBe(true);
    expect(JSON.stringify(reasoning.segments)).toContain("thinking about it");

    const bubble = streams.apply(event("r1", "text_chunk", { text: "answer" }))!;

    expect(bubble.authorKey).toBe("author.researchCopilot");
    expect(bubble.status).toBe("streaming");
    expect(bubble.segments?.length).toBeGreaterThan(0);
    expect(bubble.activityItems).toEqual([]);
    // The answer is on screen now, so the hint has nothing to say.
    expect(bubble.thinkingActive).toBe(false);
    expect(bubble.content).toBe("answer");
    expect(bubble.runStartedAt).toBeGreaterThan(0);
    expect(bubble.createdAt).toBeTruthy();
  });

  /**
   * A turn's tool work is the part a reader waits for, and it resolves in place.
   *
   * The host streams a call's start and its end as separate events; the row has to
   * move from "running" to its outcome in the same slot, because that is what the
   * local transcript does with the same events.
   */
  it("resolves a tool call in place", () => {
    const streams = new RemoteRunStreams();
    const started = streams.apply(event("r1", "tool_start", {
      tool_id: "call_1",
      tool_name: "shell",
      tool_args: { command: "ls -la" },
    }))!;
    expect(started.activityItems?.[0]?.status).toBe("running");
    expect(started.activityItems?.[0]?.target).toContain("ls -la");

    const finished = streams.apply(event("r1", "tool_end", {
      tool_id: "call_1",
      tool_name: "shell",
      result: "total 8\n",
      exit_code: 0,
    }))!;
    expect(finished.activityItems).toHaveLength(1);
    expect(finished.activityItems?.[0]?.id).toBe("call_1");
    expect(finished.activityItems?.[0]?.status).toBe("completed");
  });

  /**
   * Nothing renderable means no bubble.
   *
   * A host sends `agent_start` (and often a `usage` frame) before it has any text
   * or work to show. An empty bubble with an author line above it reads as a
   * broken reply, so the transcript gets nothing until there is something to draw.
   */
  it("draws nothing until the run has something to show", () => {
    const streams = new RemoteRunStreams();
    expect(streams.apply(event("r1", "agent_start", { model: "m" }))).toBeNull();
    expect(streams.apply(event("r1", "usage", { output_tokens: 12 }))).toBeNull();
    expect(streams.apply(event("r1", "text_chunk", { text: "   " }))).toBeNull();
    expect(streams.apply(event("r1", "text_chunk", { text: "now" }))).not.toBeNull();
  });

  /**
   * A settled run's projection is dropped, so the next run starts from nothing.
   *
   * The projection is an accumulator: keeping it would prepend the finished
   * reply's text to the next one, which is a reply that never existed and cannot
   * be explained by anything on screen.
   */
  it("forgets a run's reply when it settles", () => {
    const streams = new RemoteRunStreams();
    streams.apply(event("r1", "text_chunk", { text: "first answer" }));
    streams.settle("r1");

    const next = streams.apply(event("r2", "text_chunk", { text: "second answer" }))!;
    expect(next.content).toBe("second answer");
    expect(next.id).toBe("stream_r2");
  });

  /** Another conversation's reply is not this one's, and neither is its history. */
  it("keeps each conversation's replies to itself once cleared", () => {
    const streams = new RemoteRunStreams();
    streams.apply(event("r1", "text_chunk", { text: "theirs" }));
    streams.clear();
    const next = streams.apply(event("r1", "text_chunk", { text: "ours" }))!;
    expect(next.content).toBe("ours");
  });

  /**
   * A page read during a run restarts the reply at it, keeping the run's clock.
   *
   * The reader's transcript now carries everything the pushes had built up to
   * that read, so the bubble starts over from the page — but the run did not
   * start again, and the elapsed time of a reply they are watching must not fall
   * back to zero because a link blipped.
   */
  it("restarts the projection at a page without restarting the run's clock", () => {
    const streams = new RemoteRunStreams();
    streams.apply(event("r1", "text_chunk", { text: "half " }));
    const startedAt = streams.apply(event("r1", "text_chunk", { text: "of it" }))!.runStartedAt;

    streams.restart();

    const after = streams.apply(event("r1", "text_chunk", { text: "the rest" }))!;
    expect(after.content).toBe("the rest");
    expect(after.runStartedAt).toBe(startedAt);
  });

  /**
   * Projectors are bounded, because deltas are gone once applied.
   *
   * A conversation that has answered many prompts would otherwise keep one
   * accumulator per run for the life of the window, and none of them would ever be
   * read again — the settled reply comes from the host's history.
   */
  it("bounds how many runs it keeps", () => {
    const streams = new RemoteRunStreams();
    for (let index = 0; index < 10; index += 1)
      streams.apply(event(`r${index}`, "text_chunk", { text: `reply ${index}` }));

    // The newest is the one in hand; an older run rebuilt from its next frame
    // starts with only what arrived since (nothing is carried over).
    const rebuilt = streams.apply(event("r0", "text_chunk", { text: "again" }))!;
    expect(rebuilt.content).toBe("again");
  });
});
