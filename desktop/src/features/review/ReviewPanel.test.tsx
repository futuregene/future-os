// @vitest-environment jsdom
import type {
  GitReview,
  GitReviewFile,
  LastRunReviewData,
  StoredReviewChangeset,
  StoredReviewFileChange,
  StoredRun,
} from "../../integrations/storage/types";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { emitFutureEvent } from "../../lib/futureEvents";
import { ReviewPanel } from "./ReviewPanel";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
(globalThis as Record<string, unknown>).ResizeObserver = ResizeObserverStub;

const getLastRunReview = vi.fn<(threadId: string) => Promise<LastRunReviewData | null>>();
const retryRunReview = vi.fn<(runId: string) => Promise<LastRunReviewData | null>>();

vi.mock("../../integrations/storage/threadStore", () => ({
  getLastRunReview: (threadId: string) => getLastRunReview(threadId),
  retryRunReview: (runId: string) => retryRunReview(runId),
}));

function changeset(overrides: Partial<StoredReviewChangeset> = {}): StoredReviewChangeset {
  return {
    id: "cs1",
    threadId: "thread-1",
    runId: "run1",
    title: "run",
    status: "applied",
    filesChanged: 1,
    additions: 4,
    deletions: 2,
    sourceKind: "run_snapshot",
    binaryFiles: 0,
    omittedFiles: 0,
    completeness: "complete",
    confidence: "normal",
    overlapped: false,
    createdAt: 1,
    updatedAt: 1,
    ...overrides,
  };
}

function change(overrides: Partial<StoredReviewFileChange> = {}): StoredReviewFileChange {
  return {
    id: "c1",
    changesetId: "cs1",
    targetType: "file",
    path: "src/last-run.ts",
    changeType: "M",
    diff: "@@ -1 +1 @@\n-old\n+from-last-run",
    additions: 1,
    deletions: 1,
    binary: false,
    diffTruncated: false,
    createdAt: 1,
    updatedAt: 1,
    ...overrides,
  };
}

function runReview(overrides: Partial<LastRunReviewData> = {}): LastRunReviewData {
  return {
    changeset: changeset(),
    files: [change()],
    run: null,
    snapshotStatus: "complete",
    confidence: "normal",
    overlapped: false,
    ...overrides,
  };
}

function gitFile(overrides: Partial<GitReviewFile> = {}): GitReviewFile {
  return {
    path: "src/working-tree.ts",
    status: "M",
    additions: 2,
    deletions: 1,
    diff: "@@ -1 +1 @@\n-old\n+from-working-tree",
    binary: false,
    diffTruncated: false,
    ...overrides,
  };
}

function gitReview(overrides: Partial<GitReview> = {}): GitReview {
  return {
    isGitWorkspace: true,
    workspacePath: "/w",
    branch: "feat/panel",
    additions: 2,
    deletions: 1,
    files: [gitFile()],
    ...overrides,
  };
}

interface Props {
  branchReview?: GitReview | null;
  changePreview?: "ready" | "unsupported_too_large";
  isGitWorkspace?: boolean | null;
  threadId?: string;
  uncommittedReview?: GitReview | null;
}

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

async function mount(overrides: Props = {}, extra: { flush?: boolean } = {}) {
  const props = {
    branchReview: gitReview(),
    changePreview: "ready" as const,
    isGitWorkspace: true as boolean | null,
    threadId: "thread-1",
    uncommittedReview: gitReview({ files: [gitFile({ path: "src/uncommitted.ts", diff: "@@ -1 +1 @@\n-old\n+from-uncommitted" })] }),
    ...overrides,
  };
  await act(async () => {
    root.render(createElement(ReviewPanel, props));
  });
  if (extra.flush !== false)
    await flush();
  return props;
}

async function rerender(props: Parameters<typeof ReviewPanel>[0]) {
  await act(async () => {
    root.render(createElement(ReviewPanel, props));
  });
  await flush();
}

async function flush() {
  await act(async () => {
    await Promise.resolve();
  });
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, reject, resolve };
}

function tabs(): HTMLButtonElement[] {
  return [...container.querySelectorAll<HTMLButtonElement>("button")]
    .filter(button => ["Branch", "Uncommitted", "Last run"].includes(button.textContent ?? ""));
}

function fileHeaders(): HTMLButtonElement[] {
  return [...container.querySelectorAll<HTMLButtonElement>("section > button")];
}

