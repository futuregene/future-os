import { act } from "react";
// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync, renderHook } from "../../test/renderHook";

import {
  getAgentState,
  getCachedAgentState,
  installAgentEventListener,
  invalidateAgentState,
  listStreamingThreadIds,
  prefetchAgentState,
  revalidateAgentState,
  updateCachedAgentState,
  useCachedAgentState,
} from "./agentStateCache";

const invokeMock = vi.fn<(cmd: string, args?: unknown) => Promise<unknown>>();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}));

type Listener = (event: { payload: Record<string, unknown> | undefined }) => void;
let agentEventListener: Listener | null = null;
let providerConfigListener: Listener | null = null;

vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, handler: Listener) => {
    if (name === "agent-event")
      agentEventListener = handler;
    if (name === "provider-config-changed")
      providerConfigListener = handler;
    return Promise.resolve(() => {});
  },
}));

beforeEach(() => {
  invokeMock.mockReset();
});

function statePayload(overrides: Record<string, unknown> = {}) {
  return {
    model: "m1",
    thinkingLevel: "high",
    session_name: "Session",
    sessionId: "s1",
    cwd: "/w",
    parentSessionId: null,
    ...overrides,
  };
}

function emit(payload: Record<string, unknown> | undefined) {
  act(() => {
    agentEventListener?.({ payload });
  });
}

