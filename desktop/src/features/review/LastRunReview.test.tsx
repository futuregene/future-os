// @vitest-environment jsdom
import type {
  LastRunReviewData,
  StoredReviewChangeset,
  StoredReviewFileChange,
} from "../../integrations/storage/types";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LastRunReview } from "./LastRunReview";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
(globalThis as Record<string, unknown>).ResizeObserver = ResizeObserverStub;

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
    path: "src/a.ts",
    changeType: "M",
    diff: "@@ -1 +1 @@\n-old\n+new",
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

interface Props {
  changePreview?: "ready" | "unsupported_too_large";
  error?: string | null;
  loading?: boolean;
  onRetry?: () => void;
  retrying?: boolean;
  review?: LastRunReviewData | null;
}

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

async function mount(overrides: Props = {}) {
  const props = {
    changePreview: "ready" as const,
    error: null,
    loading: false,
    onRetry: () => {},
    retrying: false,
    review: runReview(),
    ...overrides,
  };
  await act(async () => {
    root.render(createElement(LastRunReview, props));
  });
  return props;
}

function fileHeaders(): HTMLButtonElement[] {
  return [...container.querySelectorAll<HTMLButtonElement>("section > button")];
}

function expandAllToggle(): HTMLButtonElement | null {
  return container.querySelector<HTMLButtonElement>("button[aria-label='Expand all'], button[aria-label='Collapse all']");
}

function bannerTexts(): string[] {
  return [...container.querySelectorAll(".border-warning-line")]
    .map(node => node.textContent ?? "");
}

beforeEach(() => {
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("lastRunReview states", () => {
  it("reports a workspace too large to preview, before any load state", async () => {
    await mount({ changePreview: "unsupported_too_large", review: null, loading: true, error: "boom" });
    expect(container.textContent).toContain("Directory too large, change preview disabled");
    expect(container.textContent).toContain("This Workspace has too many files; change preview is not generated for now.");
    expect(container.textContent).not.toContain("boom");
  });

  it("shows a loading placeholder only while there is nothing to show", async () => {
    await mount({ review: null, loading: true });
    expect(container.textContent).toContain("Loading last run changes...");

    // A silent poll tick (loading=false, data present) keeps the diff on screen.
    act(() => root.unmount());
    root = createRoot(container);
    await mount({ review: runReview(), loading: true });
    expect(container.textContent).not.toContain("Loading last run changes...");
    expect(fileHeaders()).toHaveLength(1);
  });

  it("surfaces the load error verbatim, ahead of the empty state", async () => {
    await mount({ review: null, loading: false, error: "get_last_run_review failed" });
    expect(container.textContent).toContain("get_last_run_review failed");
    expect(container.textContent).not.toContain("No previous run to review yet");
  });

  it("explains that there is no run to review yet", async () => {
    await mount({ review: null });
    expect(container.textContent).toContain("No previous run to review yet");
    expect(container.textContent).toContain("After an Agent run completes, file changes will appear here.");
  });

  it("explains a run that produced no file changes", async () => {
    await mount({ review: runReview({ files: [], changeset: changeset({ filesChanged: 0, additions: 0, deletions: 0 }) }) });
    expect(container.textContent).toContain("No file changes in the last run");
    expect(container.textContent).toContain("This run produced no workspace file changes.");
    expect(expandAllToggle()).toBeNull();
    // The stats header is still there, reading zero.
    expect(container.textContent).toContain("0 files");
    expect(container.textContent).toContain("+0");
    expect(container.textContent).toContain("-0");
  });
});

describe("lastRunReview banners", () => {
  it("stays silent for an ordinary complete snapshot", async () => {
    await mount({ review: runReview() });
    expect(bannerTexts()).toHaveLength(0);
  });

  it("names every degradation, and only the ones that apply", async () => {
    await mount({
      review: runReview({
        overlapped: true,
        confidence: "recovered",
        snapshotStatus: "partial",
      }),
    });
    const banners = bannerTexts();
    expect(banners).toHaveLength(3);
    expect(banners.join("|")).toContain("Other runs occurred in this Workspace during this run; some changes may come from concurrent runs");
    expect(banners.join("|")).toContain("Snapshot recovered after an app restart; change attribution may be imprecise");
    expect(banners.join("|")).toContain("Some files exceeded the size limit and no diff was generated");

    // snapshotStatus === "unavailable" replaces the partial wording.
    act(() => root.unmount());
    root = createRoot(container);
    await mount({ review: runReview({ snapshotStatus: "unavailable" }) });
    expect(bannerTexts()).toHaveLength(1);
    expect(bannerTexts()[0]).toContain("This run's change snapshot is unavailable");
  });

  it("offers a retry on an incomplete snapshot and reports the in-flight state", async () => {
    const onRetry = vi.fn();
    await mount({ review: runReview({ snapshotStatus: "incomplete" }), onRetry });
    expect(bannerTexts()[0]).toContain("This run's snapshot is incomplete; click to retry");

    const retry = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent === "Retry");
    expect(retry).toBeTruthy();
    expect(retry?.disabled).toBe(false);
    await act(async () => {
      retry?.click();
    });
    expect(onRetry).toHaveBeenCalledTimes(1);

    // While the retry is in flight the button is disabled and says so.
    act(() => root.unmount());
    root = createRoot(container);
    await mount({ review: runReview({ snapshotStatus: "incomplete" }), retrying: true });
    const busy = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent === "Retrying...");
    expect(busy).toBeTruthy();
    expect(busy?.disabled).toBe(true);
    expect(container.textContent).not.toContain("RetryRetry");
  });
});

