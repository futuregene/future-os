import type { AgentMessage } from "@future-os/thread-projection";
import type { StoredRun, StoredRunEvent } from "../../integrations/storage/threadStore";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { emitFutureEvent } from "../../lib/futureEvents";
import {
  applyRecoveredEvents,
  applyRunMetadata,
  buildStreamingPreview,
  loadCurrentRun,
  mergeStreamingPreview,
  recoverAbortedTurns,
  safeListRunEvents,
  updatePendingMessageFromRunEvents,
  upsertStreamingPreview,
} from "./threadRunProjection";

const getRun = vi.fn();
const listRunEvents = vi.fn();
const listRunEventsSince = vi.fn();
const listRunEventsBulk = vi.fn();

vi.mock("../../integrations/storage/threadStore", () => ({
  getRun: (...args: unknown[]) => getRun(...args),
  listRunEvents: (...args: unknown[]) => listRunEvents(...args),
  listRunEventsSince: (...args: unknown[]) => listRunEventsSince(...args),
  listRunEventsBulk: (...args: unknown[]) => listRunEventsBulk(...args),
  storedTimeToIso: (value: number) => new Date(value).toISOString(),
}));

vi.mock("../../lib/futureEvents", () => ({
  emitFutureEvent: vi.fn(),
}));

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

function userMessage(id: string): AgentMessage {
  return { ...message(id), role: "user", authorKey: "author.you", content: id };
}

function runEvents(runId: string, list: Array<[string, Record<string, unknown>]>, offset = 0): StoredRunEvent[] {
  return list.map(([eventType, payload], index) => ({
    id: `${runId}_e${offset + index}`,
    runId,
    eventType,
    payload: JSON.stringify(payload),
    sequence: offset + index,
    createdAt: offset + index,
  }));
}

function textEvents(runId: string, text: string, offset = 0): StoredRunEvent[] {
  return runEvents(runId, [["text_chunk", { text }]], offset);
}

/** Drive a setMessages mock to a final state. */
function collect(initial: AgentMessage[]) {
  let state = initial;
  const setMessages = vi.fn((action: unknown) => {
    state = typeof action === "function" ? (action as (p: AgentMessage[]) => AgentMessage[])(state) : action as AgentMessage[];
  });
  return { setMessages, state: () => state };
}

beforeEach(() => {
  getRun.mockReset();
  listRunEvents.mockReset();
  listRunEventsSince.mockReset();
  listRunEventsBulk.mockReset();
});