describe("agentStateCache fetch/cache", () => {
  it("installs the agent event listener exactly once", () => {
    installAgentEventListener();
    installAgentEventListener();
    expect(agentEventListener).not.toBeNull();
  });

  it("bridges Agent provider completion into the typed UI event bus", () => {
    installAgentEventListener();
    const handler = vi.fn();
    window.addEventListener("futureos:providers-changed", handler);
    act(() => {
      providerConfigListener?.({
        payload: {
          revision: 7,
          providerId: "custom",
          operation: "updated",
          authChanged: true,
          modelsChanged: true,
        },
      });
    });
    expect(handler).toHaveBeenCalledTimes(1);
    expect((handler.mock.calls[0]?.[0] as CustomEvent).detail).toMatchObject({
      revision: 7,
      providerId: "custom",
      operation: "updated",
    });
    window.removeEventListener("futureos:providers-changed", handler);
  });

  it("fetches and parses session state including activeRun variants", async () => {
    invokeMock.mockResolvedValue(statePayload({
      activeRun: { runId: "r1", state: "running", epoch: 2, lastEventIdx: 7 },
    }));
    const state = await getAgentState("t-fetch");
    expect(state).toMatchObject({
      model: "m1",
      thinkingLevel: "high",
      sessionName: "Session",
      sessionId: "s1",
      cwd: "/w",
      activeRun: { runId: "r1", state: "running", epoch: 2, lastEventIdx: 7 },
    });

    invokeMock.mockResolvedValue(statePayload({ activeRun: "nope" }));
    expect((await getAgentState("t-run-str")).activeRun).toBeNull();
    invokeMock.mockResolvedValue(statePayload({ activeRun: { runId: 1 } }));
    expect((await getAgentState("t-run-bad")).activeRun).toBeNull();
    invokeMock.mockResolvedValue(statePayload({ activeRun: { runId: "r", state: "queued" } }));
    expect((await getAgentState("t-run-min")).activeRun).toMatchObject({ epoch: 0, lastEventIdx: -1 });
  });

  it("parses missing fields as null", async () => {
    invokeMock.mockResolvedValue({});
    const state = await getAgentState("t-empty");
    expect(state).toMatchObject({ model: null, thinkingLevel: null, sessionName: null });
  });

  it("serves fresh entries from cache and dedupes in-flight requests", async () => {
    invokeMock.mockResolvedValue(statePayload());
    const first = await getAgentState("t-cache");
    // Fresh: no second invoke.
    const second = await getAgentState("t-cache");
    expect(second).toBe(first);
    expect(invokeMock).toHaveBeenCalledTimes(1);

    // Force bypasses the TTL.
    await getAgentState("t-cache", { force: true });
    expect(invokeMock).toHaveBeenCalledTimes(2);

    // In-flight dedupe.
    let resolveInvoke!: (v: unknown) => void;
    invokeMock.mockImplementation(() => new Promise((resolve) => {
      resolveInvoke = resolve;
    }));
    const a = getAgentState("t-flight", { force: true });
    const b = getAgentState("t-flight", { force: true });
    resolveInvoke(statePayload());
    expect(await a).toBe(await b);
  });

  it("getCachedAgentState returns undefined for null ids and misses", () => {
    expect(getCachedAgentState(null)).toBeUndefined();
    expect(getCachedAgentState("never-fetched")).toBeUndefined();
  });

  it("updateCachedAgentState patches existing and creates new entries", async () => {
    invokeMock.mockResolvedValue(statePayload());
    await getAgentState("t-patch");
    updateCachedAgentState("t-patch", { model: "m2" });
    expect(getCachedAgentState("t-patch")).toMatchObject({ model: "m2", thinkingLevel: "high" });

    updateCachedAgentState("t-fresh", { model: "m3" } as never);
    expect(getCachedAgentState("t-fresh")).toMatchObject({ model: "m3" });
  });

  it("invalidateAgentState drops the entry and notifies only when present", async () => {
    invokeMock.mockResolvedValue(statePayload());
    await getAgentState("t-inv");
    expect(getCachedAgentState("t-inv")).toBeDefined();
    invalidateAgentState("t-inv");
    expect(getCachedAgentState("t-inv")).toBeUndefined();
    // Absent entry: no notification, no throw.
    invalidateAgentState("t-inv");
  });

  it("prefetch and revalidate tolerate nulls and swallow rejections", async () => {
    prefetchAgentState(null);
    revalidateAgentState(undefined);
    invokeMock.mockRejectedValue(new Error("offline"));
    prefetchAgentState("t-pre");
    revalidateAgentState("t-rev");
    await flushAsync();
    // No unhandled rejection.
  });

  it("keeps the optimistic update when a slower fetch resolves behind it", async () => {
    let resolveInvoke!: (v: unknown) => void;
    invokeMock.mockImplementation(() => new Promise((resolve) => {
      resolveInvoke = resolve;
    }));
    const pending = getAgentState("t-race");
    // The user changes the model while the fetch is in flight.
    updateCachedAgentState("t-race", { model: "m-optimistic" } as never);
    resolveInvoke(statePayload({ model: "m-stale" }));
    const state = await pending;
    expect(state.model).toBe("m-optimistic");
    expect(getCachedAgentState("t-race")).toMatchObject({ model: "m-optimistic" });
  });

  it("prunes past the 100-entry cap", () => {
    for (let i = 0; i < 105; i += 1) {
      updateCachedAgentState(`t-prune-${i}`, { model: "m" } as never);
    }
    expect(getCachedAgentState("t-prune-0")).toBeUndefined();
    expect(getCachedAgentState("t-prune-104")).toBeDefined();
  });

  it("useCachedAgentState re-renders subscribers on cache mutations", async () => {
    invokeMock.mockResolvedValue(statePayload());
    const h = renderHook(() => useCachedAgentState("t-sub"));
    expect(h.current).toBeUndefined();
    await act(async () => {
      await getAgentState("t-sub");
    });
    expect(h.current).toMatchObject({ model: "m1" });
    act(() => {
      updateCachedAgentState("t-sub", { model: "m9" });
    });
    expect(h.current).toMatchObject({ model: "m9" });
    h.unmount();
  });
});

