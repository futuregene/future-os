import type { AgentMessage, MessageSegment } from "@future-os/thread-projection";
import type { StoredRunEvent } from "../../../integrations/storage/threadStore";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { hasStaleRunningTool, resetRunProjection, upsertStreamingPreview } from "./liveRunProjection";

const listRunEventsSince = vi.fn();

vi.mock("../../../integrations/storage/threadStore", () => ({
  listRunEventsSince: (...args: unknown[]) => listRunEventsSince(...args),
}));

vi.mock("../../../lib/futureEvents", () => ({ emitFutureEvent: vi.fn() }));

function events(runId: string, list: Array<[string, Record<string, unknown>]>, offset = 0): StoredRunEvent[] {
  return list.map(([eventType, payload], index) => ({
    id: `${runId}_e${offset + index}`,
    runId,
    eventType,
    payload: JSON.stringify(payload),
    sequence: offset + index,
    createdAt: offset + index,
  }));
}

function shell(id: string): Record<string, unknown> {
  return { tool_id: id, tool_name: "shell", tool_args: { command: "ls" } };
}

/** Answer the ticks in order; each responder sees the cursor it was asked for. */
function scripted(responders: Array<(since: number) => StoredRunEvent[]>) {
  let index = 0;
  listRunEventsSince.mockImplementation(async (_runId: string, since: number) => {
    const responder = responders[index];
    index += 1;
    return responder ? responder(since) : [];
  });
}

/** The cursors the live preview asked for, in order. */
function cursors(): unknown[] {
  return listRunEventsSince.mock.calls.map(call => call[1]);
}

/** Drive a setMessages mock to a final state. */
function collect(initial: AgentMessage[]) {
  let state = initial;
  const setMessages = vi.fn((action: unknown) => {
    state = typeof action === "function" ? (action as (previous: AgentMessage[]) => AgentMessage[])(state) : action as AgentMessage[];
  });
  return { setMessages, state: () => state };
}

function bubbleSegments(state: AgentMessage[]): MessageSegment[] {
  return state.find(message => message.role === "assistant")?.segments ?? [];
}

function runningIds(segments: MessageSegment[]): string[] {
  return segments
    .filter((segment): segment is Extract<MessageSegment, { kind: "activity" }> => segment.kind === "activity")
    .filter(segment => segment.item.status === "running")
    .map(segment => segment.item.id);
}

beforeEach(() => {
  listRunEventsSince.mockReset();
});

describe("hasStaleRunningTool", () => {
  const running = (id: string): MessageSegment => (
    { kind: "activity", id, item: { id, kind: "shell", status: "running" } }
  );
  const completed = (id: string): MessageSegment => (
    { kind: "activity", id, item: { id, kind: "shell", status: "completed" } }
  );
  const thinking: MessageSegment = { kind: "thinking", id: "t", text: "" };

  it("flags a running call a later call already settled past", () => {
    expect(hasStaleRunningTool([running("a"), thinking, completed("b")])).toBe(true);
  });

  it("keeps a call the same message is still narrating", () => {
    // One assistant message may announce a call and keep streaming reasoning or
    // text of its own; `file-tree invalidation` tests cover that shape upstream.
    expect(hasStaleRunningTool([running("a"), thinking])).toBe(false);
    expect(hasStaleRunningTool([running("a"), { kind: "text", id: "x", text: "answer" }])).toBe(false);
  });

  it("keeps a call that is still the newest thing the run did", () => {
    expect(hasStaleRunningTool([thinking, completed("a"), running("b")])).toBe(false);
    // Two parallel calls announced in one step, either one still executing:
    // nothing settled after it, and the Agent runs a batch in order anyway.
    expect(hasStaleRunningTool([running("a"), running("b")])).toBe(false);
  });

  it("keeps a call interrupted by a compaction divider", () => {
    expect(hasStaleRunningTool([running("a"), { kind: "compaction", id: "c", status: "completed" }])).toBe(false);
  });
});