describe("lastRunReview files", () => {
  it("keys each row by id and shows the headline stats", async () => {
    await mount({
      review: runReview({
        changeset: changeset({ additions: 1234567, deletions: 89, filesChanged: 2 }),
        files: [change({ id: "c1" }), change({ id: "c2", path: "src/b.ts" })],
      }),
    });
    expect(fileHeaders()).toHaveLength(2);
    expect(container.textContent).toContain("2 files");
    expect(container.textContent).toContain("+1,234,567");
    expect(container.textContent).toContain("-89");
  });

  it("marks a rename with both paths", async () => {
    await mount({
      review: runReview({
        files: [change({ previousPath: "src/old.ts", path: "src/new.ts" })],
      }),
    });
    expect(fileHeaders()[0]?.textContent).toContain("src/old.ts → src/new.ts");
  });

  it("falls back to Unknown file when the change has no path", async () => {
    await mount({ review: runReview({ files: [change({ path: null })] }) });
    expect(fileHeaders()[0]?.textContent).toContain("Unknown file");
  });

  it("reads a legacy change row that predates the stored counts as +0/-0", async () => {
    // Rows written before the +/- columns existed have no numbers; the header
    // must render placeholders rather than "undefined".
    const legacy = change({
      additions: undefined as unknown as number,
      deletions: undefined as unknown as number,
    });
    await mount({ review: runReview({ files: [legacy] }) });
    expect(fileHeaders()[0]?.textContent).toContain("+0");
    expect(fileHeaders()[0]?.textContent).toContain("-0");
    expect(fileHeaders()[0]?.textContent).not.toContain("undefined");
  });

  it("opens and closes a row on click, and expands all", async () => {
    await mount({
      review: runReview({ files: [change({ id: "c1" }), change({ id: "c2", path: "src/b.ts", diff: "@@ -0,0 +1 @@\n+b" })] }),
    });
    expect(container.textContent).not.toContain("+new");

    await act(async () => {
      fileHeaders()[0]?.click();
    });
    expect(container.textContent).toContain("+new");
    expect(container.textContent).not.toContain("+b");

    // One open row means the toggle offers to collapse everything…
    expect(expandAllToggle()?.getAttribute("aria-label")).toBe("Collapse all");
    await act(async () => {
      expandAllToggle()?.click();
    });
    expect(expandAllToggle()?.getAttribute("aria-label")).toBe("Expand all");
    expect(container.textContent).not.toContain("+b");
    expect(container.textContent).not.toContain("+new");

    // …and from fully collapsed it expands everything again.
    await act(async () => {
      expandAllToggle()?.click();
    });
    expect(expandAllToggle()?.getAttribute("aria-label")).toBe("Collapse all");
    expect(container.textContent).toContain("+b");
    expect(container.textContent).toContain("+new");
  });

  it.each([
    ["A", "Added"],
    ["M", "Modified"],
    ["D", "Deleted"],
    ["R", "Renamed"],
    ["C", "Copied"],
  ])("labels a %s change as %s", async (changeType, label) => {
    await mount({ review: runReview({ files: [change({ changeType })] }) });
    expect(fileHeaders()[0]?.textContent).toContain(label);
  });

  it("passes an unknown change type through instead of mislabelling it", async () => {
    await mount({ review: runReview({ files: [change({ changeType: "T" })] }) });
    expect(fileHeaders()[0]?.textContent).toContain("T");
    expect(fileHeaders()[0]?.textContent).not.toContain("Modified");
  });

  it("labels a binary change Binary and a sensitive one Sensitive", async () => {
    await mount({ review: runReview({ files: [change({ binary: true, changeType: "M" })] }) });
    expect(fileHeaders()[0]?.textContent).toContain("Binary");

    act(() => root.unmount());
    root = createRoot(container);
    await mount({ review: runReview({ files: [change({ omissionReason: "sensitive", diff: null })] }) });
    expect(fileHeaders()[0]?.textContent).toContain("Sensitive file");
    await act(async () => {
      fileHeaders()[0]?.click();
    });
    expect(container.textContent).toContain("A sensitive file changed; its contents were not saved.");
  });

  it("describes a binary file with its mime type and both sizes", async () => {
    await mount({
      review: runReview({
        changeset: changeset({ binaryFiles: 1 }),
        files: [change({ binary: true, mime: "image/png", beforeSize: 1024, afterSize: 3_500_000 })],
      }),
    });
    await act(async () => {
      fileHeaders()[0]?.click();
    });
    expect(container.textContent).toContain("Binary file; text diff not supported.");
    expect(container.textContent).toContain("Type: image/png");
    expect(container.textContent).toContain("Size: 1.0 KiB → 3.3 MiB");
    // No +/- counts on a binary row.
    expect(fileHeaders()[0]?.textContent).not.toContain("+1");
  });

  it("renders missing binary metadata as dashes instead of crashing", async () => {
    await mount({
      review: runReview({ files: [change({ binary: true, mime: null, beforeSize: null, afterSize: undefined })] }),
    });
    await act(async () => {
      fileHeaders()[0]?.click();
    });
    expect(container.textContent).toContain("Size: — → —");
    expect(container.textContent).not.toContain("Type:");
  });

  it("says a text file has no diff rather than rendering an empty one", async () => {
    await mount({ review: runReview({ files: [change({ diff: null })] }) });
    await act(async () => {
      fileHeaders()[0]?.click();
    });
    expect(container.textContent).toContain("No text diff.");
  });

  it("flags a truncated diff after the body", async () => {
    await mount({ review: runReview({ files: [change({ diffTruncated: true })] }) });
    await act(async () => {
      fileHeaders()[0]?.click();
    });
    expect(container.textContent).toContain("The diff is too large and has been truncated.");
    expect(container.textContent).toContain("+new");
  });

  it("does not flag truncation on an untruncated diff", async () => {
    await mount({ review: runReview({ files: [change()] }) });
    await act(async () => {
      fileHeaders()[0]?.click();
    });
    expect(container.textContent).not.toContain("has been truncated");
  });

  it("renders CJK and very long paths as the row title", async () => {
    const cjk = "工作区/模块/文件-テキスト.md";
    await mount({ review: runReview({ files: [change({ path: cjk })] }) });
    expect(fileHeaders()[0]?.querySelector("span span")?.textContent).toBe(cjk);
  });
});
