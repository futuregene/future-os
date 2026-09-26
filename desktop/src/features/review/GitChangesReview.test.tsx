// @vitest-environment jsdom
import type { GitReview, GitReviewFile } from "../../integrations/storage/types";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ReviewHeader, ReviewStats, WorkingTreeReview } from "./GitChangesReview";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

// jsdom has no ResizeObserver; the floating scrollbar only needs the API to exist.
class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
(globalThis as Record<string, unknown>).ResizeObserver = ResizeObserverStub;

function gitFile(overrides: Partial<GitReviewFile> = {}): GitReviewFile {
  return {
    path: "src/a.ts",
    status: "M",
    additions: 3,
    deletions: 1,
    diff: "@@ -1,2 +1,3 @@\n context\n-old\n+new\n+extra",
    binary: false,
    diffTruncated: false,
    ...overrides,
  };
}

function review(overrides: Partial<GitReview> = {}): GitReview {
  return {
    isGitWorkspace: true,
    workspacePath: "/w",
    branch: "feat/x",
    additions: 10,
    deletions: 4,
    files: [gitFile()],
    ...overrides,
  };
}

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

async function mount(element: React.ReactElement) {
  await act(async () => {
    root.render(element);
  });
}

function fileHeaders(): HTMLButtonElement[] {
  return [...container.querySelectorAll<HTMLButtonElement>("section > button")];
}

