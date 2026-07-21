import type { StoredRun, StoredRunEvent } from "../../integrations/storage/threadStore";
import type { AgentMessage } from "./agentThreadTypes";
import { describe, expect, it } from "vitest";
import { applyRecoveredEvents, applyRunMetadata, deriveRenderFields, patchMessage, runDurationMs, streamingBubbleBase } from "./threadRunProjection";

function message(id: string, patch: Partial<AgentMessage> = {}): AgentMessage {
  return {
    id,
    role: "assistant",
    authorKey: "author.researchCopilot",
    content: "",
    createdAt: "2026-01-01T00:00:00.000Z",
    ...patch,
  };
}

function events(list: Array<[string, Record<string, unknown>]>): StoredRunEvent[] {
  return list.map(([eventType, payload], index) => ({
    id: `e${index}`,
    runId: "r1",
    eventType,
    payload: JSON.stringify(payload),
    sequence: index,
    createdAt: index,
  }));
}

function applyPatch(
  state: AgentMessage[],
  ...args: Parameters<typeof patchMessage> extends [unknown, ...infer Rest] ? Rest : never
): AgentMessage[] {
  let next = state;
  patchMessage((action) => {
    next = typeof action === "function" ? action(next) : action;
  }, ...args);
  return next;
}

describe("patchMessage", () => {
  it("patches only the matching message and leaves others untouched", () => {
    const state = applyPatch(
      [message("a", { content: "one" }), message("b", { content: "two" })],
      "b",
      { content: "patched" },
    );
    expect(state.map(m => m.content)).toEqual(["one", "patched"]);
  });

  it("supports a functional patch derived from the current message", () => {
    const state = applyPatch(
      [message("a", { content: "x", outputTokens: 5 })],
      "a",
      prev => ({ outputTokens: (prev.outputTokens ?? 0) + 1 }),
    );
    expect(state[0]?.outputTokens).toBe(6);
  });

  it("is a no-op when no id matches", () => {
    const state = applyPatch([message("a", { content: "x" })], "missing", { content: "y" });
    expect(state[0]?.content).toBe("x");
  });
});

describe("runDurationMs", () => {
  it("uses persisted start/end when both present and ordered", () => {
    const run = { startedAt: 1000, endedAt: 3500 } as StoredRun;
    expect(runDurationMs(run)).toBe(2500);
  });

  it("ignores an inverted end/start and falls back to null without a fallback anchor", () => {
    const run = { startedAt: 3000, endedAt: 1000 } as StoredRun;
    expect(runDurationMs(run)).toBeNull();
  });

  it("returns null when nothing is known", () => {
    expect(runDurationMs(null)).toBeNull();
    expect(runDurationMs(undefined)).toBeNull();
  });

  it("falls back to elapsed-since-anchor while the run is still settling", () => {
    expect(runDurationMs(null, Date.now())).toBeGreaterThanOrEqual(0);
  });
});

function user(id: string, patch: Partial<AgentMessage> = {}): AgentMessage {
  return message(id, { role: "user", authorKey: "author.you", status: "complete", ...patch });
}

function assistant(id: string, patch: Partial<AgentMessage> = {}): AgentMessage {
  return message(id, { status: "complete", ...patch });
}

function run(id: string, patch: Partial<StoredRun> = {}): StoredRun {
  return {
    id,
    threadId: "t1",
    status: "completed",
    createdAt: 0,
    updatedAt: 0,
    ...patch,
  } as StoredRun;
}