describe("upsertStreamingPreview", () => {
  it("does nothing when the send is no longer current", async () => {
    listRunEventsSince.mockResolvedValue(textEvents("r-up1", "hello"));
    const { setMessages } = collect([]);
    await upsertStreamingPreview("r-up1", null, setMessages, () => false);
    expect(setMessages).not.toHaveBeenCalled();
  });

  it("does nothing when a persisted message already carries the run", async () => {
    listRunEventsSince.mockResolvedValue(textEvents("r-up2", "hello"));
    const persisted = message("m1", { runId: "r-up2", content: "final" });
    const { setMessages, state } = collect([userMessage("u1"), persisted]);
    await upsertStreamingPreview("r-up2", null, setMessages);
    // Base is null → the state updater returns the list unchanged.
    expect(state()[1]).toBe(persisted);
  });

  it("inserts a streaming bubble and updates it in place on the next push", async () => {
    listRunEventsSince.mockResolvedValue(textEvents("r-up3", "partial"));
    const { setMessages, state } = collect([userMessage("u1")]);
    await upsertStreamingPreview("r-up3", 1000, setMessages);
    const bubble = state().find(m => m.id === "stream_r-up3");
    expect(bubble).toMatchObject({ content: "partial", status: "streaming", runId: "r-up3" });

    // Second push: incremental fetch (since watermark) updates in place.
    listRunEventsSince.mockResolvedValue(textEvents("r-up3", " more", 1));
    await upsertStreamingPreview("r-up3", 1000, setMessages);
    const bubbles = state().filter(m => m.id === "stream_r-up3");
    expect(bubbles).toHaveLength(1);
    expect(bubbles[0]).toMatchObject({ content: "partial more" });
    expect(listRunEventsSince).toHaveBeenLastCalledWith("r-up3", 0);
  });

  it("rebuilds from the full log when the event sequence regresses", async () => {
    listRunEventsSince.mockResolvedValue(textEvents("r-up4", "one"));
    const { setMessages } = collect([userMessage("u1")]);
    await upsertStreamingPreview("r-up4", null, setMessages);
    // Regression: the tail starts at sequence 0 again.
    listRunEventsSince.mockResolvedValue(textEvents("r-up4", "restarted"));
    await upsertStreamingPreview("r-up4", null, setMessages);
    expect(listRunEventsSince).toHaveBeenLastCalledWith("r-up4", -1);
  });

  it("aborts the regression rebuild when the send goes stale mid-flight", async () => {
    listRunEventsSince.mockResolvedValue(textEvents("r-up5", "one"));
    const { setMessages } = collect([userMessage("u1")]);
    await upsertStreamingPreview("r-up5", null, setMessages);
    let calls = 0;
    listRunEventsSince.mockImplementation(() => {
      calls += 1;
      return Promise.resolve(textEvents("r-up5", "x"));
    });
    // shouldApply: true until the rebuild fetch starts, then false.
    let applies = 0;
    const shouldApply = () => {
      applies += 1;
      return applies <= 1;
    };
    await upsertStreamingPreview("r-up5", null, setMessages, shouldApply);
    expect(calls).toBe(2);
  });

  it("evicts the oldest projector past the cache cap", async () => {
    for (let i = 0; i < 10; i += 1) {
      listRunEventsSince.mockResolvedValue(textEvents(`r-lru-${i}`, "x"));
      const { setMessages } = collect([userMessage("u1")]);

      await upsertStreamingPreview(`r-lru-${i}`, null, setMessages);
    }
    // Re-projecting the oldest starts a fresh (full-log) fetch.
    listRunEventsSince.mockResolvedValue(textEvents("r-lru-0", "x"));
    const { setMessages } = collect([userMessage("u1")]);
    await upsertStreamingPreview("r-lru-0", null, setMessages);
    expect(listRunEventsSince).toHaveBeenLastCalledWith("r-lru-0", -1);
  });

  it("swallows projection failures", async () => {
    listRunEventsSince.mockRejectedValue(new Error("ipc"));
    const { setMessages } = collect([userMessage("u1")]);
    await expect(upsertStreamingPreview("r-err", null, setMessages)).resolves.toBeUndefined();
  });
});

describe("buildStreamingPreview", () => {
  it("returns null when nothing is renderable and a bubble otherwise", async () => {
    listRunEventsSince.mockResolvedValue([]);
    await expect(buildStreamingPreview("r-b1")).resolves.toBeNull();
    listRunEventsSince.mockResolvedValue(textEvents("r-b2", "content"));
    const bubble = await buildStreamingPreview("r-b2", 500);
    expect(bubble).toMatchObject({ id: "stream_r-b2", content: "content", status: "streaming" });
  });

  it("emits file-tree-refresh when tool activity appears", async () => {
    vi.mocked(emitFutureEvent).mockClear();
    listRunEventsSince.mockResolvedValue(runEvents("r-tool", [
      ["toolcall_start", { tool_name: "read", tool_args: { path: "/a.ts" } }],
    ]));
    await buildStreamingPreview("r-tool");
    expect(emitFutureEvent).toHaveBeenCalledWith("file-tree-refresh", undefined);
  });
});

