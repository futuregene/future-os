import type {
  StoredApprovalRequest,
  StoredArtifact,
  StoredFile,
  StoredReviewChangeset,
  StoredRun,
} from "../../../integrations/storage/types";
import type { FutureReference } from "../futureMarkdownTypes";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { referenceKey } from "../futureMarkdownTypes";
import { renderFileReference } from "./fileReference";
import { FutureEmbed } from "./FutureEmbed";
import { MissingReference } from "./MissingReference";
import { PendingReference } from "./PendingReference";
import { ReferenceChip } from "./ReferenceChip";

const invokeMock = vi.fn<(cmd: string, args?: unknown) => Promise<unknown>>(() => Promise.resolve(null));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function ref(overrides: Partial<FutureReference> = {}): FutureReference {
  return {
    source: "block",
    targetId: "id-1",
    targetType: "run",
    view: "card",
    ...overrides,
  };
}

const storedFile: StoredFile = { path: "/w/a.md", name: "a.md", relativePath: "a.md", insideWorkspace: true };

const storedRun: StoredRun = {
  id: "run_abc_1",
  threadId: "t",
  status: "completed",
  createdAt: 1_000,
  updatedAt: 2_000,
};

const storedArtifact: StoredArtifact = {
  id: "art-1",
  workspaceId: "w",
  title: "Report",
  artifactType: "report",
  path: "/w/report.md",
  createdAt: 1_000,
  updatedAt: 2_000,
};

const storedApproval = {
  id: "ap-1",
  threadId: "t",
  kind: "shell",
  status: "pending",
  title: "Run ls",
  createdAt: 1_000,
  updatedAt: 2_000,
} as StoredApprovalRequest;

const storedReview: StoredReviewChangeset = {
  id: "rev-1",
  threadId: "t",
  title: "Changes",
  status: "pending",
  filesChanged: 2,
  additions: 10,
  deletions: 3,
  sourceKind: "git",
  binaryFiles: 0,
  omittedFiles: 0,
  completeness: "complete",
  confidence: "normal",
  overlapped: false,
  createdAt: 1_000,
  updatedAt: 2_000,
} as StoredReviewChangeset;

beforeEach(() => {
  invokeMock.mockClear();
});

describe("referenceKey", () => {
  it("joins type and id", () => {
    expect(referenceKey({ targetType: "run", targetId: "r1" })).toBe("run:r1");
  });
});

describe("pendingReference", () => {
  it("shows the label or target id", () => {
    expect(renderToStaticMarkup(createElement(PendingReference, { reference: ref({ label: "L" }) }))).toContain("L");
    expect(renderToStaticMarkup(createElement(PendingReference, { reference: ref() }))).toContain("id-1");
  });
});

describe("missingReference", () => {
  it("renders the red badge with the error as title when present", () => {
    const html = renderToStaticMarkup(
      createElement(MissingReference, { reference: ref(), error: "gone" }),
    );
    expect(html).toContain("title=\"gone\"");
  });

  it("falls back to the reference identity as title", () => {
    const html = renderToStaticMarkup(createElement(MissingReference, { reference: ref() }));
    expect(html).toContain("run:id-1");
  });
});

describe("referenceChip", () => {
  it("renders a pending placeholder while resolving", () => {
    const html = renderToStaticMarkup(createElement(ReferenceChip, { reference: ref() }));
    expect(html).toContain("id-1");
  });

  it("renders the missing badge for non-resolved statuses", () => {
    const html = renderToStaticMarkup(createElement(ReferenceChip, {
      reference: ref(),
      resolved: { targetType: "run", targetId: "id-1", status: "missing" },
    }));
    expect(html).toContain("bg-danger-soft");
  });

  it("renders the chip with per-type icons and label fallback", () => {
    for (const targetType of ["approval", "review", "run", "artifact"] as const) {
      const html = renderToStaticMarkup(createElement(ReferenceChip, {
        reference: ref({ targetType, label: undefined }),
        resolved: { targetType, targetId: "id-1", status: "resolved" },
      }));
      expect(html).toContain("id-1");
      expect(html).toContain("svg");
    }
    const labeled = renderToStaticMarkup(createElement(ReferenceChip, {
      reference: ref({ label: "My run" }),
      resolved: { targetType: "run", targetId: "id-1", status: "resolved" },
    }));
    expect(labeled).toContain("My run");
  });
});

