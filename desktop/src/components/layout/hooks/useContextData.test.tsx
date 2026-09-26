// @vitest-environment jsdom
import type { GitReview, StoredArtifact, StoredRun, StoredToolCall, WorkspaceReviewCapabilities } from "../../../integrations/storage/types";
import type { ContextTab } from "./useContextData";
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync, renderHook } from "../../../test/renderHook";
import { useContextData } from "./useContextData";

const mocks = vi.hoisted(() => ({
  artifacts: vi.fn(),
  capabilities: vi.fn(),
  ensureGit: vi.fn(),
  gitReview: vi.fn(),
  listRuns: vi.fn(),
  toolCallsBulk: vi.fn(),
}));

vi.mock("../../../integrations/storage/threadStore", () => ({
  ensureWorkspaceGit: (...args: unknown[]) => mocks.ensureGit(...args),
  getGitReview: (...args: unknown[]) => mocks.gitReview(...args),
  getWorkspaceReviewCapabilities: (...args: unknown[]) => mocks.capabilities(...args),
  listArtifacts: (...args: unknown[]) => mocks.artifacts(...args),
  listRuns: (...args: unknown[]) => mocks.listRuns(...args),
  listToolCallsBulk: (...args: unknown[]) => mocks.toolCallsBulk(...args),
}));

function run(id: string, threadId = "t1", status: StoredRun["status"] = "running"): StoredRun {
  return { id, threadId, status, createdAt: 0, updatedAt: 0 };
}

function toolCall(id: string, runId: string): StoredToolCall {
  return { id, runId, name: "read", kind: "read", status: "completed", createdAt: 0 };
}

function artifact(id: string): StoredArtifact {
  return { id, workspaceId: "w1", title: id, artifactType: "markdown", createdAt: 0, updatedAt: 0 };
}

function gitReview(branch: string): GitReview {
  return { isGitWorkspace: true, workspacePath: "/ws", branch, additions: 1, deletions: 0, files: [] };
}

const CAPABILITIES: WorkspaceReviewCapabilities = {
  isGitWorkspace: true,
  views: ["git_changes"],
  defaultView: "git_changes",
  changePreview: "ready",
};

interface Inputs {
  activeThreadId: string | null;
  activeThreadMode: "chat" | "workspace";
  activeWorkspaceId: string | null;
  activeWorkspacePath: string | null;
  workspaceScopeReady: boolean;
  activeTab: ContextTab;
  expanded: boolean;
}

function inputs(overrides: Partial<Inputs> = {}): Inputs {
  return {
    activeThreadId: "t1",
    activeThreadMode: "chat",
    activeWorkspaceId: "w1",
    activeWorkspacePath: "/ws",
    workspaceScopeReady: true,
    activeTab: "runs",
    expanded: true,
    ...overrides,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, reject, resolve };
}

/** Mount the hook over a mutable inputs object the test can rewrite. */
function mount(initial: Inputs) {
  const ref = { current: initial };
  const harness = renderHook(() => useContextData(ref.current));
  return { harness, ref };
}