describe("live projection repairs", () => {
  // The projector cache is module state: every case owns its run id and begins
  // from a clean one, or a previous case's watermark would steer it.
  it("rebuilds from the whole log when the Agent says the window lost its head", async () => {
    const runId = "run-truncated-window";
    resetRunProjection(runId);
    scripted([
      // Tick 1: the run so far — one call still executing.
      () => events(runId, [["tool_start", shell("a")]]),
      // Tick 2: the Agent cannot continue from cursor 0 and says so. Appending
      // this tail would step the watermark over the call's result and leave the
      // row reading "running" for the rest of the turn.
      () => events(runId, [
        ["thinking_start", {}],
        ["tool_start", shell("b")],
        ["tool_end", { tool_id: "b", text: "ok" }],
      ], 2).map(event => ({ ...event, truncated: true })),
      // The rebuild reads the run whole, where the result exists.
      () => events(runId, [
        ["tool_start", shell("a")],
        ["tool_end", { tool_id: "a", text: "ok" }],
        ["thinking_start", {}],
        ["tool_start", shell("b")],
        ["tool_end", { tool_id: "b", text: "ok" }],
      ]),
    ]);
    const { setMessages, state } = collect([]);

    await upsertStreamingPreview(runId, 1000, setMessages);
    await upsertStreamingPreview(runId, 1000, setMessages);

    expect(cursors()).toEqual([-1, 0, -1]);
    expect(runningIds(bubbleSegments(state()))).toEqual([]);
  });

  it("re-projects once when a completion never reached the projector", async () => {
    const runId = "run-lost-completion";
    resetRunProjection(runId);
    scripted([
      () => events(runId, [["tool_start", shell("a")]]),
      // The call's result is missing from this window — the shape a live reply
      // showed for minutes: a running row, then the model already reasoning again.
      () => events(runId, [
        ["thinking_start", {}],
        ["tool_start", shell("b")],
        ["tool_end", { tool_id: "b", text: "ok" }],
      ], 2),
      // The journal knows the completion, so the repair is what fixes the row.
      () => events(runId, [
        ["tool_start", shell("a")],
        ["tool_end", { tool_id: "a", text: "ok" }],
        ["thinking_start", {}],
        ["tool_start", shell("b")],
        ["tool_end", { tool_id: "b", text: "ok" }],
      ]),
      () => events(runId, [["tool_end", { tool_id: "c", text: "ok" }]], 5),
    ]);
    const { setMessages, state } = collect([]);

    await upsertStreamingPreview(runId, 1000, setMessages);
    await upsertStreamingPreview(runId, 1000, setMessages);

    expect(cursors()).toEqual([-1, 0, -1]);
    expect(runningIds(bubbleSegments(state()))).toEqual([]);

    // One repair per run: the next tick continues incrementally instead of
    // paying another full read for a reply that still looks stale.
    await upsertStreamingPreview(runId, 1000, setMessages);
    expect(cursors()).toEqual([-1, 0, -1, 4]);
  });

  it("leaves a call that is still the live tail alone", async () => {
    const runId = "run-live-tail";
    resetRunProjection(runId);
    scripted([
      () => events(runId, [["thinking_start", {}], ["tool_start", shell("a")]]),
      () => [],
    ]);
    const { setMessages, state } = collect([]);

    await upsertStreamingPreview(runId, 1000, setMessages);
    await upsertStreamingPreview(runId, 1000, setMessages);

    expect(cursors()).toEqual([-1, 1]);
    expect(runningIds(bubbleSegments(state()))).toEqual(["a"]);
  });

  it("rebuilds when a payload cannot be applied to this projector", async () => {
    const runId = "run-content-gap";
    resetRunProjection(runId);
    scripted([
      () => events(runId, [["text_chunk", { text: "first" }]]),
      // A chunked body whose prefix this projector never saw: `applyContentRange`
      // refuses it, and the rest of the batch would be silently dropped with it.
      () => events(runId, [[
        "text_chunk",
        {
          text: "second",
          content_key: "body",
          content_offset: 64,
          content_bytes: 6,
        },
      ]], 1),
      () => events(runId, [
        ["text_chunk", { text: "first" }],
        ["text_chunk", { text: "second" }],
      ]),
    ]);
    const { setMessages, state } = collect([]);

    await upsertStreamingPreview(runId, 1000, setMessages);
    await upsertStreamingPreview(runId, 1000, setMessages);

    expect(cursors()).toEqual([-1, 0, -1]);
    expect(state().find(message => message.role === "assistant")?.content).toBe("firstsecond");
  });

  it("does not keep rereading the whole log after the rebuild itself fails", async () => {
    const runId = "run-unrepairable-content";
    resetRunProjection(runId);
    // The durable log itself starts in the middle of a content body. Both the
    // initial projection and its full rebuild must reject the same payload.
    listRunEventsSince.mockImplementation(async () => events(runId, [[
      "text_chunk",
      {
        text: "orphan",
        content_key: "body",
        content_offset: 64,
        content_bytes: 6,
      },
    ]]));
    const { setMessages } = collect([]);

    await upsertStreamingPreview(runId, 1000, setMessages);
    await upsertStreamingPreview(runId, 1000, setMessages);
    await upsertStreamingPreview(runId, 1000, setMessages);

    expect(cursors()).toEqual([-1, -1]);

    // A backend reset explicitly invalidates that failure and permits one new
    // attempt; without it, every preview tick above stays cheap.
    resetRunProjection(runId);
    await upsertStreamingPreview(runId, 1000, setMessages);
    expect(cursors()).toEqual([-1, -1, -1, -1]);
  });
});