beforeEach(() => {
  localStorage.clear();
  getLastRunReview.mockReset();
  retryRunReview.mockReset();
  getLastRunReview.mockResolvedValue(null);
  retryRunReview.mockResolvedValue(null);
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("reviewPanel view selection", () => {
  it("opens a git workspace on the committed branch delta", async () => {
    await mount();
    expect(tabs().map(tab => tab.textContent)).toEqual(["Branch", "Uncommitted", "Last run"]);
    expect(container.textContent).toContain("feat/panel");
    expect(container.textContent).toContain("src/working-tree.ts");
    expect(container.textContent).not.toContain("src/uncommitted.ts");
    await act(async () => {
      fileHeaders()[0]?.click();
    });
    expect(container.textContent).toContain("from-working-tree");
    // The last-run load still happens, so switching tabs is instant.
    expect(getLastRunReview).toHaveBeenCalledWith("thread-1");
  });

  it("switches between the three views on click", async () => {
    getLastRunReview.mockResolvedValue(runReview());
    await mount();
    await act(async () => {
      tabs()[1]?.click();
    });
    expect(fileHeaders()[0]?.textContent).toContain("src/uncommitted.ts");
    await act(async () => {
      fileHeaders()[0]?.click();
    });
    expect(container.textContent).toContain("from-uncommitted");
    expect(container.textContent).not.toContain("src/working-tree.ts");

    await act(async () => {
      tabs()[2]?.click();
    });
    await flush();
    // Last-run files default collapsed; open the single row.
    expect(container.textContent).toContain("Last run");
    await act(async () => {
      fileHeaders()[0]?.click();
    });
    expect(container.textContent).toContain("from-last-run");
  });

  it("shows only the last-run view for a non-git workspace", async () => {
    getLastRunReview.mockResolvedValue(runReview());
    await mount({ isGitWorkspace: false, branchReview: null, uncommittedReview: null });
    expect(tabs()).toHaveLength(0);
    expect(container.textContent).toContain("Last run");
    await act(async () => {
      fileHeaders()[0]?.click();
    });
    expect(container.textContent).toContain("from-last-run");
  });

  it("trusts the loaded review's capability while the context refresh is still null", async () => {
    // Capabilities arrive on the lightweight context refresh; until then the
    // review payload's own isGitWorkspace decides, so the UI never flashes the
    // non-git layout for a git workspace.
    getLastRunReview.mockResolvedValue(runReview());
    await mount({ isGitWorkspace: null, branchReview: null });
    expect(tabs()).toHaveLength(3);

    act(() => root.unmount());
    root = createRoot(container);
    await mount({ isGitWorkspace: null, branchReview: null, uncommittedReview: null });
    expect(tabs()).toHaveLength(0);
    expect(container.textContent).toContain("Last run");
  });

  it("trusts a loaded review that reports itself non-git while the capability is null", async () => {
    getLastRunReview.mockResolvedValue(runReview());
    await mount({
      isGitWorkspace: null,
      branchReview: gitReview({ isGitWorkspace: false }),
      uncommittedReview: null,
    });
    expect(tabs()).toHaveLength(0);
    expect(container.textContent).toContain("Last run");
  });

  it("renders nothing for a missing git diff instead of crashing", async () => {
    await mount({ branchReview: null, uncommittedReview: null });
    expect(tabs()).toHaveLength(3);
    expect(container.textContent).not.toContain("src/working-tree.ts");
    await act(async () => {
      tabs()[1]?.click();
    });
    expect(fileHeaders()).toHaveLength(0);
  });

  it("falls back to the branch review when there is no uncommitted diff", async () => {
    await mount({ uncommittedReview: null });
    expect(container.textContent).toContain("src/working-tree.ts");
  });

  it("returns to the branch view after visiting another tab", async () => {
    getLastRunReview.mockResolvedValue(runReview());
    await mount();
    await act(async () => {
      tabs()[1]?.click();
    });
    expect(container.textContent).toContain("src/uncommitted.ts");

    await act(async () => {
      tabs()[0]?.click();
    });
    expect(container.textContent).toContain("src/working-tree.ts");
    expect(container.textContent).not.toContain("src/uncommitted.ts");
  });
});

describe("reviewPanel last-run loading", () => {
  it("surfaces a load failure", async () => {
    getLastRunReview.mockRejectedValue(new Error("AGENT_UNREACHABLE: no agent"));
    await mount({ isGitWorkspace: false, branchReview: null, uncommittedReview: null });
    expect(container.textContent).toContain("AGENT_UNREACHABLE: no agent");
  });

  it("skips the load entirely when change previews are disabled for the workspace", async () => {
    await mount({ changePreview: "unsupported_too_large" });
    expect(getLastRunReview).not.toHaveBeenCalled();
    await act(async () => {
      tabs()[2]?.click();
    });
    expect(container.textContent).toContain("Directory too large, change preview disabled");
  });

  it("reloads when this thread's review is updated, and ignores other threads", async () => {
    await mount();
    expect(getLastRunReview).toHaveBeenCalledTimes(1);

    await act(async () => {
      emitFutureEvent("review-updated", { threadId: "other-thread" });
    });
    await flush();
    expect(getLastRunReview).toHaveBeenCalledTimes(1);

    await act(async () => {
      emitFutureEvent("review-updated", { threadId: "thread-1" });
    });
    await flush();
    expect(getLastRunReview).toHaveBeenCalledTimes(2);
  });

  it("unsubscribes on unmount so a late event cannot reload a dead panel", async () => {
    await mount();
    const calls = getLastRunReview.mock.calls.length;
    act(() => root.unmount());
    root = createRoot(container);
    await act(async () => {
      emitFutureEvent("review-updated", { threadId: "thread-1" });
    });
    await flush();
    expect(getLastRunReview).toHaveBeenCalledTimes(calls);
  });

  it("reloads for the new thread and drops the previous thread's late response", async () => {
    const first = deferred<LastRunReviewData | null>();
    const second = deferred<LastRunReviewData | null>();
    getLastRunReview.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);

    const props = await mount();
    expect(getLastRunReview).toHaveBeenCalledWith("thread-1");

    await rerender({ ...props, threadId: "thread-2" });
    expect(getLastRunReview).toHaveBeenLastCalledWith("thread-2");

    // The new thread answers first…
    await act(async () => {
      second.resolve(runReview({ files: [change({ path: "src/thread-2.ts", diff: "@@ -0,0 +1 @@\n+for-thread-2" })] }));
    });
    await flush();
    // …then the stale thread-1 response lands, and must not replace it.
    await act(async () => {
      first.resolve(runReview({ files: [change({ path: "src/thread-1.ts", diff: "@@ -0,0 +1 @@\n+for-thread-1" })] }));
    });
    await flush();

    const panelProps = { ...props, threadId: "thread-2" };
    await rerender(panelProps);
    await act(async () => {
      tabs()[2]?.click();
    });
    await flush();
    await act(async () => {
      fileHeaders()[0]?.click();
    });
    expect(container.textContent).toContain("for-thread-2");
    expect(container.textContent).not.toContain("for-thread-1");
  });

  it("swallows a rejection that arrives after the thread changed", async () => {
    const first = deferred<LastRunReviewData | null>();
    const second = deferred<LastRunReviewData | null>();
    getLastRunReview.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);

    const props = await mount();
    await rerender({ ...props, threadId: "thread-2" });
    await act(async () => {
      second.resolve(runReview());
    });
    await flush();
    await act(async () => {
      first.reject(new Error("stale failure"));
    });
    await flush();
    expect(container.textContent).not.toContain("stale failure");
  });
});

