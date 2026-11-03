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
 * The panel's shape: a card, divided.
 *
 * The Files list is a full-height column in the context panel. Left edge-to-edge
 * it read as a list spilling out of the panel — its border ran into the window's
 * bottom edge and nothing closed it. It is a card now (`rounded-lg border
 * border-line`, the composer's and the approval prompt's own treatment), inset
 * from the panel's edges, with the rows divided inside it and the card's rounded
 * border closing the bottom. There is no separate strip under the scroll area:
 * one line only, ever.
 *
 * These assertions are about the DOM relationship — jsdom has no layout, so the
 * pixels (radius visible, nothing clipped, one line at the bottom) are the
 * capture scenario's job.
 */
describe("fileTreePanel is a card", () => {
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

  it("draws the list inside a rounded, bordered card", async () => {
    await mount();
    const card = viewport().parentElement!;
    expect(card.className).toContain("rounded-lg");
    expect(card.className).toContain("border");
    expect(card.className).toContain("border-line");
    // Clipped by the radius, not merely drawn next to it: the rows must not
    // paint over the card's rounded corners.
    expect(card.className).toContain("overflow-hidden");
    // Inset from the panel's edges, so the radius has something to show against.
    expect(card.parentElement?.className).toContain("px-2");
    expect(card.parentElement?.className).toContain("pb-2");
  });

  it("closes on the card's own border, with no second rule under the list", async () => {
    await mount();
    // The strip that used to sit below the viewport is gone: the card's border is
    // the only line at the bottom of the panel.
    expect(container.querySelector("[data-testid='file-tree-bottom-edge']")).toBeNull();
    const card = viewport().parentElement!;
    expect(card.querySelector("[data-testid='file-tree-bottom-edge']")).toBeNull();
  });

  it("divides the rows inside the card and leaves the last one unruled", async () => {
    await mount();
    const rows = [...viewport().querySelectorAll<HTMLElement>("li > div")];
    expect(rows.length).toBe(3);
    for (const row of rows.slice(1)) {
      expect(row.className).toContain("border-t");
      expect(row.className).toContain("border-line-soft/50");
      // A rule under the last row would be the card's second bottom line.
      expect(row.className).not.toContain("border-b");
    }
    // The card's top border stands in for the first row's rule.
    expect(viewport().className).toContain("[&>ul>li:first-child>div]:border-t-0");
  });
});
