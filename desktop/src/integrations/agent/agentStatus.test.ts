import { beforeEach, describe, expect, it, vi } from "vitest";
import { getAgentStatus, subscribeShutdownProgress } from "./agentStatus";

const invokeMock = vi.fn<(cmd: string, args?: unknown) => Promise<unknown>>();
const listenMock = vi.fn<(event: string, handler: (event: { payload: unknown }) => void) => Promise<() => void>>();

vi.mock("../tauri/invoke", () => ({
  invokeCommand: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: (event: string, handler: (e: { payload: unknown }) => void) => listenMock(event, handler),
}));

beforeEach(() => {
  invokeMock.mockReset();
  listenMock.mockReset();
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

  describe("subscribeShutdownProgress", () => {
    it("forwards each progress payload and resolves with the unlisten handle", async () => {
      const unlisten = vi.fn();
      const registered = vi.fn();
      listenMock.mockImplementation((_event: string, handler: (event: { payload: unknown }) => void) => {
        registered(handler);
        return Promise.resolve(unlisten);
      });

      const seen: unknown[] = [];
      const stop = await subscribeShutdownProgress(progress => seen.push(progress));

      expect(listenMock).toHaveBeenCalledWith("desktop-shutdown-progress", expect.any(Function));
      const handler = registered.mock.calls[0]![0] as (event: { payload: unknown }) => void;
      const progress = { operationId: "op-1", reason: "user", state: "running", stage: "agent", elapsedMs: 12, errors: [] };
      handler({ payload: progress });

      expect(seen).toEqual([progress]);
      expect(stop).toBe(unlisten);
    });

    it("degrades to a no-op unlisten when the listener cannot be registered", async () => {
      // Outside Tauri (a plain browser build, or an IPC channel that is not up
      // yet) the subscribe must not reject the caller's effect — it hands back
      // a removable no-op instead.
      listenMock.mockRejectedValue(new Error("no ipc"));

      const stop = await subscribeShutdownProgress(() => {});
      expect(stop).toBeTypeOf("function");
      expect(() => stop()).not.toThrow();
    });
  });
});
