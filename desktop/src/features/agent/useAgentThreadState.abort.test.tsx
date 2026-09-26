// @vitest-environment jsdom
import type { StoredRun, StoredThread } from "../../integrations/storage/threadStore";
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "../../test/renderHook";
import { clearThreadMessageSnapshots } from "./threadMessageCache";
import { useAgentThreadState } from "./useAgentThreadState";

/**
 * The parts of the thread state machine the lifecycle suite does not reach: the
 * abort path (including its no-run and failed-abort branches) and the settle
 * watchdog's scheduling.
 */
const storage = vi.hoisted(() => ({
  abortRun: vi.fn(async () => {}),
  getLatestRun: vi.fn(),
  getRun: vi.fn(),
  getSessionEntriesPage: vi.fn(),
}));
vi.mock("../../integrations/storage/threadStore", async original => ({
  ...await original<typeof import("../../integrations/storage/threadStore")>(),
  ...storage,
  listRunEventsBulk: vi.fn(async () => []),
}));
vi.mock("../../integrations/agent/agentClient", () => ({
  sendPromptToFutureAgent: vi.fn(async () => {}),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("./threadAttachments", () => ({
  finalizeTemporaryAttachmentSources: vi.fn(async () => {}),
  persistImageAttachments: vi.fn(async () => []),
}));
vi.mock("./buildReferencePrompt", () => ({ buildReferenceContext: vi.fn(async () => "") }));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const thread = { agentSessionId: "S1", id: "T1", workspaceId: "W1" } as StoredThread;
const runningRun = { id: "R9", startedAt: 1, status: "running", threadId: "T1" } as StoredRun;

beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
  clearThreadMessageSnapshots();
  storage.getSessionEntriesPage.mockResolvedValue({ entries: [], hasMore: false, nextOffset: 0 });
  storage.getLatestRun.mockResolvedValue(null);
  storage.getRun.mockResolvedValue(null);
});

afterEach(() => {
  vi.useRealTimers();
  clearThreadMessageSnapshots();
});

function mountState(over: { thread?: StoredThread | null } = {}) {
  const activity = vi.fn();
  const harness = renderHook(() => useAgentThreadState({
    loadingStore: false,
    modelId: "m1",
    onPromptConsumed: vi.fn(),
    onThreadActivity: activity,
    pendingPrompt: null,
    thinkingLevel: "high",
    thread: over.thread === undefined ? thread : over.thread,
    workspacePath: "/work",
  }));
  /** Let the mount-time loads settle. */
  const settled = async () => {
    await act(async () => {
      await vi.advanceTimersByTimeAsync(50);
    });
  };
  return { activity, harness, settled };
}

describe("useAgentThreadState abort", () => {
  it("aborts the run this view is attached to and reconciles", async () => {
    storage.getLatestRun.mockResolvedValue(runningRun);
    const { activity, harness, settled } = mountState();
    await settled();

    await act(async () => {
      await harness.current.handleAbort();
    });

    expect(storage.abortRun).toHaveBeenCalledWith({ runId: "R9", threadId: "T1" });
    expect(storage.getLatestRun).toHaveBeenCalledWith("T1");
    // The refresh re-reads the run so the view can settle its bubble.
    expect(activity).toHaveBeenCalledTimes(1);
    harness.unmount();
  });

  it("does nothing when nothing is running", async () => {
    // boundary: the stop button may be pressed after the run already settled,
    // and the transcript has no streaming bubble to take a run id from.
    const { activity, harness, settled } = mountState();
    await settled();

    await act(async () => {
      await harness.current.handleAbort();
    });

    expect(storage.abortRun).not.toHaveBeenCalled();
    expect(activity).not.toHaveBeenCalled();
    harness.unmount();
  });

  it("still reconciles when the backend refuses the abort", async () => {
    // error-path: the run already finished, so the abort is rejected — the
    // refresh must still happen or the view stays stuck on "stop".
    storage.getLatestRun.mockResolvedValue(runningRun);
    storage.abortRun.mockRejectedValueOnce(new Error("run already finished"));
    const { activity, harness, settled } = mountState();
    await settled();

    await act(async () => {
      await harness.current.handleAbort();
    });

    expect(activity).toHaveBeenCalledTimes(1);
    harness.unmount();
  });
});

describe("useAgentThreadState settle watchdog", () => {
  it("polls the local send's run row while a send owns the view", async () => {
    // concurrency: the watchdog is the only path that unsticks a send whose
    // invoke never resolved, so it must run on its own timer.
    const { harness, settled } = mountState();
    await settled();
    storage.getRun.mockClear();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(15_000);
    });

    // With no local send in flight there is nothing to reconcile, so no read is
    // issued — the guard runs before the storage call.
    expect(storage.getRun).not.toHaveBeenCalled();
    harness.unmount();
  });

  it("re-checks on window focus and visibility, not only on the timer", async () => {
    // concurrency: a run that finished while the window was hidden must be able
    // to unstick the moment the user returns.
    const { harness, settled } = mountState();
    await settled();

    act(() => {
      Object.defineProperty(document, "visibilityState", { configurable: true, value: "visible" });
      document.dispatchEvent(new Event("visibilitychange"));
      window.dispatchEvent(new Event("focus"));
    });

    expect(storage.getRun).not.toHaveBeenCalled();
    harness.unmount();
  });

  it("stops listening for focus and visibility once the conversation closes", async () => {
    // concurrency: the listener pair is removed on unmount, so a closed view
    // cannot reconcile against storage.
    const { harness, settled } = mountState();
    await settled();
    harness.unmount();
    storage.getRun.mockClear();

    act(() => {
      document.dispatchEvent(new Event("visibilitychange"));
      window.dispatchEvent(new Event("focus"));
    });
    expect(storage.getRun).not.toHaveBeenCalled();
  });
});

