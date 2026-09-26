// @vitest-environment jsdom
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync } from "../../test/renderHook";
import { getAgentState, getCachedAgentState, installAgentEventListener } from "./agentStateCache";

const invokeMock = vi.fn<(cmd: string, args?: unknown) => Promise<unknown>>();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}));

type Listener = (event: { payload: Record<string, unknown> | undefined }) => void;
let agentEventListener: Listener | null = null;

vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, handler: Listener) => {
    if (name === "agent-event")
      agentEventListener = handler;
    return Promise.resolve(() => {});
  },
}));

beforeEach(() => {
  invokeMock.mockReset();
  installAgentEventListener();
});

function emit(payload: Record<string, unknown> | undefined) {
  act(() => {
    agentEventListener?.({ payload });
  });
}

function statePayload(overrides: Record<string, unknown> = {}) {
  return { model: "m1", thinkingLevel: "high", sessionName: "S", cwd: "/w", ...overrides };
}

function stateReads() {
  return invokeMock.mock.calls.filter(([cmd]) => cmd === "get_thread_agent_state").length;
}

describe("agentStateCache compaction events", () => {
  it("drops a compaction frame that names no thread, even when the session matches", async () => {
    // The cached entry shares the event's session, so a threadId would have made
    // this frame flip `isCompacting`. Without one there is nothing to stamp, and
    // the handler must bail out before touching any entry.
    invokeMock.mockResolvedValue(statePayload({ sessionId: "s-noThread", isCompacting: false }));
    await getAgentState("t-no-thread");
    const reads = stateReads();

    emit({ _eventType: "compaction_started", sessionId: "s-noThread", operation_id: "cmp_1" });
    emit({ _eventType: "compaction_committed", sessionId: "s-noThread", operation_id: "cmp_1" });
    emit({ _eventType: "compaction_failed", sessionId: "s-noThread", operation_id: "cmp_1" });
    emit({ _eventType: "compaction_unchanged", sessionId: "s-noThread", operation_id: "cmp_1" });
    await flushAsync();

    expect(getCachedAgentState("t-no-thread")?.isCompacting).toBe(false);
    expect(stateReads()).toBe(reads);

    // Control: the very same frame *with* a threadId does apply.
    emit({ _eventType: "compaction_started", sessionId: "s-noThread", threadId: "t-no-thread", operation_id: "cmp_1" });
    expect(getCachedAgentState("t-no-thread")?.isCompacting).toBe(true);
  });

  it("force-fetches authoritative state when a compaction event beats the prefetch", async () => {
    // Nothing in the cache matches this thread *or* this session: the event
    // cannot be applied, so the handler must ask the agent instead of leaving
    // the thread (whose prefetch lost the race) without any state.
    invokeMock.mockResolvedValue(statePayload({ sessionId: "s-cold", isCompacting: true }));
    expect(getCachedAgentState("t-cold")).toBeUndefined();

    emit({ _eventType: "compaction_started", sessionId: "s-cold", threadId: "t-cold", operation_id: "cmp_1" });
    await flushAsync();
    await flushAsync();

    expect(stateReads()).toBe(1);
    expect(getCachedAgentState("t-cold")?.isCompacting).toBe(true);
  });

  it("force-fetches for a terminal compaction event on a cold thread too", async () => {
    invokeMock.mockResolvedValue(statePayload({ sessionId: "s-cold-2", isCompacting: false }));

    emit({ _eventType: "compaction_committed", sessionId: "s-cold-2", threadId: "t-cold-2", operation_id: "cmp_2" });
    await flushAsync();
    await flushAsync();

    expect(stateReads()).toBe(1);
    expect(getCachedAgentState("t-cold-2")?.isCompacting).toBe(false);

    emit({ _eventType: "compaction_unchanged", sessionId: "s-cold-2", threadId: "t-cold-2", operation_id: "cmp_3" });
    await flushAsync();
    // The entry now exists, so the frame is applied in place — no extra read.
    expect(stateReads()).toBe(1);
  });

  it("swallows a failed recovery fetch instead of surfacing an unhandled rejection", async () => {
    invokeMock.mockRejectedValue(new Error("agent offline"));

    emit({ _eventType: "compaction_failed", sessionId: "s-cold-3", threadId: "t-cold-3", operation_id: "cmp_4" });
    await flushAsync();
    await flushAsync();

    expect(stateReads()).toBe(1);
    expect(getCachedAgentState("t-cold-3")).toBeUndefined();
  });
});