describe("applyRunMetadata", () => {
  it("marks the most recent turn failed when its run failed", () => {
    const messages = [
      user("u1"),
      assistant("a1"),
      user("u2"),
      assistant("a2"),
    ];
    // Newest run first (created_at DESC): a2 ↔ r2 (failed), a1 ↔ r1.
    const result = applyRunMetadata(messages, [
      run("r2", { status: "failed", modelId: "m-2" }),
      run("r1", { status: "completed", modelId: "m-1" }),
    ]);
    expect(result[3]).toMatchObject({ id: "a2", runId: "r2", status: "failed", modelId: "m-2", stopped: false });
    expect(result[1]).toMatchObject({ id: "a1", runId: "r1", status: "complete", modelId: "m-1", stopped: false });
  });

  it("marks a cancelled run's turn as stopped without failing it", () => {
    const result = applyRunMetadata([user("u1"), assistant("a1")], [run("r1", { status: "cancelled" })]);
    expect(result[1]).toMatchObject({ id: "a1", runId: "r1", status: "complete", stopped: true });
  });

  it("aligns from the newest end and ignores extra older runs", () => {
    // One turn, two runs: only the newest run pairs with the turn.
    const result = applyRunMetadata([user("u1"), assistant("a1")], [
      run("r-new", { status: "failed" }),
      run("r-old", { status: "completed" }),
    ]);
    expect(result[1]).toMatchObject({ id: "a1", runId: "r-new", status: "failed" });
  });

  it("leaves older turns untouched when there are fewer runs than turns", () => {
    const result = applyRunMetadata([
      user("u1"),
      assistant("a1"),
      user("u2"),
      assistant("a2"),
    ], [run("r2", { status: "failed" })]);
    // Newest turn pairs with the only run; the older turn keeps its defaults.
    expect(result[3]).toMatchObject({ id: "a2", runId: "r2", status: "failed" });
    expect(result[1]?.runId).toBeUndefined();
    expect(result[1]?.status).toBe("complete");
  });

  it("does not consume a run slot for a compaction divider", () => {
    const divider = assistant("div", { content: "", segments: [{ id: "s", kind: "compaction" }] });
    const result = applyRunMetadata([
      user("u1"),
      assistant("a1"),
      divider,
      user("u2"),
      assistant("a2"),
    ], [
      run("r2", { status: "failed" }),
      run("r1", { status: "completed" }),
    ]);
    expect(result[4]).toMatchObject({ id: "a2", runId: "r2", status: "failed" });
    expect(result[1]).toMatchObject({ id: "a1", runId: "r1" });
    // The divider stays a plain complete marker with no run attached.
    expect(result[2]?.runId).toBeUndefined();
    expect(result[2]?.status).toBe("complete");
  });

  it("stamps an aborted (empty) turn with the run's end time — the stop time", () => {
    const stopMs = Date.parse("2026-07-01T10:00:06.000Z");
    const result = applyRunMetadata(
      [user("u1", { createdAt: "2026-07-01T10:00:00.000Z" }), assistant("a1", { content: "" })],
      [run("r1", { status: "cancelled", startedAt: stopMs - 6000, endedAt: stopMs })],
    );
    expect(result[1]?.createdAt).toBe(new Date(stopMs).toISOString());
    expect(result[1]?.stopped).toBe(true);
  });

  it("keeps a completed turn's own reply time rather than restamping it", () => {
    const replyTs = "2026-07-01T10:00:07.000Z";
    const result = applyRunMetadata(
      [user("u1"), assistant("a1", { content: "answer", createdAt: replyTs })],
      [run("r1", { status: "completed", endedAt: 999 })],
    );
    expect(result[1]?.createdAt).toBe(replyTs);
  });

  it("returns messages unchanged when there are no runs", () => {
    const messages = [user("u1"), assistant("a1")];
    expect(applyRunMetadata(messages, [])).toBe(messages);
  });

  it("keeps an existing agent-recorded durationMs over the run's wall-clock", () => {
    const result = applyRunMetadata(
      [user("u1"), assistant("a1", { durationMs: 1234 })],
      [run("r1", { startedAt: 1000, endedAt: 9000 })],
    );
    expect(result[1]?.durationMs).toBe(1234);
  });
});

describe("applyRecoveredEvents", () => {
  it("fills an empty aborted turn with the streamed partial text", () => {
    const messages = [
      user("u1"),
      assistant("a1", { content: "", runId: "r1", stopped: true }),
    ];
    const result = applyRecoveredEvents(
      messages,
      new Map([["r1", events([["text_chunk", { text: "half a poem" }]])]]),
    );
    expect(result[1]?.content).toBe("half a poem");
    expect(result[1]?.segments).toBeDefined();
    // Recovery doesn't touch the stopped marker the run metadata set.
    expect(result[1]?.stopped).toBe(true);
  });

  it("leaves a turn that already has content untouched", () => {
    const messages = [user("u1"), assistant("a1", { content: "final answer", runId: "r1" })];
    const result = applyRecoveredEvents(
      messages,
      new Map([["r1", events([["text_chunk", { text: "something else" }]])]]),
    );
    expect(result[1]?.content).toBe("final answer");
  });

  it("leaves a turn with segments untouched (tool activity already projected)", () => {
    const withSegments = assistant("a1", {
      content: "",
      runId: "r1",
      segments: [{ id: "s", kind: "text", text: "kept" }],
    });
    const result = applyRecoveredEvents(
      [user("u1"), withSegments],
      new Map([["r1", events([["text_chunk", { text: "ignored" }]])]]),
    );
    expect(result[1]?.segments).toEqual([{ id: "s", kind: "text", text: "kept" }]);
    expect(result[1]?.content).toBe("");
  });

  it("leaves an empty turn untouched when its events carried no text", () => {
    const messages = [user("u1"), assistant("a1", { content: "", runId: "r1" })];
    const result = applyRecoveredEvents(messages, new Map([["r1", events([])]]));
    expect(result[1]?.content).toBe("");
  });

  it("ignores turns without a runId", () => {
    const messages = [user("u1"), assistant("a1", { content: "" })];
    const result = applyRecoveredEvents(messages, new Map([["r1", events([["text_chunk", { text: "x" }]])]]));
    expect(result[1]?.content).toBe("");
  });
});

