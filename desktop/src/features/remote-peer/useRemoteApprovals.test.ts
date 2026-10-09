// @vitest-environment jsdom
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync, renderHook } from "../../test/renderHook";

/**
 * Approvals on a remote host are the one interactive thing this feature adds,
 * and they are the easiest to get wrong in a way nobody notices: the wrong
 * machine gets authorized, or a card lingers after someone else already
 * answered. Both are asserted here.
 */

const requestMock = vi.fn();
vi.mock("./remotePeerClient", () => ({
  requestRemotePeer: (...args: unknown[]) => requestMock(...args),
}));

let listeners: Array<(event: { payload: unknown }) => void> = [];
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (_name: string, handler: (event: { payload: unknown }) => void) => {
    listeners.push(handler);
    return () => {
      listeners = listeners.filter(item => item !== handler);
    };
  },
}));

const { useRemoteApprovals } = await import("./useRemoteApprovals");

function emit(payload: Record<string, unknown>, desktopId = "desktop_a"): void {
  act(() => {
    listeners.forEach(listener => listener({ payload: { desktopId, kind: "event", payload } }));
  });
}

function request(id: string, sessionId = "sess_1"): Record<string, unknown> {
  return {
    sessionId,
    type: "approval_request",
    eventId: `evt_${id}`,
    timestamp: "2026-01-01T00:00:00Z",
    data: JSON.stringify({
      approval_request_id: id,
      tool_name: "Bash",
      title: "Run a command",
      summary: "rm -rf /tmp/x",
      risk_level: "high",
    }),
  };
}

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i += 1) await flushAsync();
}

beforeEach(() => {
  requestMock.mockReset();
  requestMock.mockResolvedValue(undefined);
  listeners = [];
});

describe("useRemoteApprovals", () => {
  it("collects this session's pending approvals with what the user needs to decide", async () => {
    const hook = renderHook(() => useRemoteApprovals("desktop_a", "sess_1"));
    await settle();
    emit(request("ap1"));
    await settle();

    expect(hook.current.approvals).toHaveLength(1);
    expect(hook.current.approvals[0]).toMatchObject({
      id: "ap1",
      sessionId: "sess_1",
      toolName: "Bash",
      title: "Run a command",
      summary: "rm -rf /tmp/x",
      riskLevel: "high",
    });
    hook.unmount();
  });

  /** An approval for another host or another session is not ours to answer. */
  it("ignores approvals from another host or another session", async () => {
    const hook = renderHook(() => useRemoteApprovals("desktop_a", "sess_1"));
    await settle();
    emit(request("ap1"), "desktop_b");
    emit(request("ap2", "sess_other"));
    await settle();
    expect(hook.current.approvals).toEqual([]);
    hook.unmount();
  });

  /**
   * The decision must name the host, not "the current one": with several
   * desktops paired, sending it to the wrong machine either does nothing or
   * answers a question on a machine the user was not looking at.
   */
  it("sends the decision to the host that asked, for the session that asked", async () => {
    const hook = renderHook(() => useRemoteApprovals("desktop_a", "sess_1"));
    await settle();
    emit(request("ap1"));
    await settle();

    await act(async () => {
      await hook.current.decide(hook.current.approvals[0]!, "allow");
    });

    expect(requestMock).toHaveBeenCalledWith(
      "desktop_a",
      expect.objectContaining({
        type: "approval_decision",
        sessionId: "sess_1",
        entryId: "ap1",
        decision: "allow",
      }),
      "sess_1",
    );
    // Answered cards leave the list so the user cannot answer twice.
    expect(hook.current.approvals).toEqual([]);
    hook.unmount();
  });

  /** A re-delivered request replaces its card instead of duplicating it. */
  it("does not duplicate a re-delivered request", async () => {
    const hook = renderHook(() => useRemoteApprovals("desktop_a", "sess_1"));
    await settle();
    emit(request("ap1"));
    await settle();
    emit(request("ap1"));
    await settle();
    expect(hook.current.approvals).toHaveLength(1);
    hook.unmount();
  });

  /**
   * Another client (the phone, the host's own window) can answer the same
   * request. Its `approval_decision` push must clear our card: a stale card
   * invites the user to answer a question that no longer exists.
   */
  it("clears a card that someone else answered", async () => {
    const hook = renderHook(() => useRemoteApprovals("desktop_a", "sess_1"));
    await settle();
    emit(request("ap1"));
    await settle();
    expect(hook.current.approvals).toHaveLength(1);

    emit({
      sessionId: "sess_1",
      type: "approval_decision",
      eventId: "evt_d",
      timestamp: "2026-01-01T00:00:01Z",
      data: JSON.stringify({ approval_request_id: "ap1", decision: "allow" }),
    });
    await settle();
    expect(hook.current.approvals).toEqual([]);
    hook.unmount();
  });

  /** A decision that never reached the host must be visible, not silently lost. */
  it("reports a failed decision and keeps the card", async () => {
    requestMock.mockRejectedValue(new Error("peer_not_connected"));
    const hook = renderHook(() => useRemoteApprovals("desktop_a", "sess_1"));
    await settle();
    emit(request("ap1"));
    await settle();

    await act(async () => {
      await hook.current.decide(hook.current.approvals[0]!, "allow");
    });

    expect(hook.current.errors.ap1).toBe("peer_not_connected");
    expect(hook.current.approvals).toHaveLength(1);
    expect(hook.current.pending).toBeNull();
    hook.unmount();
  });

  it("keeps approvals from different sessions of the same host apart", async () => {
    const hook = renderHook(() => useRemoteApprovals("desktop_a", "sess_1"));
    await settle();
    emit(request("ap1", "sess_1"));
    await settle();
    expect(hook.current.approvals.map(item => item.id)).toEqual(["ap1"]);
    hook.unmount();

    requestMock.mockClear();
    const other = renderHook(() => useRemoteApprovals("desktop_a", "sess_2"));
    await settle();
    emit(request("ap2", "sess_2"));
    await settle();
    expect(other.current.approvals.map(item => item.id)).toEqual(["ap2"]);
    other.unmount();
  });

  it("collects nothing while no conversation is open", async () => {
    const hook = renderHook(() => useRemoteApprovals(null, null));
    await settle();
    emit(request("ap1"));
    await settle();
    expect(hook.current.approvals).toEqual([]);
    hook.unmount();
  });
});