function expandAllToggle(): HTMLButtonElement | null {
  return container.querySelector<HTMLButtonElement>("button[aria-label='Expand all'], button[aria-label='Collapse all']");
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

describe("workingTreeReview", () => {
  it("shows the empty state and no expand toggle when the tree is clean", async () => {
    await mount(createElement(WorkingTreeReview, { review: review({ files: [] }) }));
    expect(container.textContent).toContain("No uncommitted changes in the working tree");
    expect(container.textContent).toContain("The current branch has no changes relative to HEAD.");
    expect(expandAllToggle()).toBeNull();
    expect(fileHeaders()).toHaveLength(0);
  });

  it("renders file rows collapsed and toggles one open on click", async () => {
    const two = review({
      files: [
        gitFile({ path: "src/a.ts" }),
        gitFile({ path: "src/b.ts", diff: "@@ -0,0 +1 @@\n+b-only" }),
      ],
    });
    await mount(createElement(WorkingTreeReview, { review: two }));

    expect(fileHeaders()).toHaveLength(2);
    // Bodies default collapsed: neither diff is in the DOM yet.
    expect(container.textContent).not.toContain("b-only");

    await act(async () => {
      fileHeaders()[1]?.click();
    });
    expect(container.textContent).toContain("b-only");
    // …and only the clicked row opened.
    expect(container.textContent).not.toContain("+extra");

    await act(async () => {
      fileHeaders()[1]?.click();
    });
    expect(container.textContent).not.toContain("b-only");
  });

  it("expands and collapses every file from the header toggle, relabelling itself", async () => {
    const many = review({ files: [gitFile({ path: "a.ts" }), gitFile({ path: "b.ts" }), gitFile({ path: "c.ts" })] });
    await mount(createElement(WorkingTreeReview, { review: many }));

    const toggle = expandAllToggle();
    expect(toggle?.getAttribute("aria-label")).toBe("Expand all");
    expect(fileHeaders()).toHaveLength(3);

    await act(async () => {
      toggle?.click();
    });
    expect(expandAllToggle()?.getAttribute("aria-label")).toBe("Collapse all");
    expect(container.textContent).toContain("+extra");

    await act(async () => {
      expandAllToggle()?.click();
    });
    expect(expandAllToggle()?.getAttribute("aria-label")).toBe("Expand all");
    expect(container.textContent).not.toContain("+extra");
  });

  it("puts the header toggle back to Expand all once a partially open list is collapsed", async () => {
    const many = review({ files: [gitFile({ path: "a.ts" }), gitFile({ path: "b.ts" })] });
    await mount(createElement(WorkingTreeReview, { review: many }));

    await act(async () => {
      fileHeaders()[0]?.click();
    });
    expect(expandAllToggle()?.getAttribute("aria-label")).toBe("Collapse all");
    await act(async () => {
      expandAllToggle()?.click();
    });
    expect(expandAllToggle()?.getAttribute("aria-label")).toBe("Expand all");
  });

  it.each([
    [
      "a binary file",
      { binary: true, diff: "", additions: 0, deletions: 0 },
      "Binary file; text diff not supported.",
    ],
    [
      "a sensitive file",
      { omissionReason: "sensitive", diff: "" },
      "A sensitive file changed; its contents were not saved.",
    ],
    [
      "a file omitted for size",
      { omissionReason: "too_large", diff: "" },
      "Diff content is too large; the file and statistics are retained.",
    ],
    [
      "a file dropped by the total limit",
      { omissionReason: "total_limit", diff: "" },
      "Diff content is too large; the file and statistics are retained.",
    ],
    ["a file with no text diff", { diff: "" }, "No textual diff available."],
  ])("explains %s instead of rendering a diff", async (_label, overrides, expected) => {
    await mount(createElement(WorkingTreeReview, { review: review({ files: [gitFile(overrides)] }) }));
    await act(async () => {
      fileHeaders()[0]?.click();
    });
    expect(container.textContent).toContain(expected);
  });

  it("hides the +/- counts for a binary file but keeps them for an omitted text file", async () => {
    await mount(createElement(WorkingTreeReview, {
      review: review({ files: [gitFile({ path: "logo.png", binary: true, additions: 0, deletions: 0 })] }),
    }));
    // The header itself shows no counts, and the omission is stated once open.
    expect(fileHeaders()[0]?.textContent).not.toContain("+0");
    expect(fileHeaders()[0]?.textContent).not.toContain("-0");

    act(() => root.unmount());
    root = createRoot(container);
    await mount(createElement(WorkingTreeReview, {
      review: review({ files: [gitFile({ path: "big.ts", omissionReason: "too_large", additions: 7, deletions: 2 })] }),
    }));
    expect(fileHeaders()[0]?.textContent).toContain("+7");
    expect(fileHeaders()[0]?.textContent).toContain("-2");
  });

  it("renders a Unicode path and a 4k-character path intact", async () => {
    const longPath = `src/${"深い/".repeat(300)}файл-Ω.ts`;
    await mount(createElement(WorkingTreeReview, {
      review: review({ files: [gitFile({ path: longPath })] }),
    }));
    expect(fileHeaders()[0]?.textContent).toContain("файл-Ω.ts");
    expect(fileHeaders()[0]?.textContent?.length).toBeGreaterThan(longPath.length);
    expect(fileHeaders()[0]?.querySelector("span span")?.textContent).toBe(longPath);
  });

  it("keeps a 200-file working tree interactive", async () => {
    const files = Array.from({ length: 200 }, (_, index) => gitFile({
      path: `src/mod${index}/file${index}.ts`,
      diff: `@@ -0,0 +1 @@\n+line-${index}`,
    }));
    await mount(createElement(WorkingTreeReview, { review: review({ files }) }));
    expect(fileHeaders()).toHaveLength(200);
    expect(container.textContent).not.toContain("line-199");
    await act(async () => {
      expandAllToggle()?.click();
    });
    // Every row opened at once, first to last.
    expect(container.textContent).toContain("+line-0");
    expect(container.textContent).toContain("+line-199");
    expect(container.querySelectorAll("section > div")).toHaveLength(200);
    // A DOM-heavy boundary case: give it room to finish on a loaded machine.
  }, 20_000);
});

describe("reviewHeader", () => {
  it("names the branch and the change totals", async () => {
    await mount(createElement(ReviewHeader, {
      review: review({ branch: "feat/cov", additions: 1234567, deletions: 89 }),
      files: [gitFile(), gitFile({ path: "b.ts" })],
      showBranch: true,
    }));
    expect(container.textContent).toContain("feat/cov");
    expect(container.textContent).toContain("2 files");
    expect(container.textContent).toContain("+1,234,567");
    expect(container.textContent).toContain("-89");
  });

  it("falls back to HEAD for a detached workspace and can hide the branch entirely", async () => {
    await mount(createElement(ReviewHeader, {
      review: review({ branch: null }),
      files: [],
      showBranch: true,
    }));
    expect(container.textContent).toContain("HEAD");

    act(() => root.unmount());
    root = createRoot(container);
    await mount(createElement(ReviewHeader, {
      review: review({ branch: "feat/hidden" }),
      files: [],
      showBranch: false,
    }));
    expect(container.textContent).not.toContain("feat/hidden");
    expect(container.textContent).toContain("0 files");
  });

  it.each([
    [0, "0 files"],
    [1, "1 file"],
    [2, "2 files"],
  ])("pluralises a %i-file change set", async (count, expected) => {
    await mount(createElement(ReviewStats, {
      additions: 0,
      deletions: 0,
      filesChanged: count,
      numberFormat: new Intl.NumberFormat("en"),
    }));
    expect(container.textContent).toContain(expected);
  });
});
