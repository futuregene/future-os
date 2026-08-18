import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import { flushAsync } from "../../test/renderHook";

import { MarkdownContent } from "./MarkdownContent";

const resolveReferencesMock = vi.fn<(w: string, refs: unknown[]) => Promise<Array<Record<string, unknown>>>>(
  () => Promise.resolve([]),
);
const resolvePreviewLinkMock = vi.fn<(base: string, target: string) => Promise<{ path: string; name: string }>>();
const readFileBase64Mock = vi.fn<(path: string) => Promise<string>>(() => Promise.resolve("QUJD"));

vi.mock("../../integrations/storage/markdownReferences", () => ({
  resolveMarkdownReferences: (w: string, refs: unknown[]) => resolveReferencesMock(w, refs),
}));

vi.mock("../../integrations/storage/files", () => ({
  openPath: () => Promise.resolve(),
  openExternalUrl: () => Promise.resolve(),
  readFileBase64: ({ path }: { path: string }) => readFileBase64Mock(path),
  readTextFilePreview: () => Promise.resolve({ content: "", size: 0, truncated: false }),
  resolvePreviewLinkPath: (base: string, target: string) => resolvePreviewLinkMock(base, target),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.resolve(() => {}),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/** Flush the reference store's 0ms batching timer + the resolve promise chain. */
async function flushStore() {
  await act(async () => {
    await new Promise(resolve => setTimeout(resolve, 10));
  });
}

function mount(node: React.ReactElement) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => {
    root.render(node);
  });
  return {
    container,
    cleanup: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

describe("markdownContent block rendering", () => {
  it("renders headings, lists, blockquotes, tables and breaks", () => {
    const html = renderToStaticMarkup(createElement(MarkdownContent, {
      content: [
        "## Second",
        "",
        "### Third",
        "",
        "1. one",
        "2. two",
        "",
        "> quoted",
        "",
        "| a | b |",
        "| :--- | ---: |",
        "| 1 | 2 |",
        "",
        "---",
        "",
        "hard  ",
        "break",
      ].join("\n"),
    }));
    expect(html).toContain("<h2");
    expect(html).toContain("<h3");
    expect(html).toContain("<ol");
    expect(html).toContain("<blockquote");
    expect(html).toContain("text-align:left");
    expect(html).toContain("text-align:right");
    expect(html).toContain("<hr");
    expect(html).toContain("<br");
  });

  it("renders task lists and loose lists with nested blocks", () => {
    const html = renderToStaticMarkup(createElement(MarkdownContent, {
      content: "- [ ] todo\n- [x] done\n- loose\n\n  nested paragraph",
    }));
    expect(html).toContain("type=\"checkbox\"");
    expect(html).toContain("nested paragraph");
  });

  it("renders italic and strong inline spans", () => {
    const html = renderToStaticMarkup(createElement(MarkdownContent, {
      content: "an *italic* and a **bold** span",
    }));
    expect(html).toContain("<em class=\"italic\">italic</em>");
    expect(html).toContain("<strong");
  });

  it("renders inline code, images and links", () => {
    const html = renderToStaticMarkup(createElement(MarkdownContent, {
      content: "some `code` and ![alt](https://x/y.png) and [site](https://example.com)",
    }));
    expect(html).toContain("<code");
    expect(html).toContain("<img");
    expect(html).toContain("href=\"https://example.com\"");
  });

  it("renders an inline file reference as a pending chip under SSR", () => {
    const html = renderToStaticMarkup(createElement(MarkdownContent, {
      content: "[report](/abs/report.md)",
      workspaceId: "w1",
    }));
    expect(html).toContain("report");
  });
});

describe("markdownContent reference resolution", () => {
  it("resolves a file reference into a FileLink once the store resolves", async () => {
    resolveReferencesMock.mockResolvedValue([{
      targetType: "file",
      targetId: "/abs/report.md",
      status: "resolved",
      data: { path: "/abs/report.md", name: "report.md", insideWorkspace: false },
    }]);
    const { container, cleanup } = mount(createElement(MarkdownContent, {
      content: "[report](/abs/report.md)",
      workspaceId: "w-resolve",
    }));
    await flushStore();
    await flushAsync();
    const anchor = container.querySelector("a");
    expect(anchor?.getAttribute("href")).toBe("file:///abs/report.md");
    cleanup();
  });

  it("resolves a futureos-file block embed into a FileLink", async () => {
    resolveReferencesMock.mockResolvedValue([{
      targetType: "file",
      targetId: "/abs/embed.md",
      status: "resolved",
      data: { path: "/abs/embed.md", name: "embed.md", insideWorkspace: false },
    }]);
    const { container, cleanup } = mount(createElement(MarkdownContent, {
      content: "```futureos-file\nid: /abs/embed.md\n```",
      workspaceId: "w-embed",
    }));
    await flushStore();
    await flushAsync();
    expect(container.querySelector("a")?.getAttribute("href")).toBe("file:///abs/embed.md");
    cleanup();
  });

  it("resolves preview-mode file links against the previewed file", async () => {
    resolvePreviewLinkMock.mockResolvedValue({ path: "/w/dir/pic.md", name: "pic.md" });
    const { container, cleanup } = mount(createElement(MarkdownContent, {
      content: "[pic](dir/pic.md)",
      basePath: "/w/doc.md",
    }));
    await flushAsync();
    const anchor = container.querySelector("a");
    expect(anchor?.getAttribute("href")).toBe("file:///w/dir/pic.md");
    cleanup();
  });

  it("renders a workspace-relative local Markdown image", async () => {
    resolveReferencesMock.mockResolvedValue([{
      targetType: "file",
      targetId: "assets/pic.png",
      status: "resolved",
      data: { path: "/w/assets/pic.png", name: "pic.png", insideWorkspace: true, relativePath: "assets/pic.png" },
    }]);
    const { container, cleanup } = mount(createElement(MarkdownContent, {
      content: "![diagram](assets/pic.png)",
      workspaceId: "w-local-image",
    }));
    await flushStore();
    await flushAsync();
    expect(container.querySelector("img")?.getAttribute("src")).toBe("data:image/png;base64,QUJD");
    expect(readFileBase64Mock).toHaveBeenCalledWith("/w/assets/pic.png");
    cleanup();
  });

  it("renders a local Markdown image relative to the previewed file", async () => {
    resolvePreviewLinkMock.mockResolvedValue({ path: "/w/assets/pic.png", name: "pic.png" });
    const { container, cleanup } = mount(createElement(MarkdownContent, {
      content: "![diagram](assets/pic.png)",
      basePath: "/w/doc.md",
    }));
    await flushAsync();
    await flushAsync();
    expect(container.querySelector("img")?.getAttribute("src")).toBe("data:image/png;base64,QUJD");
    cleanup();
  });

  it("shows a pending placeholder while the preview link resolves", () => {
    resolvePreviewLinkMock.mockReturnValue(new Promise(() => {}));
    const { container, cleanup } = mount(createElement(MarkdownContent, {
      content: "[pic](dir/pic.md)",
      basePath: "/w/doc.md",
    }));
    expect(container.textContent).toContain("pic");
    expect(container.querySelector("a")).toBeNull();
    cleanup();
  });
});
