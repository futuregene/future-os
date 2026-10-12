// @vitest-environment jsdom
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync, renderHook } from "../../test/renderHook";

/**
 * A conversation's usage, from the host that spent it.
 *
 * Read from that host's own session state, so its figure and this dialog cannot
 * disagree — and read when the dialog opens, because nobody looks at it otherwise
 * and a state read per streaming push would be a request per delta.
 */
const request = vi.fn();
vi.mock("./remotePeerClient", () => ({
  requestRemotePeer: (...args: unknown[]) => request(...args),
}));

const { useRemoteUsage } = await import("./useRemoteUsage");

beforeEach(() => {
  request.mockReset();
});

describe("useRemoteUsage", () => {
  it("reads the host's state and reports its usage", async () => {
    request.mockResolvedValue({
      usage: { inputTokens: 100, outputTokens: 20, cacheReadTokens: 40, costCny: 0.5, costInputCny: 0.3, costOutputCny: 0.2, costCacheReadCny: 0, costCacheWriteCny: 0 },
    });
    const hook = renderHook(() => useRemoteUsage("desktop_a", "sess_1", true));
    await flushAsync();

    expect(request).toHaveBeenCalledWith("desktop_a", { type: "get_state", sessionId: "sess_1" }, "sess_1");
    expect(hook.current.usage).toMatchObject({ inputTokens: 100, outputTokens: 20, costCny: 0.5 });
    hook.unmount();
  });

  /** Reads nothing until it is asked for. */
  it("does nothing while it is disabled", async () => {
    const hook = renderHook(() => useRemoteUsage("desktop_a", "sess_1", false));
    await flushAsync();
    expect(request).not.toHaveBeenCalled();
    expect(hook.current.usage).toBeUndefined();
    hook.unmount();
  });

  /**
   * A host that does not report usage is not a host that used nothing: the
   * dialog says so rather than showing a fabricated zero.
   */
  it("reports no usage for a host that does not send the field", async () => {
    request.mockResolvedValue({});
    const hook = renderHook(() => useRemoteUsage("desktop_a", "sess_1", true));
    await flushAsync();
    expect(hook.current.usage).toBeUndefined();
    hook.unmount();
  });

  /** A malformed field reads as zero, not as NaN. */
  it("treats a malformed figure as zero", async () => {
    request.mockResolvedValue({ usage: { inputTokens: "many", outputTokens: 5 } });
    const hook = renderHook(() => useRemoteUsage("desktop_a", "sess_1", true));
    await flushAsync();
    expect(hook.current.usage).toMatchObject({ inputTokens: 0, outputTokens: 5 });
    hook.unmount();
  });

  it("surfaces a read failure", async () => {
    request.mockRejectedValue(new Error("host refused"));
    const hook = renderHook(() => useRemoteUsage("desktop_a", "sess_1", true));
    await flushAsync();
    expect(hook.current.error).toBe("host refused");
    hook.unmount();
  });

  /** Switching conversations does not carry the previous one's figure. */
  it("drops the previous conversation's usage when the conversation changes", async () => {
    request.mockResolvedValue({ usage: { inputTokens: 7 } });
    // The harness re-renders the same closure, so the session is a variable it
    // can read across renders.
    let session = "sess_1";
    const hook = renderHook(() => useRemoteUsage("desktop_a", session, true));
    await flushAsync();
    expect(hook.current.usage?.inputTokens).toBe(7);

    // The second read is still in flight: the window in which a stale figure
    // would otherwise be on screen.
    request.mockReturnValue(new Promise(() => {}));
    session = "sess_2";
    await act(async () => hook.rerender());
    expect(hook.current.loading).toBe(true);
    expect(hook.current.usage).toBeUndefined();
    hook.unmount();
  });
});