describe("deriveRenderFields", () => {
  it("prefers event-derived content and segments when the events carried text", () => {
    const result = deriveRenderFields(
      events([["text_chunk", { text: "Hello" }]]),
      "fallback",
    );
    expect(result.content).toBe("Hello");
    expect(result.segments).toBeDefined();
  });

  it("falls back to the stored reply when events carried no assistant text", () => {
    const result = deriveRenderFields(events([]), "stored reply");
    expect(result.content).toBe("stored reply");
    expect(result.segments).toBeUndefined();
  });
});

describe("streamingBubbleBase", () => {
  const RUN = "r1";
  const BUBBLE = `stream_${RUN}`;

  it("returns null when a settled run's persisted message already carries the runId", () => {
    const current = [user("u1"), assistant("a1", { runId: RUN, content: "done" })];
    expect(streamingBubbleBase(current, RUN, BUBBLE, "done")).toBeNull();
  });

  it("ignores the bubble itself when checking the runId guard", () => {
    const current = [user("u1"), assistant(BUBBLE, { runId: RUN, content: "live" })];
    const base = streamingBubbleBase(current, RUN, BUBBLE, "live");
    expect(base).toBe(current);
  });

  it("drops the mid-run persisted entry that duplicates the live projection (short snapshot)", () => {
    // The reported bug: persisted "Hello wor" (< 80 chars) failed the old
    // includes(content[:80]) guard, so the bubble was inserted alongside it.
    const persisted = assistant("a-partial", { content: "Hello wor" });
    const current = [user("u1"), assistant("a1", { content: "earlier reply" }), user("u2"), persisted];
    const base = streamingBubbleBase(current, RUN, BUBBLE, "Hello world, how are you?");
    expect(base?.some(m => m.id === "a-partial")).toBe(false);
    expect(base?.some(m => m.id === "a1")).toBe(true);
    expect(base?.some(m => m.id === "u2")).toBe(true);
  });

  it("drops the persisted entry of a multi-call turn (finalText is the last call's text)", () => {
    // Two LLM calls persisted separately; entriesToMessages keeps only the last
    // call's text as content, which is a substring (not prefix) of the live projection.
    const persisted = assistant("a-partial", { content: "second call text" });
    const current = [user("u1"), persisted];
    const base = streamingBubbleBase(current, RUN, BUBBLE, "first call text second call text and more");
    expect(base?.some(m => m.id === "a-partial")).toBe(false);
  });

  it("keeps an earlier turn's reply even when the new stream starts alike", () => {
    const earlier = assistant("a1", { content: "OK" });
    const current = [user("u1"), earlier, user("u2")];
    const base = streamingBubbleBase(current, RUN, BUBBLE, "OK, let me help with that");
    expect(base?.some(m => m.id === "a1")).toBe(true);
  });

  it("returns null when another turn's persisted reply already covers the live text", () => {
    const earlier = assistant("a1", { content: "Hello world, how are you today?" });
    const current = [user("u1"), earlier, user("u2")];
    // u2's turn has no persisted entry; u1's reply happens to contain the head.
    expect(streamingBubbleBase(current, RUN, BUBBLE, "Hello world, how")).toBeNull();
  });

  it("returns the list unchanged when the in-flight turn has no persisted entry", () => {
    const current = [user("u1"), assistant("a1", { content: "previous reply" }), user("u2")];
    const base = streamingBubbleBase(current, RUN, BUBBLE, "brand new stream");
    expect(base).toBe(current);
  });

  it("returns the list unchanged when live content is empty (thinking-only so far)", () => {
    const persisted = assistant("a-partial", { content: "partial text" });
    const current = [user("u1"), persisted];
    const base = streamingBubbleBase(current, RUN, BUBBLE, "");
    expect(base).toBe(current);
  });
});