describe("renderFileReference", () => {
  it("returns null for non-file references", () => {
    expect(renderFileReference(ref())).toBeNull();
  });

  it("renders a FileLink for a resolved file", () => {
    const node = renderFileReference(ref({ targetType: "file" }), {
      targetType: "file",
      targetId: "id-1",
      status: "resolved",
      data: storedFile,
    });
    const html = renderToStaticMarkup(createElement("div", null, node));
    expect(html).toContain("file:///w/a.md");
  });

  it("renders a pending placeholder for unresolved or malformed payloads", () => {
    const pending = renderFileReference(ref({ targetType: "file", label: "F" }));
    expect(renderToStaticMarkup(createElement("div", null, pending))).toContain("F");
    const malformed = renderFileReference(ref({ targetType: "file", label: "F" }), {
      targetType: "file",
      targetId: "id-1",
      status: "resolved",
      data: { bogus: true } as never,
    });
    expect(renderToStaticMarkup(createElement("div", null, malformed))).toContain("F");
  });
});

describe("futureEmbed", () => {
  function renderEmbed(reference: FutureReference, resolved?: Parameters<typeof FutureEmbed>[0]["resolved"]) {
    return renderToStaticMarkup(createElement(FutureEmbed, { reference, resolved }));
  }

  it("renders file references as links", () => {
    const html = renderEmbed(ref({ targetType: "file" }), {
      targetType: "file",
      targetId: "id-1",
      status: "resolved",
      data: storedFile,
    });
    expect(html).toContain("<a");
  });

  it("renders pending and missing states", () => {
    expect(renderEmbed(ref({ label: "P" }))).toContain("P");
    expect(renderEmbed(ref(), { targetType: "run", targetId: "id-1", status: "missing" }))
      .toContain("bg-danger-soft");
  });

  it("renders each resolved payload type", () => {
    expect(renderEmbed(ref({ targetType: "artifact" }), {
      targetType: "artifact",
      targetId: "id-1",
      status: "resolved",
      data: storedArtifact,
    })).toContain("Report");
    expect(renderEmbed(ref({ targetType: "run" }), {
      targetType: "run",
      targetId: "id-1",
      status: "resolved",
      data: storedRun,
    })).toContain("run_abc");
    expect(renderEmbed(ref({ targetType: "approval" }), {
      targetType: "approval",
      targetId: "id-1",
      status: "resolved",
      data: storedApproval,
    })).toContain("Run ls");
    expect(renderEmbed(ref({ targetType: "review" }), {
      targetType: "review",
      targetId: "id-1",
      status: "resolved",
      data: storedReview,
    })).toContain("Changes");
  });

  it("renders the missing badge for malformed payloads and type mismatches", () => {
    for (const targetType of ["artifact", "run", "approval", "review"] as const) {
      const html = renderEmbed(ref({ targetType }), {
        targetType,
        targetId: "id-1",
        status: "resolved",
        data: null,
      });
      expect(html).toContain("bg-danger-soft");
    }
    const mismatch = renderEmbed(ref({ targetType: "run" }), {
      targetType: "artifact",
      targetId: "id-1",
      status: "resolved",
      data: storedArtifact,
    });
    expect(mismatch).toContain("bg-danger-soft");
  });
});

