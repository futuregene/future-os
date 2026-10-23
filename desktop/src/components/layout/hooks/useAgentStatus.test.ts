// @vitest-environment jsdom
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { getAgentStatus, subscribeShutdownProgress } from "../../../integrations/agent/agentStatus";
import { flushAsync, renderHook } from "../../../test/renderHook";
import {
  AGENT_STARTUP_TIMEOUT_MS,
  AGENT_WAIT_HINT_DELAY_MS,
  MINIMUM_AGENT_SPLASH_MS,
  useAgentStatus,
} from "./useAgentStatus";

vi.mock("../../../integrations/agent/agentStatus", () => ({
  getAgentStatus: vi.fn(),
  subscribeShutdownProgress: vi.fn(() => Promise.resolve(() => {})),
}));

describe("useAgentStatus", () => {
  beforeEach(() => {
    vi.mocked(subscribeShutdownProgress).mockImplementation(() => Promise.resolve(() => {}));
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-09-18T00:00:00Z"));
    vi.mocked(getAgentStatus).mockResolvedValue({
      phase: "ready",
      desktopVersion: "1.1.8",
      agentVersion: "1.1.8",
    });
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.mocked(getAgentStatus).mockReset();
  });

  it("accepts live cleanup stages and cannot be reset by a stale ready probe", async () => {
    let receive: Parameters<typeof subscribeShutdownProgress>[0] | undefined;
    const unlisten = vi.fn();
    vi.mocked(subscribeShutdownProgress).mockImplementation((callback) => {
      receive = callback;
      return Promise.resolve(unlisten);
    });
    const hook = renderHook(useAgentStatus);
    await flushAsync();
    const progress = { operationId: "stop-1", reason: "quit", state: "running" as const, stage: "agent" as const, elapsedMs: 10, errors: [] };
    act(() => receive?.(progress));
    expect(hook.current.phase).toBe("stopping");
    act(() => vi.advanceTimersByTime(MINIMUM_AGENT_SPLASH_MS));
    expect(hook.current.phase).toBe("stopping");
    act(() => receive?.({ ...progress, state: "failed", errors: ["deadline exceeded"] }));
    expect(hook.current.shutdown?.state).toBe("failed");
    expect(hook.current.error).toBe("deadline exceeded");
    act(() => receive?.(progress));
    expect(hook.current.shutdown?.state).toBe("failed");
  });

  it("keeps the startup screen visible for at least one second when Agent is already ready", async () => {
    const hook = renderHook(useAgentStatus);
    await flushAsync();
    expect(hook.current.phase).toBe("checking");

    act(() => vi.advanceTimersByTime(MINIMUM_AGENT_SPLASH_MS - 1));
    expect(hook.current.phase).toBe("checking");

    act(() => vi.advanceTimersByTime(1));
    expect(hook.current.phase).toBe("ready");
    hook.unmount();
  });

  it("keeps an admitted shell mounted through recovery while tracking real availability", async () => {
    const hook = renderHook(useAgentStatus);
    await flushAsync();
    act(() => vi.advanceTimersByTime(MINIMUM_AGENT_SPLASH_MS));
    expect(hook.current.admitted).toBe(true);
    vi.mocked(getAgentStatus).mockResolvedValue({ phase: "unavailable", desktopVersion: "1.1.8", agentVersion: null });
    act(() => vi.advanceTimersByTime(5000));
    await flushAsync();
    expect(hook.current.phase).toBe("unavailable");
    expect(hook.current.admitted).toBe(true);
    vi.mocked(getAgentStatus).mockResolvedValue({ phase: "ready", desktopVersion: "1.1.8", agentVersion: "1.1.8" });
    act(() => vi.advanceTimersByTime(2000));
    await flushAsync();
    expect(hook.current.phase).toBe("ready");
    expect(hook.current.admitted).toBe(true);
    hook.unmount();
  });

  it("shows the waiting hint only after five seconds", async () => {
    vi.mocked(getAgentStatus).mockResolvedValue({
      phase: "starting",
      desktopVersion: "1.1.8",
      agentVersion: null,
    });
    const hook = renderHook(useAgentStatus);
    await flushAsync();

    act(() => vi.advanceTimersByTime(AGENT_WAIT_HINT_DELAY_MS - 1));
    expect(hook.current.showWait).toBe(false);
    act(() => vi.advanceTimersByTime(1));
    expect(hook.current.showWait).toBe(true);
    hook.unmount();
  });

  it("classifies an Agent that is still starting after fifteen seconds as timed out", async () => {
    vi.mocked(getAgentStatus).mockResolvedValue({
      phase: "starting",
      desktopVersion: "1.1.8",
      agentVersion: null,
    });
    const hook = renderHook(useAgentStatus);
    await flushAsync();

    act(() => vi.advanceTimersByTime(AGENT_STARTUP_TIMEOUT_MS));
    expect(hook.current.phase).toBe("startup_timeout");
    hook.unmount();
  });

  it("does not turn an active recovery back into a startup timeout", async () => {
    vi.mocked(getAgentStatus).mockResolvedValue({
      phase: "recovering",
      desktopVersion: "1.1.8",
      agentVersion: null,
    });
    const hook = renderHook(useAgentStatus);
    await flushAsync();

    act(() => vi.advanceTimersByTime(AGENT_STARTUP_TIMEOUT_MS));
    expect(hook.current.phase).toBe("recovering");
    hook.unmount();
  });

  it("abandons a deferred commit once the hook is gone, and stops polling", async () => {
    // A ready probe inside the minimum-splash window is committed from a timer
    // rather than inline, so the commit has to re-check `cancelled` on its own:
    // the effect cleanup's `clearTimeout` cannot stop a callback the host has
    // already dequeued (the fault injected here), and a commit that ran anyway
    // would also schedule the *next* poll — a loop that outlives its view.
    const clearTimeoutSpy = vi.spyOn(window, "clearTimeout").mockImplementation(() => {});
    try {
      const hook = renderHook(useAgentStatus);
      await flushAsync();
      expect(vi.mocked(getAgentStatus)).toHaveBeenCalledTimes(1);

      hook.unmount();
      // The dequeued commit fires, then any poll it scheduled would fire too.
      act(() => vi.advanceTimersByTime(MINIMUM_AGENT_SPLASH_MS));
      act(() => vi.advanceTimersByTime(AGENT_WAIT_HINT_DELAY_MS + AGENT_STARTUP_TIMEOUT_MS));

      expect(vi.mocked(getAgentStatus)).toHaveBeenCalledTimes(1);
    }
    finally {
      clearTimeoutSpy.mockRestore();
    }
  });
});
