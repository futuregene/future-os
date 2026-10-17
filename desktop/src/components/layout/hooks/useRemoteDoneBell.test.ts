// @vitest-environment jsdom
import type { RemotePeerEvent } from "../../../features/remote-peer/remotePeerClient";
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync, renderHook } from "../../../test/renderHook";

vi.mock("@tauri-apps/api/window");

const playDoneBell = vi.fn();
vi.mock("../../../lib/doneBell", () => ({
  playDoneBell: () => playDoneBell(),
}));

const requestUserAttention = vi.fn();
vi.mock("../../../lib/windowAttention", () => ({
  requestUserAttention: () => requestUserAttention(),
}));

let listeners = new Set<(event: { payload: RemotePeerEvent }) => void>();
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (_name: string, handler: (event: { payload: RemotePeerEvent }) => void) => {
    listeners.add(handler);
    return () => {
      listeners.delete(handler);
    };
  },
}));

const { useRemoteDoneBell } = await import("./useRemoteDoneBell");

function push(payload: RemotePeerEvent): void {
  act(() => {
    listeners.forEach(listener => listener({ payload }));
  });
}

function event(type: string, overrides: Partial<RemotePeerEvent["payload"]> & { runId?: string } = {}): RemotePeerEvent {
  const { runId, ...rest } = overrides;
  return {
    desktopId: "desktop_a",
    kind: "event",
    payload: {
      sessionId: "sess_1",
      runId,
      type,
      ...rest,
    } as Record<string, unknown>,
  };
}

beforeEach(() => {
  playDoneBell.mockReset();
  requestUserAttention.mockReset();
  listeners = new Set();
});

describe("useRemoteDoneBell", () => {
  it("rings once when a remote run finishes", async () => {
    renderHook(() => useRemoteDoneBell(true));
    await flushAsync();

    push(event("run_started", { runId: "r1" }));
    push(event("run_finished", { runId: "r1" }));
    await flushAsync();

    expect(playDoneBell).toHaveBeenCalledTimes(1);
    expect(requestUserAttention).toHaveBeenCalledTimes(1);
  });

  it("does not ring for the same run twice", async () => {
    renderHook(() => useRemoteDoneBell(true));
    await flushAsync();

    push(event("run_started", { runId: "r1" }));
    push(event("run_finished", { runId: "r1" }));
    push(event("agent_end", { runId: "r1" }));
    push(event("run_finished", { runId: "r1" }));
    await flushAsync();

    expect(playDoneBell).toHaveBeenCalledTimes(1);
  });

  it("stays silent when bellOnComplete is off", async () => {
    renderHook(() => useRemoteDoneBell(false));
    await flushAsync();

    push(event("run_started", { runId: "r1" }));
    push(event("run_finished", { runId: "r1" }));
    await flushAsync();

    expect(playDoneBell).not.toHaveBeenCalled();
    expect(requestUserAttention).not.toHaveBeenCalled();
  });

  it("does not ring for run_started without a matching end", async () => {
    renderHook(() => useRemoteDoneBell(true));
    await flushAsync();

    push(event("run_started", { runId: "r1" }));
    await flushAsync();

    expect(playDoneBell).not.toHaveBeenCalled();
  });

  it("does not ring for historical end events after reconnect", async () => {
    renderHook(() => useRemoteDoneBell(true));
    await flushAsync();

    // Without a preceding run_start, the end is treated as stale history.
    push(event("run_finished", { runId: "r1" }));
    push(event("agent_end", { runId: "r1" }));
    await flushAsync();

    expect(playDoneBell).not.toHaveBeenCalled();
  });

  it("rings for a failed run too", async () => {
    renderHook(() => useRemoteDoneBell(true));
    await flushAsync();

    push(event("run_started", { runId: "r1" }));
    push(event("run_failed", { runId: "r1" }));
    await flushAsync();

    expect(playDoneBell).toHaveBeenCalledTimes(1);
  });

  it("does not ring for non-termination events", async () => {
    renderHook(() => useRemoteDoneBell(true));
    await flushAsync();

    push(event("text_chunk", { runId: "r1" }));
    push(event("tool_start", { runId: "r1" }));
    push(event("compaction_started", {}));
    await flushAsync();

    expect(playDoneBell).not.toHaveBeenCalled();
  });

  it("stops listening once unmounted", async () => {
    const hook = renderHook(() => useRemoteDoneBell(true));
    await flushAsync();
    hook.unmount();
    await flushAsync();

    push(event("run_started", { runId: "r1" }));
    push(event("run_finished", { runId: "r1" }));
    await flushAsync();

    expect(playDoneBell).not.toHaveBeenCalled();
  });

  it("still rings for a legacy agent_end without a runId", async () => {
    renderHook(() => useRemoteDoneBell(true));
    await flushAsync();

    push(event("run_started", { runId: "r1" }));
    push({
      desktopId: "desktop_a",
      kind: "event",
      payload: { sessionId: "sess_1", type: "agent_end" },
    });
    await flushAsync();

    expect(playDoneBell).toHaveBeenCalledTimes(1);
  });

  it("rings separately for different runs", async () => {
    renderHook(() => useRemoteDoneBell(true));
    await flushAsync();

    push(event("run_started", { runId: "r1" }));
    push(event("run_finished", { runId: "r1" }));
    push(event("run_started", { runId: "r2" }));
    push(event("run_failed", { runId: "r2" }));
    await flushAsync();

    expect(playDoneBell).toHaveBeenCalledTimes(2);
  });

  it("ignores a push that is not an event, or names no session", async () => {
    // A host's pushes also carry status chatter and partially-shaped events; a
    // payload that is not an object, or that carries no session, cannot be
    // identified as a run at all and must not ring.
    renderHook(() => useRemoteDoneBell(true));
    await flushAsync();

    push({ desktopId: "desktop_a", kind: "presence", payload: { sessionId: "sess_1", type: "agent_end" } });
    push({ desktopId: "desktop_a", kind: "event", payload: null as unknown as RemotePeerEvent["payload"] });
    push({ desktopId: "desktop_a", kind: "event", payload: "agent_end" as unknown as RemotePeerEvent["payload"] });
    push({ desktopId: "desktop_a", kind: "event", payload: { type: "agent_end" } as Record<string, unknown> });
    await flushAsync();

    expect(playDoneBell).not.toHaveBeenCalled();
    expect(requestUserAttention).not.toHaveBeenCalled();
  });
});