describe("runEmbed interactions", () => {
  function mount(run: StoredRun) {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    act(() => {
      root.render(createElement(FutureEmbed, {
        reference: ref(),
        resolved: { targetType: "run", targetId: run.id, status: "resolved", data: run },
      }));
    });
    return { container, root };
  }

  it("emits inspect-run on button click and shows startedAt/errorMessage", () => {
    const events: CustomEvent[] = [];
    window.addEventListener("futureos:inspect-run", e => events.push(e as CustomEvent));
    const { container, root } = mount({
      ...storedRun,
      startedAt: 500,
      errorMessage: "boom",
      modelId: "m1",
    });
    const html = container.innerHTML;
    expect(html).toContain("boom");
    expect(html).toContain("m1");
    const button = container.querySelector("button")!;
    act(() => {
      button.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(events.map(e => e.detail)).toEqual([{ runId: "run_abc_1" }]);
    act(() => root.unmount());
    container.remove();
  });
});

describe("artifactEmbed variants", () => {
  function renderArtifact(artifact: StoredArtifact) {
    return createElement(FutureEmbed, {
      reference: ref({ targetType: "artifact" }),
      resolved: { targetType: "artifact", targetId: artifact.id, status: "resolved", data: artifact },
    });
  }

  it("renders the content (no path) variant with summary", () => {
    const html = renderToStaticMarkup(renderArtifact({
      ...storedArtifact,
      path: null,
      content: "body",
      summary: "sum",
    }));
    expect(html).toContain("body");
    expect(html).toContain("sum");
    expect(html).not.toContain("copyPath");
  });

  it("renders title fallbacks", () => {
    const noTitle = renderToStaticMarkup(renderArtifact({ ...storedArtifact, title: "" }));
    expect(noTitle).toContain("art-1");
  });

  it("copy-path and open buttons invoke the clipboard and backend", async () => {
    const exec = vi.fn().mockReturnValue(true);
    Object.defineProperty(document, "execCommand", { value: exec, configurable: true });
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    act(() => {
      root.render(renderArtifact(storedArtifact));
    });
    const buttons = [...container.querySelectorAll("button")];
    // details, copy path, open
    expect(buttons.length).toBe(3);
    const [detailsButton, copyButton, openButton] = buttons as unknown as [Element, Element, Element];
    const events: CustomEvent[] = [];
    window.addEventListener("futureos:inspect-artifact", e => events.push(e as CustomEvent));
    await act(async () => {
      detailsButton.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(events.map(e => e.detail)).toEqual([{ artifactId: "art-1" }]);
    await act(async () => {
      copyButton.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(exec).toHaveBeenCalledWith("copy");
    await act(async () => {
      openButton.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(invokeMock).toHaveBeenCalledWith("open_path", { path: "/w/report.md" });
    act(() => root.unmount());
    container.remove();
  });
});

describe("approval/Review embeds", () => {
  it("renders approval with summary and requested action", () => {
    const html = renderToStaticMarkup(createElement(FutureEmbed, {
      reference: ref({ targetType: "approval" }),
      resolved: {
        targetType: "approval",
        targetId: "ap-1",
        status: "resolved",
        data: { ...storedApproval, summary: "why", requestedAction: "ls -la", title: "" },
      },
    }));
    expect(html).toContain("why");
    expect(html).toContain("ls -la");
    expect(html).toContain("ap-1");
  });

  it("emits open-review from the review embed", () => {
    const events: CustomEvent[] = [];
    window.addEventListener("futureos:open-review", e => events.push(e as CustomEvent));
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    act(() => {
      root.render(createElement(FutureEmbed, {
        reference: ref({ targetType: "review" }),
        resolved: {
          targetType: "review",
          targetId: "rev-1",
          status: "resolved",
          data: { ...storedReview, summary: "s", title: "" },
        },
      }));
    });
    expect(container.innerHTML).toContain("rev-1");
    const button = container.querySelector("button")!;
    act(() => {
      button.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(events.map(e => e.detail)).toEqual([{ reviewId: "rev-1" }]);
    act(() => root.unmount());
    container.remove();
  });
});