describe("agentStateCache event listener", () => {
  it("ignores malformed payloads", () => {
    emit(undefined);
    emit({ threadId: "t1" }); // no sessionId/_eventType
    emit({ sessionId: "s1" }); // no _eventType
    // No crash, no cache writes.
  });

  it("applies settings events to cached threads sharing the session", async () => {
    invokeMock.mockResolvedValue(statePayload());
    await getAgentState("t-set");
    expect(getCachedAgentState("t-set")).toMatchObject({ model: "m1" });

    emit({ _eventType: "model_changed", sessionId: "s1", model: "m-new" });
    expect(getCachedAgentState("t-set")).toMatchObject({ model: "m-new" });

    emit({ _eventType: "thinking_level_changed", sessionId: "s1", level: "low" });
    expect(getCachedAgentState("t-set")).toMatchObject({ thinkingLevel: "low" });

    emit({ _eventType: "session_name_changed", sessionId: "s1", name: "Renamed" });
    expect(getCachedAgentState("t-set")).toMatchObject({ sessionName: "Renamed" });

    // Non-string values leave the entry unchanged.
    emit({ _eventType: "model_changed", sessionId: "s1", model: 42 });
    expect(getCachedAgentState("t-set")).toMatchObject({ model: "m-new" });

    // Events for other sessions are skipped.
    emit({ _eventType: "model_changed", sessionId: "other", model: "m-x" });
    expect(getCachedAgentState("t-set")).toMatchObject({ model: "m-new" });
  });

  it("cwd_changed reconciles the workspace and updates the cache", async () => {
    invokeMock.mockResolvedValue(statePayload());
    await getAgentState("t-cwd");
    invokeMock.mockResolvedValue(undefined);
    const cwdEvents: Event[] = [];
    window.addEventListener("future:cwd-changed", e => cwdEvents.push(e));
    emit({ _eventType: "cwd_changed", sessionId: "s1", cwd: "/new" });
    await flushAsync();
    expect(invokeMock).toHaveBeenCalledWith("reconcile_thread_workspace", { sessionId: "s1", cwd: "/new" });
    expect(cwdEvents).toHaveLength(1);
    expect(getCachedAgentState("t-cwd")).toMatchObject({ cwd: "/new" });
  });

  it("toasts when the workspace reconcile fails", async () => {
    invokeMock.mockRejectedValue(new Error("no access"));
    const toasts: CustomEvent[] = [];
    window.addEventListener("futureos:toast", e => toasts.push(e as CustomEvent));
    emit({ _eventType: "cwd_changed", sessionId: "s1", cwd: "/bad" });
    await flushAsync();
    await flushAsync();
    expect(toasts.length).toBeGreaterThan(0);
    expect(toasts[0]?.detail.tone).toBe("error");
  });

  it("config_reloaded drops and revalidates the entry", async () => {
    invokeMock.mockResolvedValue(statePayload());
    await getAgentState("t-conf");
    invokeMock.mockResolvedValue(statePayload({ model: "m-reloaded" }));
    emit({ _eventType: "config_reloaded", sessionId: "s1" });
    await flushAsync();
    await flushAsync();
    expect(getCachedAgentState("t-conf")).toMatchObject({ model: "m-reloaded" });
  });

  it("forwards content events as window CustomEvents", () => {
    const received: CustomEvent[] = [];
    window.addEventListener("future:agent-event", e => received.push(e as CustomEvent));
    emit({ _eventType: "user_message", sessionId: "s1", threadId: "t1", text: "hi" });
    emit({
      _eventType: "compaction_started",
      sessionId: "s1",
      threadId: "t1",
      operation_id: "cmp_1",
      trigger: "manual",
      phase: "before_model_switch",
    });
    emit({
      _eventType: "compaction_committed",
      sessionId: "s1",
      threadId: "t1",
      operation_id: "cmp_1",
      checkpoint_id: "cp_1",
    });
    emit({
      _eventType: "compaction_failed",
      sessionId: "s1",
      threadId: "t1",
      operation_id: "cmp_2",
      error: "provider unavailable",
    });
    expect(received).toHaveLength(4);
    expect(received[0]?.detail).toMatchObject({ threadId: "t1", eventType: "user_message" });
    expect(received[1]?.detail).toMatchObject({
      threadId: "t1",
      eventType: "compaction_started",
      payload: { operation_id: "cmp_1" },
    });
    expect(received[2]?.detail).toMatchObject({
      threadId: "t1",
      eventType: "compaction_committed",
      payload: { operation_id: "cmp_1", checkpoint_id: "cp_1" },
    });
    expect(received[3]?.detail).toMatchObject({
      threadId: "t1",
      eventType: "compaction_failed",
      payload: { operation_id: "cmp_2", error: "provider unavailable" },
    });
    // Without a threadId the event is dropped.
    emit({ _eventType: "agent_end", sessionId: "s1" });
    expect(received).toHaveLength(4);
  });
});

describe("listStreamingThreadIds", () => {
  it("returns the ids, tolerating non-array and failure", async () => {
    invokeMock.mockResolvedValue(["t1", "t2"]);
    await expect(listStreamingThreadIds()).resolves.toEqual(["t1", "t2"]);
    invokeMock.mockResolvedValue({ bad: true });
    await expect(listStreamingThreadIds()).resolves.toEqual([]);
    invokeMock.mockRejectedValue(new Error("down"));
    await expect(listStreamingThreadIds()).resolves.toEqual([]);
  });
});
