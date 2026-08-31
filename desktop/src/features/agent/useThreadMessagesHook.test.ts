// @vitest-environment jsdom
import type { AgentMessage } from "@future-os/thread-projection";
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "../../test/renderHook";
import { clearThreadMessageSnapshots, setThreadMessageSnapshot } from "./threadMessageCache";
import { useThreadMessages } from "./useThreadMessages";

const storageMocks = vi.hoisted(() => ({
  getLatestRun: vi.fn(),
  getRun: vi.fn(),
  getSessionEntries: vi.fn(),
  listRuns: vi.fn(),
}));

vi.mock("../../integrations/storage/threadStore", () => storageMocks);

beforeEach(() => {
  vi.useFakeTimers();
  clearThreadMessageSnapshots();
  storageMocks.getLatestRun.mockResolvedValue(null);
  storageMocks.getRun.mockResolvedValue(null);
  storageMocks.listRuns.mockResolvedValue([]);
  storageMocks.getSessionEntries.mockReturnValue(new Promise(() => {}));
});

afterEach(() => {
  vi.useRealTimers();
  vi.clearAllMocks();
});

describe("useThreadMessages warm snapshots", () => {
  it("keeps cached messages visible without a loading indicator while revalidating", async () => {
    const cached = [{ id: "cached", role: "user", content: "cached" }] as AgentMessage[];
    setThreadMessageSnapshot("thread-1", "session-1", cached);

    const hook = renderHook(() => useThreadMessages({
      threadId: "thread-1",
      agentSessionId: "session-1",
    }));

    expect(hook.current.messages).toBe(cached);
    expect(hook.current.loadingThread).toBe(true);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(500);
    });
    expect(hook.current.loadingIndicator).toBe(false);
    hook.unmount();
  });

  it("does not show a snapshot from an obsolete Agent session", () => {
    setThreadMessageSnapshot(
      "thread-1",
      "session-old",
      [{ id: "stale", role: "user", content: "stale" }] as AgentMessage[],
    );

    const hook = renderHook(() => useThreadMessages({
      threadId: "thread-1",
      agentSessionId: "session-new",
    }));

    expect(hook.current.messages).toEqual([]);
    hook.unmount();
  });
});
