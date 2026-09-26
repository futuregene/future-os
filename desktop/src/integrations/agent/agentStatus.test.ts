import { beforeEach, describe, expect, it, vi } from "vitest";
import { getAgentStatus } from "./agentStatus";

const invokeMock = vi.fn<(cmd: string, args?: unknown) => Promise<unknown>>();

vi.mock("../tauri/invoke", () => ({
  invokeCommand: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}));

beforeEach(() => {
  invokeMock.mockReset();
});

describe("getAgentStatus", () => {
  it("returns the backend phase, desktop version and agent version", async () => {
    const status = { phase: "ready", desktopVersion: "1.2.3", agentVersion: "1.2.3" };
    invokeMock.mockResolvedValue(status);

    await expect(getAgentStatus()).resolves.toBe(status);
    expect(invokeMock).toHaveBeenCalledExactlyOnceWith("get_agent_status", undefined);
  });

  it("accepts a null agent version (the agent has not reported one yet)", async () => {
    invokeMock.mockResolvedValue({ phase: "starting", desktopVersion: "1.2.3", agentVersion: null });

    await expect(getAgentStatus()).resolves.toMatchObject({ phase: "starting", agentVersion: null });
  });

  it.each([
    "checking",
    "starting",
    "recovering",
    "ready",
    "spawn_failed",
    "exited",
    "startup_timeout",
    "unavailable",
    "incompatible",
  ])("passes the %s phase through unchanged", async (phase) => {
    invokeMock.mockResolvedValue({ phase, desktopVersion: "1.2.3", agentVersion: null });

    await expect(getAgentStatus()).resolves.toMatchObject({ phase });
  });

  it("rejects when the backend cannot answer (the caller keeps its last phase)", async () => {
    invokeMock.mockRejectedValue(new Error("ipc closed"));

    await expect(getAgentStatus()).rejects.toThrow("ipc closed");
  });
});