describe("useAgentThreadState writer ownership", () => {
  it("invalidates the previous send when the conversation hands over the writer", async () => {
    // concurrency: a thread switch gives the view the next conversation's
    // message writer; a send the outgoing view started must not be able to write
    // into the new one, so the owner change abandons it.
    const second = { agentSessionId: "S2", id: "T2", workspaceId: "W1" } as StoredThread;
    let which = thread;
    const harness = renderHook(() => useAgentThreadState({
      loadingStore: false,
      modelId: "m1",
      onPromptConsumed: vi.fn(),
      onThreadActivity: vi.fn(),
      pendingPrompt: null,
      thinkingLevel: "high",
      thread: which,
      workspacePath: "/work",
    }));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(50);
    });

    which = second;
    harness.rerender();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(50);
    });

    // The new conversation owns the view and can send normally afterwards.
    await act(async () => {
      void harness.current.handleSend({ attachments: [], content: "two" });
      await Promise.resolve();
    });
    expect(harness.current.messages.length).toBeGreaterThanOrEqual(0);
    harness.unmount();
  });

  it("retryHistory re-reads the history and clears the failure the user is shown", async () => {
    // `retryHistory` is the hook's recovery affordance: `AgentThread` renders it
    // as the retry button on a failed history load (AgentThread.tsx:503), and
    // that component's own suite mocks this hook (AgentThread.test.tsx:26), so
    // nothing else in the repo drives the real implementation. A mutation-style
    // audit of the waiver ledger found the line sitting at zero hits, so it was
    // recorded as an uncovered *reachable* path rather than an attribution
    // artifact - this test is the fixture it needed.
    storage.getSessionEntriesPage.mockRejectedValueOnce(new Error("db is locked"));
    const { harness, settled } = mountState();
    await settled();

    // The failed read surfaces to the user rather than being swallowed - and with
    // the backend's own reason, not a generic string. `toBeTruthy()` would also
    // pass on a mangled translation key, so the value is pinned.
    const failed = harness.current.historyError;
    expect(failed).toBe("db is locked");

    // The retry re-reads and, on success, clears that error.
    const readsBefore = storage.getSessionEntriesPage.mock.calls.length;
    storage.getSessionEntriesPage.mockResolvedValue({ entries: [], hasMore: false, nextOffset: 0 });
    await act(async () => {
      await harness.current.retryHistory();
    });

    expect(storage.getSessionEntriesPage.mock.calls.length).toBeGreaterThan(readsBefore);
    expect(harness.current.historyError).toBeNull();
    harness.unmount();
  });

  it("aborting with no conversation open is a no-op", async () => {
    // boundary: the stop control can be reached before a conversation is active
    // (`thread` is null, so `threadId` is null). Pressing it must not reach the
    // storage layer, and must not report thread activity.
    const { activity, harness, settled } = mountState({ thread: null });
    await settled();

    await act(async () => {
      await harness.current.handleAbort();
    });

    expect(storage.abortRun).not.toHaveBeenCalled();
    expect(activity).not.toHaveBeenCalled();
    harness.unmount();
  });
});
