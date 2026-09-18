// @vitest-environment jsdom
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { getAgentStatus } from "../../../integrations/agent/agentStatus";
import { flushAsync, renderHook } from "../../../test/renderHook";
import {
  AGENT_STARTUP_TIMEOUT_MS,
  AGENT_WAIT_HINT_DELAY_MS,
  MINIMUM_AGENT_SPLASH_MS,
  useAgentStatus,
} from "./useAgentStatus";

vi.mock("../../../integrations/agent/agentStatus", () => ({
  getAgentStatus: vi.fn(),
}));

describe("useAgentStatus", () => {
  beforeEach(() => {
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
});