describe("updatePendingMessageFromRunEvents", () => {
  it("returns early when the send is stale", async () => {
    listRunEventsSince.mockResolvedValue(textEvents("r-p1", "text"));
    const { setMessages } = collect([]);
    await updatePendingMessageFromRunEvents("r-p1", "pending", setMessages, () => false);
    expect(setMessages).not.toHaveBeenCalled();
  });

  it("returns early when nothing is renderable yet", async () => {
    listRunEventsSince.mockResolvedValue([]);
    const { setMessages } = collect([]);
    await updatePendingMessageFromRunEvents("r-p2", "pending", setMessages);
    expect(setMessages).not.toHaveBeenCalled();
  });

  it("updates the pending bubble in place and ignores a missing bubble", async () => {
    listRunEventsSince.mockResolvedValue(textEvents("r-p3", "streamed"));
    const pending = message("pending", { content: "", status: "streaming" });
    const { setMessages, state } = collect([userMessage("u1"), pending]);
    await updatePendingMessageFromRunEvents("r-p3", "pending", setMessages);
    expect(state()[1]).toMatchObject({ content: "streamed" });

    // A second run whose pending bubble is gone leaves the state alone.
    const { setMessages: set2, state: state2 } = collect([userMessage("u1")]);
    await updatePendingMessageFromRunEvents("r-p3", "missing", set2);
    expect(state2()).toHaveLength(1);
  });
});

describe("mergeStreamingPreview", () => {
  it("returns the list unchanged when the preview has no runId", () => {
    const current = [userMessage("u1")];
    expect(mergeStreamingPreview(current, message("p"))).toBe(current);
  });
});

describe("safeListRunEvents / loadCurrentRun failure paths", () => {
  it("safeListRunEvents returns [] on failure", async () => {
    listRunEvents.mockRejectedValue(new Error("down"));
    await expect(safeListRunEvents("r1")).resolves.toEqual([]);
  });

  it("loadCurrentRun returns null on failure", async () => {
    getRun.mockRejectedValue(new Error("down"));
    await expect(loadCurrentRun("r1")).resolves.toBeNull();
  });

  it("loadCurrentRun returns the run", async () => {
    getRun.mockResolvedValue({ id: "r1" } as StoredRun);
    await expect(loadCurrentRun("r1")).resolves.toMatchObject({ id: "r1" });
  });
});

describe("recoverAbortedTurns / applyRecoveredEvents", () => {
  it("returns the messages unchanged when there are no empty turns", async () => {
    const messages = [message("m1", { content: "full" })];
    await expect(recoverAbortedTurns(messages)).resolves.toBe(messages);
    expect(listRunEventsBulk).not.toHaveBeenCalled();
  });

  it("recovers content for empty turns from bulk events", async () => {
    const empty = message("m1", { runId: "r-rec", content: "" });
    listRunEventsBulk.mockResolvedValue([["r-rec", textEvents("r-rec", "recovered")]]);
    const result = await recoverAbortedTurns([empty]);
    expect(result[0]).toMatchObject({ content: "recovered" });
  });

  it("keeps the empty message when the projection has no content or segments", () => {
    const empty = message("m1", { runId: "r-empty", content: "" });
    const result = applyRecoveredEvents([empty], new Map([["r-empty", []]]));
    expect(result[0]).toBe(empty);
    // Events exist but project to nothing.
    const result2 = applyRecoveredEvents([empty], new Map([["r-empty", runEvents("r-empty", [["usage", { completion_tokens: 3 }]])]]));
    expect(result2[0]).toBe(empty);
  });

  it("returns the messages unchanged when the bulk fetch fails", async () => {
    const empty = message("m1", { runId: "r-fail", content: "" });
    listRunEventsBulk.mockRejectedValue(new Error("down"));
    await expect(recoverAbortedTurns([empty])).resolves.toEqual([empty]);
  });
});

describe("applyRunMetadata run lookup", () => {
  it("skips assistant messages whose run is not in the run list", () => {
    const otherRun = {
      id: "other-run",
      threadId: "t",
      status: "completed",
      createdAt: 1_000,
      updatedAt: 2_000,
      endedAt: 2_000,
    } as StoredRun;
    const messages = [userMessage("u1"), message("a1", { runId: "missing-run", content: "x" })];
    const result = applyRunMetadata(messages, [otherRun]);
    expect(result[1]).toMatchObject({ runId: "missing-run" });
  });
});
