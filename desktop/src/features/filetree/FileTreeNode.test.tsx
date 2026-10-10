// @vitest-environment jsdom
import type { DirEntry } from "../../integrations/storage/files";
import type { FileTree } from "./useFileTree";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { FileTreeNode } from "./FileTreeNode";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function entry(name: string, isDir = false, parent = "/ws"): DirEntry {
  return { isDir, modified: 1, name, path: `${parent}/${name}`, size: isDir ? 0 : 10 };
}

function tree(overrides: Partial<FileTree> = {}): FileTree {
  return {
    childrenOf: () => null,
    isErrored: () => false,
    isExpanded: () => false,
    isLoading: () => false,
    refresh: async () => {},
    reload: vi.fn(),
    rootEntries: null,
    rootErrored: false,
    rootLoading: false,
    toggle: vi.fn(),
    ...overrides,
  };
}

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

async function mount(props: Parameters<typeof FileTreeNode>[0]) {
  await act(async () => {
    root.render(createElement(FileTreeNode, props));
  });
}

function rowButton(): HTMLButtonElement {
  return container.querySelector<HTMLButtonElement>("li > div > button")!;
}

function actionsButton(): HTMLButtonElement | null {
  return container.querySelector<HTMLButtonElement>("button[aria-label^='Actions for']");
}