describe("useContextData", () => {
  beforeEach(() => {
    // `performance` is faked too so the spinner's min-duration arithmetic is
    // driven by the fake clock instead of wall-clock time.
    vi.useFakeTimers({
      toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval", "Date", "performance"],
    });
    mocks.artifacts.mockReset().mockResolvedValue([]);
    mocks.capabilities.mockReset().mockResolvedValue(null);
    mocks.ensureGit.mockReset().mockResolvedValue(undefined);
    mocks.gitReview.mockReset().mockResolvedValue(gitReview("main"));
    mocks.listRuns.mockReset().mockResolvedValue([]);
    mocks.toolCallsBulk.mockReset().mockResolvedValue([]);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("clears everything and fetches nothing when there is no active thread", async () => {
    const { harness } = mount(inputs({ activeThreadId: null }));
    await flushAsync();

    expect(harness.current.runs).toEqual([]);
    expect(harness.current.runsScope).toBeNull();
    expect(harness.current.artifacts).toEqual([]);
    expect(harness.current.loading).toBe(false);
    expect(mocks.listRuns).not.toHaveBeenCalled();
    harness.unmount();
  });

  it("fetches runs, tool calls and artifacts for a chat thread", async () => {
    mocks.listRuns.mockResolvedValue([run("r1", "t1", "completed")]);
    mocks.toolCallsBulk.mockResolvedValue([["r1", [toolCall("c1", "r1")]]]);
    mocks.artifacts.mockResolvedValue([artifact("a1")]);

    const { harness } = mount(inputs());
    await flushAsync();

    expect(mocks.listRuns).toHaveBeenCalledWith("t1");
    expect(mocks.toolCallsBulk).toHaveBeenCalledWith(["r1"]);
    expect(harness.current.runs.map(item => item.id)).toEqual(["r1"]);
    expect(harness.current.toolsByRun).toEqual({ r1: [toolCall("c1", "r1")] });
    expect(harness.current.artifacts.map(item => item.id)).toEqual(["a1"]);
    expect(harness.current.runsScope).toEqual({ threadId: "t1", workspaceId: "w1", workspacePath: "/ws" });
    // Chat thread on the Runs tab: no git query, no capabilities probe.
    expect(mocks.gitReview).not.toHaveBeenCalled();
    expect(mocks.capabilities).not.toHaveBeenCalled();
    expect(harness.current.gitReviews).toEqual({ branch: null, uncommitted: null });
    harness.unmount();
  });

  it("queries both git diffs only while the Review tab is open", async () => {
    mocks.gitReview.mockImplementation(async ({ base }: { base: string }) => gitReview(base));

    const { harness, ref } = mount(inputs({ activeTab: "runs" }));
    await flushAsync();
    expect(mocks.gitReview).not.toHaveBeenCalled();

    ref.current = inputs({ activeTab: "review" });
    harness.rerender();
    await flushAsync();

    expect(mocks.gitReview.mock.calls.map(([args]) => args)).toEqual([
      { base: "branch", workspaceId: "w1" },
      { base: "head", workspaceId: "w1" },
    ]);
    expect(harness.current.gitReviews.branch?.branch).toBe("branch");
    expect(harness.current.gitReviews.uncommitted?.branch).toBe("head");
    harness.unmount();
  });

  it("ensures a workspace is under git and reports capabilities, skipping artifacts", async () => {
    mocks.capabilities.mockResolvedValue(CAPABILITIES);

    const { harness } = mount(inputs({ activeThreadMode: "workspace" }));
    await flushAsync();

    expect(mocks.ensureGit).toHaveBeenCalledWith("w1");
    expect(mocks.capabilities).toHaveBeenCalledWith("w1");
    expect(harness.current.reviewCapabilities).toEqual(CAPABILITIES);
    // Workspace threads show Review, not Artifacts (§14.6).
    expect(mocks.artifacts).not.toHaveBeenCalled();
    expect(harness.current.artifacts).toEqual([]);
    harness.unmount();
  });

  it("keeps loading the context when ensuring git fails", async () => {
    mocks.ensureGit.mockRejectedValue(new Error("git missing"));
    mocks.listRuns.mockResolvedValue([run("r1", "t1", "completed")]);

    const { harness } = mount(inputs({ activeThreadMode: "workspace" }));
    await flushAsync();

    expect(harness.current.runs.map(item => item.id)).toEqual(["r1"]);
    expect(harness.current.loading).toBe(false);
    harness.unmount();
  });

  it("does not touch the store until the workspace scope is ready", async () => {
    const { harness, ref } = mount(inputs({ workspaceScopeReady: false }));
    await flushAsync();
    expect(mocks.listRuns).not.toHaveBeenCalled();

    ref.current = inputs({ workspaceScopeReady: true });
    harness.rerender();
    await flushAsync();
    expect(mocks.listRuns).toHaveBeenCalledWith("t1");
    harness.unmount();
  });

  it("blanks the context when the bootstrap fetch itself fails", async () => {
    // No poll (panel closed) so the bootstrap refresh is the current generation
    // and its failure is the one that must blank the panel.
    mocks.listRuns.mockRejectedValue(new Error("db locked"));

    const { harness } = mount(inputs({ expanded: false }));
    await flushAsync();

    expect(harness.current.runs).toEqual([]);
    expect(harness.current.artifacts).toEqual([]);
    expect(harness.current.gitReviews).toEqual({ branch: null, uncommitted: null });
    expect(harness.current.loading).toBe(false);
    harness.unmount();
  });

  it("abandons a refresh whose git probe lands after a newer refresh began", async () => {
    const ensuring = deferred<void>();
    mocks.ensureGit.mockReturnValue(ensuring.promise);
    mocks.listRuns.mockResolvedValue([run("r1", "t1", "completed")]);

    const { harness, ref } = mount(inputs({ activeThreadMode: "workspace", expanded: false }));
    await flushAsync();
    // The tab effect fetched; the bootstrap refresh is parked inside
    // ensureWorkspaceGit, before its own listRuns call.
    expect(mocks.listRuns).toHaveBeenCalledTimes(1);
    expect(mocks.ensureGit).toHaveBeenCalledTimes(1);

    // A param-driven refresh overtakes it.
    ref.current = inputs({ activeThreadMode: "workspace", expanded: false, activeTab: "review" });
    harness.rerender();
    await flushAsync();
    expect(mocks.listRuns).toHaveBeenCalledTimes(2);

    await act(async () => {
      ensuring.resolve();
      await Promise.resolve();
    });

    // The parked refresh must not fetch again: its generation is stale.
    expect(mocks.listRuns).toHaveBeenCalledTimes(2);
    expect(harness.current.runs.map(item => item.id)).toEqual(["r1"]);
    harness.unmount();
  });

  it("continues past the git probe when that refresh is still the current one", () => {
    return (async () => {
      // The sibling test above covers the *stale* side of the post-git generation
      // check. This is the other side: a refresh whose git probe finishes while it
      // is still the newest one must carry on and fetch. (In the rest of this file
      // every `ensureGit` refresh happens to be superseded before it resumes,
      // which left this arm untaken.)
      const ensuring = deferred<void>();
      // The bootstrap refresh parks on a probe that never settles in this test;
      // the refresh started below gets one that resolves immediately, so it owns
      // the newest generation when the check runs.
      mocks.ensureGit.mockReturnValueOnce(ensuring.promise).mockResolvedValue(undefined);
      mocks.listRuns.mockResolvedValue([run("r1", "t1", "completed")]);

      const { harness } = mount(inputs({ activeThreadMode: "workspace" }));
      await flushAsync();
      mocks.listRuns.mockClear();

      await act(async () => {
        await harness.current.refreshContext({ ensureGit: true });
      });

      // It resumed past the generation check and fetched.
      expect(mocks.listRuns).toHaveBeenCalledWith("t1");
      expect(harness.current.runs.map(item => item.id)).toEqual(["r1"]);
      harness.unmount();
    })();
  });

  it("does not let a cancelled switch's spinner blank the thread that replaced it", async () => {
    // Fault injection: the guard `if (cancelled) return` exists for a browser
    // that has already dequeued the 200ms callback when the effect cleans up.
    // Blocking clearTimeout for that timer reproduces exactly that interleaving.
    const realSetTimeout = globalThis.setTimeout;
    const realClearTimeout = globalThis.clearTimeout;
    const spinnerTimers: unknown[] = [];
    const setTimeoutSpy = vi.spyOn(globalThis, "setTimeout").mockImplementation((callback, delay, ...rest) => {
      const id = realSetTimeout(callback, delay, ...rest);
      if (delay === 200)
        spinnerTimers.push(id);
      return id;
    });
    const clearTimeoutSpy = vi.spyOn(globalThis, "clearTimeout").mockImplementation((id) => {
      // Only the abandoned thread's timer is un-clearable; every later timer
      // (including the replacement thread's) behaves normally.
      if (id === spinnerTimers[0])
        return;
      realClearTimeout(id);
    });
    try {
      const stuck = deferred<StoredRun[]>();
      mocks.listRuns.mockReturnValueOnce(stuck.promise);
      const { harness, ref } = mount(inputs({ expanded: false }));
      await flushAsync();

      mocks.listRuns.mockResolvedValue([run("r2", "t2", "completed")]);
      ref.current = inputs({ activeThreadId: "t2", expanded: false });
      harness.rerender();
      await flushAsync();
      expect(harness.current.runs.map(item => item.id)).toEqual(["r2"]);

      // Walk past the abandoned thread's spinner delay.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(200);
      });

      expect(harness.current.runs.map(item => item.id)).toEqual(["r2"]);
      expect(harness.current.loading).toBe(false);
      harness.unmount();
    }
    finally {
      clearTimeoutSpy.mockRestore();
      setTimeoutSpy.mockRestore();
    }
  });

  it("releases the spinner immediately when the fetch outlasts its minimum", async () => {
    mocks.listRuns.mockResolvedValue([]);
    const { harness, ref } = mount(inputs({ activeThreadId: null, expanded: false }));
    await flushAsync();

    const slow = deferred<StoredRun[]>();
    mocks.listRuns.mockReturnValue(slow.promise);
    ref.current = inputs({ activeThreadId: "t9", expanded: false });
    harness.rerender();
    await flushAsync();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(400);
    });
    expect(harness.current.loading).toBe(true);

    await act(async () => {
      slow.resolve([run("r9", "t9", "completed")]);
      await Promise.resolve();
    });

    // More than LOADING_SPINNER_MIN_MS has passed since the spinner appeared, so
    // it must not serve a second term.
    expect(harness.current.loading).toBe(false);
    expect(harness.current.runs.map(item => item.id)).toEqual(["r9"]);
    harness.unmount();
  });

  it("discards a superseded response when the thread switches mid-flight", async () => {
    const slow = deferred<StoredRun[]>();
    mocks.listRuns.mockReturnValueOnce(slow.promise);

    const { harness, ref } = mount(inputs());
    await flushAsync();

    mocks.listRuns.mockResolvedValue([run("r2", "t2", "completed")]);
    ref.current = inputs({ activeThreadId: "t2" });
    harness.rerender();
    await flushAsync();
    expect(harness.current.runs.map(item => item.id)).toEqual(["r2"]);

    // The abandoned t1 fetch resolving late must not overwrite t2's rows.
    await act(async () => {
      slow.resolve([run("r1", "t1", "completed")]);
      await Promise.resolve();
    });
    expect(harness.current.runs.map(item => item.id)).toEqual(["r2"]);
    expect(harness.current.runsScope?.threadId).toBe("t2");
    harness.unmount();
  });

  it("shows the spinner only when a thread switch outlasts the delay, then holds it briefly", async () => {
    mocks.listRuns.mockResolvedValue([]);
    const { harness, ref } = mount(inputs({ activeThreadId: null }));
    await flushAsync();

    const slow = deferred<StoredRun[]>();
    mocks.listRuns.mockReturnValue(slow.promise);
    ref.current = inputs({ activeThreadId: "t9" });
    harness.rerender();
    await flushAsync();

    // Under the delay the spinner stays hidden.
    expect(harness.current.loading).toBe(false);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(200);
    });
    expect(harness.current.loading).toBe(true);

    await act(async () => {
      slow.resolve([run("r9", "t9", "completed")]);
      await Promise.resolve();
    });
    expect(harness.current.runs.map(item => item.id)).toEqual(["r9"]);
    // Minimum visible duration: the spinner cannot blink off the same tick.
    expect(harness.current.loading).toBe(true);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(199);
    });
    expect(harness.current.loading).toBe(true);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(2);
    });
    expect(harness.current.loading).toBe(false);
    harness.unmount();
  });

  it("never shows the spinner for a fast thread switch", async () => {
    const { harness, ref } = mount(inputs({ activeThreadId: "t1" }));
    await flushAsync();

    ref.current = inputs({ activeThreadId: "t2" });
    harness.rerender();
    await flushAsync();

    expect(harness.current.loading).toBe(false);
    harness.unmount();
  });

  it("polls fast while a run is live and slowly once everything settles", async () => {
    mocks.listRuns.mockResolvedValue([run("r1", "t1", "running")]);
    const { harness, ref } = mount(inputs());
    await flushAsync();

    // Bootstrap effect + tab effect + the poll's immediate tick, then a second
    // immediate tick once the live run flips the cadence to fast.
    const calls = () => mocks.listRuns.mock.calls.length;
    const afterMount = calls();
    expect(afterMount).toBeGreaterThanOrEqual(2);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1500);
    });
    expect(calls()).toBeGreaterThan(afterMount);

    // Once the run settles the cadence slows: 1.5s ticks stop firing.
    mocks.listRuns.mockResolvedValue([run("r1", "t1", "completed")]);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1500);
    });
    const settled = calls();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1500);
    });
    expect(calls()).toBe(settled);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(3600);
    });
    expect(calls()).toBe(settled + 1);

    // Closing the panel stops the poll entirely.
    ref.current = inputs({ activeThreadId: "t1", expanded: false });
    harness.rerender();
    await flushAsync();
    const whileClosed = calls();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(calls()).toBe(whileClosed);
    harness.unmount();
  });

  it("stops polling on unmount", async () => {
    mocks.listRuns.mockResolvedValue([run("r1", "t1", "running")]);
    const { harness } = mount(inputs());
    await flushAsync();
    const beforeUnmount = mocks.listRuns.mock.calls.length;

    harness.unmount();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });

    expect(mocks.listRuns.mock.calls.length).toBe(beforeUnmount);
  });

  it("refreshes on demand without blanking the rows already on screen", async () => {
    mocks.listRuns.mockResolvedValue([run("r1", "t1", "completed")]);
    const { harness } = mount(inputs());
    await flushAsync();

    mocks.listRuns.mockResolvedValue([run("r1", "t1", "completed"), run("r2", "t1", "completed")]);
    await act(async () => {
      await harness.current.refreshContext();
    });

    expect(harness.current.runs.map(item => item.id)).toEqual(["r1", "r2"]);
    expect(harness.current.loading).toBe(false);
    harness.unmount();
  });
});
