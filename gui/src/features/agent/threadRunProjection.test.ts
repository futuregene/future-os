import type { StoredRun, StoredRunEvent } from "../../integrations/storage/threadStore";
import type { AgentMessage } from "./agentThreadTypes";
import { describe, expect, it } from "vitest";
import { applyRunMetadata, deriveRenderFields, patchMessage, runDurationMs } from "./threadRunProjection";

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