describe("reviewPanel retry", () => {
  function incomplete(run: StoredRun | null = null, runId: string | null = "run1") {
    return runReview({
      changeset: changeset({ runId }),
      run,
      snapshotStatus: "incomplete",
    });
  }

  async function openLastRun() {
    await act(async () => {
      tabs()[2]?.click();
    });
    await flush();
  }

  function retryButton(): HTMLButtonElement | undefined {
    return [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent === "Retry" || button.textContent === "Retrying...");
  }

  it("retries the run behind the incomplete snapshot and reloads on success", async () => {
    const first = runReview({ changeset: changeset({ runId: "run1" }), snapshotStatus: "incomplete" });
    getLastRunReview.mockResolvedValueOnce(first).mockResolvedValue(runReview());
    retryRunReview.mockResolvedValue(runReview());
    await mount();
    await openLastRun();

    await act(async () => {
      retryButton()?.click();
    });
    await flush();
    expect(retryRunReview).toHaveBeenCalledWith("run1");
    expect(getLastRunReview).toHaveBeenCalledTimes(2);
    // The snapshot is complete now, so the retry affordance is gone.
    expect(retryButton()).toBeUndefined();
  });

  it("prefers the run row's id and falls back to the changeset's", async () => {
    getLastRunReview.mockResolvedValue(incomplete({ id: "run-row" } as StoredRun, "changeset-run"));
    await mount();
    await openLastRun();
    await act(async () => {
      retryButton()?.click();
    });
    await flush();
    expect(retryRunReview).toHaveBeenCalledWith("run-row");
  });

  it("reports a failed retry", async () => {
    getLastRunReview.mockResolvedValue(incomplete());
    retryRunReview.mockRejectedValue(new Error("retry_run_review failed: snapshot gone"));
    await mount();
    await openLastRun();

    await act(async () => {
      retryButton()?.click();
    });
    await flush();

    // Observed behaviour: the retry error wins over the (still loaded) review,
    // so the banner — and with it the only Retry affordance — is replaced by
    // the error box until something else reloads the panel. Recorded in
    // docs/testing/desktop-panels.md as a finding, not silently changed here.
    expect(container.textContent).toContain("retry_run_review failed: snapshot gone");
    expect(container.textContent).not.toContain("snapshot is incomplete");
    expect(retryButton()).toBeUndefined();
    expect(getLastRunReview).toHaveBeenCalledTimes(1);
  });

  it("leaves the retry button alone when no run id is available to retry", async () => {
    const onRetry = vi.fn();
    getLastRunReview.mockResolvedValue(incomplete(null, null));
    retryRunReview.mockImplementation(onRetry as unknown as (runId: string) => Promise<LastRunReviewData | null>);
    await mount();
    await openLastRun();

    expect(retryButton()?.textContent).toBe("Retry");
    await act(async () => {
      retryButton()?.click();
    });
    await flush();
    expect(onRetry).not.toHaveBeenCalled();
    expect(getLastRunReview).toHaveBeenCalledTimes(1);
    expect(retryButton()?.textContent).toBe("Retry");
    expect(retryButton()?.disabled).toBe(false);
  });
});
