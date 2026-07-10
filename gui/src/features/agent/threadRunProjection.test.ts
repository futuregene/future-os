import type { StoredRun, StoredRunEvent } from "../../integrations/storage/threadStore";
import type { AgentMessage } from "./agentThreadTypes";
import { describe, expect, it } from "vitest";
import { deriveRenderFields, patchMessage, runDurationMs } from "./threadRunProjection";

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