function listItems(): string[] {
  return [...container.querySelectorAll("li")].map(item => item.textContent ?? "");
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

describe("fileTreeNode rows", () => {
  it("shows a collapsed chevron for a directory and expands it on click", async () => {
    const toggle = vi.fn();
    await mount({ depth: 0, entry: entry("src", true), tree: tree({ toggle }) });
    expect(container.querySelector("svg.lucide-chevron-right")).toBeTruthy();
    expect(container.querySelector("svg.lucide-chevron-down")).toBeNull();

    await act(async () => {
      rowButton().click();
    });
    expect(toggle).toHaveBeenCalledWith("/ws/src");
    // A directory click never goes to the file opener.
  });

  it("shows a file's type glyph and hands the click to the opener", async () => {
    const onOpenFile = vi.fn();
    const toggle = vi.fn();
    await mount({ depth: 0, entry: entry("main.rs"), onOpenFile, tree: tree({ toggle }) });
    expect(container.querySelector("svg.lucide-file-braces")).toBeTruthy();

    await act(async () => {
      rowButton().click();
    });
    expect(onOpenFile).toHaveBeenCalledWith(entry("main.rs"));
    expect(toggle).not.toHaveBeenCalled();
  });

  it("opens a directory with its toggle even without a file opener", async () => {
    const toggle = vi.fn();
    await mount({ depth: 0, entry: entry("docs", true), tree: tree({ toggle }) });
    await act(async () => {
      rowButton().click();
    });
    expect(toggle).toHaveBeenCalledWith("/ws/docs");
  });

  it("indents each level by one step", async () => {
    await mount({ depth: 0, entry: entry("a.txt"), tree: tree() });
    const depthZero = container.querySelector<HTMLElement>("li > div")!.style.paddingLeft;
    expect(depthZero).toBe("10px");

    act(() => root.unmount());
    root = createRoot(container);
    await mount({ depth: 3, entry: entry("a.txt"), tree: tree() });
    const depthThree = container.querySelector<HTMLElement>("li > div")!.style.paddingLeft;
    expect(depthThree).toBe("46px");
    expect(depthThree).not.toBe(depthZero);
  });
});

describe("fileTreeNode actions trigger", () => {
  it("is absent when no context-menu handler is wired", async () => {
    await mount({ depth: 0, entry: entry("a.txt"), tree: tree() });
    expect(actionsButton()).toBeNull();
  });

  it("calls the context-menu handler from both the row and the trigger, once each", async () => {
    const onContextMenu = vi.fn();
    await mount({ depth: 0, entry: entry("a.txt"), onContextMenu, tree: tree() });

    const trigger = actionsButton()!;
    expect(trigger.getAttribute("aria-label")).toBe("Actions for a.txt");

    await act(async () => {
      trigger.click();
    });
    expect(onContextMenu).toHaveBeenCalledTimes(1);
    // The trigger stops propagation, so the row handler does not also fire.
    expect(onContextMenu.mock.calls[0]![0]).toMatchObject({ path: "/ws/a.txt" });

    await act(async () => {
      rowButton().dispatchEvent(new MouseEvent("contextmenu", { bubbles: true }));
    });
    expect(onContextMenu).toHaveBeenCalledTimes(2);
  });

  it("keeps the trigger visible for the row whose menu is open", async () => {
    await mount({ activePath: "/ws/a.txt", depth: 0, entry: entry("a.txt"), onContextMenu: vi.fn(), tree: tree() });
    expect(actionsButton()?.classList.contains("opacity-100")).toBe(true);

    act(() => root.unmount());
    root = createRoot(container);
    await mount({ activePath: "/ws/other.txt", depth: 0, entry: entry("a.txt"), onContextMenu: vi.fn(), tree: tree() });
    expect(actionsButton()?.classList.contains("opacity-100")).toBe(false);
    // Otherwise it only fades in with hover.
    expect(actionsButton()?.classList.contains("opacity-0")).toBe(true);
  });
});

describe("fileTreeNode children", () => {
  it("renders nothing under a collapsed directory", async () => {
    await mount({
      depth: 0,
      entry: entry("src", true),
      tree: tree({ childrenOf: () => [entry("main.rs", false, "/ws/src")] }),
    });
    expect(container.querySelector("ul")).toBeNull();
  });

  it("renders nested rows when expanded and recurses one level deeper", async () => {
    const children = [entry("main.rs", false, "/ws/src"), entry("nested", true, "/ws/src")];
    await mount({
      depth: 0,
      entry: entry("src", true),
      tree: tree({
        childrenOf: path => (path === "/ws/src"
          ? children
          : path === "/ws/src/nested"
            ? [entry("deep.txt", false, "/ws/src/nested")]
            : null),
        isExpanded: path => path === "/ws/src" || path === "/ws/src/nested",
      }),
    });
    expect(container.querySelectorAll("li")).toHaveLength(4);
    expect(listItems()[0]).toContain("src");
    expect(listItems()[1]).toContain("main.rs");
    expect(listItems()[2]).toContain("nested");
    expect(listItems()[3]).toContain("deep.txt");
    // Depth 2 for the grandchild.
    const rows = [...container.querySelectorAll<HTMLElement>("li > div")];
    expect(rows.map(row => row.style.paddingLeft)).toEqual(["10px", "22px", "22px", "34px"]);
  });

  it("shows a loading line while an expanded directory is in flight", async () => {
    await mount({
      depth: 0,
      entry: entry("src", true),
      tree: tree({ childrenOf: () => null, isExpanded: () => true, isLoading: () => true }),
    });
    const text = container.textContent ?? "";
    expect(text).toContain("Loading…");
    // Indented one level deeper than the row.
    expect(container.querySelectorAll("li")).toHaveLength(2);
  });

  it("shows nothing (not a spinner) for an expanded, unloaded, idle directory", async () => {
    await mount({
      depth: 0,
      entry: entry("src", true),
      tree: tree({ childrenOf: () => null, isExpanded: () => true }),
    });
    expect(container.textContent ?? "").not.toContain("Loading…");
    expect(container.querySelectorAll("li")).toHaveLength(1);
  });

  it("says an expanded directory is empty when the read returned no children", async () => {
    await mount({
      depth: 0,
      entry: entry("src", true),
      tree: tree({ childrenOf: () => [], isExpanded: () => true }),
    });
    expect(container.textContent ?? "").toContain("This folder is empty");
  });

  it("offers a retry for a directory whose read failed", async () => {
    const reload = vi.fn();
    await mount({
      depth: 0,
      entry: entry("src", true),
      tree: tree({ isErrored: () => true, isExpanded: () => true, reload }),
    });
    expect(container.textContent ?? "").toContain("Couldn't read this folder");
    expect(container.textContent ?? "").toContain("Retry");

    const retry = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => (button.textContent ?? "").includes("Retry"));
    await act(async () => {
      retry?.click();
    });
    expect(reload).toHaveBeenCalledWith("/ws/src");
    // The error state wins over the cached children / loading line.
    expect(container.textContent ?? "").not.toContain("This folder is empty");
  });

  it("keeps a 300-entry directory as one row per entry", async () => {
    const children = Array.from({ length: 300 }, (_, index) => entry(`file-${index}.ts`, false, "/ws/big"));
    await mount({
      depth: 0,
      entry: entry("big", true),
      tree: tree({ childrenOf: () => children, isExpanded: () => true }),
    });
    expect(container.querySelectorAll("li")).toHaveLength(301);
    const items = listItems();
    expect(items[items.length - 1]).toContain("file-299.ts");
    // A DOM-heavy boundary case: give it room to finish on a loaded machine.
  }, 30_000);

  it("renders CJK and very long names intact", async () => {
    const children = [entry("工作区-テキスト.md", false, "/ws/深い"), entry(`${"x".repeat(300)}.rs`, false, "/ws/深い")];
    await mount({
      depth: 0,
      entry: entry("深い", true),
      tree: tree({ childrenOf: () => children, isExpanded: () => true }),
    });
    expect(container.textContent).toContain("工作区-テキスト.md");
    expect(container.textContent).toContain(`${"x".repeat(300)}.rs`);
  });
});
