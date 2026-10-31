// @vitest-environment jsdom
import type { DirEntry } from "../../integrations/storage/files";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { FileTreePanel } from "./FileTreePanel";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const listDirectory = vi.fn<(path: string) => Promise<DirEntry[]>>();

vi.mock("../../integrations/storage/files", () => ({
  listDirectory: (path: string) => listDirectory(path),
  openPath: () => Promise.resolve(),
}));

vi.mock("../filepreview/FilePreviewOverlay", () => ({
  FilePreviewOverlay: () => null,
}));

class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
(globalThis as Record<string, unknown>).ResizeObserver = ResizeObserverStub;

function entry(name: string, isDir = false, root = "/ws"): DirEntry {
  return { isDir, modified: 1, name, path: `${root}/${name}`, size: isDir ? 0 : 10 };
}

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

/**
 * The panel's closing edge.
 *
 * The Files list is the last thing in a full-height column: without a rule of its
 * own a long tree ends at the window's bottom edge, the last row sliced through
 * its glyphs and nothing under it — which is exactly how the README's hero frame
 * looked. The rule therefore belongs to a strip *below* the scroll viewport, and
 * these assertions are about that relationship rather than about pixels (jsdom
 * has no layout; the capture scenario measures the real boxes).
 */
describe("fileTreePanel closing edge", () => {
  beforeEach(() => {
    listDirectory.mockReset();
    listDirectory.mockResolvedValue([entry("README.md"), entry("src", true), entry("Cargo.toml")]);
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  async function mount() {
    await act(async () => {
      root.render(createElement(FileTreePanel, { isWorkspace: true, rootPath: "/ws" }));
    });
    await act(async () => {
      for (let index = 0; index < 4; index += 1)
        await Promise.resolve();
    });
  }

  function viewport(): HTMLElement {
    return container.querySelector<HTMLElement>("div.floating-scrollbar")!;
  }

  function closingEdge(): HTMLElement {
    return container.querySelector<HTMLElement>("[data-testid='file-tree-bottom-edge']")!;
  }

  it("draws a hairline under the viewport, outside the part that scrolls", async () => {
    await mount();
    const edge = closingEdge();
    expect(edge).not.toBeNull();

    // Outside the scroller: a border on the viewport would travel with the
    // content and land on the window's own edge.
    expect(viewport().contains(edge)).toBe(false);
    expect(edge.compareDocumentPosition(viewport()) & Node.DOCUMENT_POSITION_PRECEDING).toBeTruthy();

    // A hairline, not a band: one rule, in the token the panel's other edges use.
    expect(edge.className).toContain("border-t");
    expect(edge.className).toContain("border-line-soft");
    expect(edge.className).toContain("h-1.5");
    expect(edge.className).toContain("shrink-0");

    // The strip keeps its height while the list takes the rest, which is what
    // puts the rule below the rows instead of under them.
    const list = viewport().parentElement!;
    expect(list.className).toContain("flex-1");
    expect(list.className).toContain("flex-col");
    expect(viewport().className).toContain("flex-1");
    expect(viewport().className).toContain("min-h-0");
    expect(list.lastElementChild).toBe(edge);
  });

  it("keeps the rows in the scroller and the edge in the panel", async () => {
    await mount();
    const edge = closingEdge();
    const rows = [...viewport().querySelectorAll("li")];
    expect(rows.length).toBe(3);
    for (const row of rows)
      expect(edge.contains(row)).toBe(false);
    // The strip sits in the panel, not inside the list that scrolls.
    expect(viewport().contains(edge)).toBe(false);
  });
});
