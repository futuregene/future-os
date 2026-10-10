// @vitest-environment jsdom
import type { DirEntry } from "../../integrations/storage/files";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { emitFutureEvent } from "../../lib/futureEvents";
import { FileTreePanel } from "./FileTreePanel";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const listDirectory = vi.fn<(path: string) => Promise<DirEntry[]>>();
const openPath = vi.fn<(path: string) => Promise<void>>();

vi.mock("../../integrations/storage/files", () => ({
  listDirectory: (path: string) => listDirectory(path),
  openPath: (path: string) => openPath(path),
}));

vi.mock("../filepreview/FilePreviewOverlay", () => ({
  FilePreviewOverlay: (props: { kind: string; name: string; onClose?: () => void; onOpenExternal?: () => void; path: string }) =>
    createElement("div", {
      "data-kind": props.kind,
      "data-name": props.name,
      "data-path": props.path,
      "data-testid": "overlay",
    }, [
      createElement("button", { "data-testid": "overlay-close", "key": "close", "onClick": () => props.onClose?.(), "type": "button" }, "overlay-close"),
      createElement("button", { "data-testid": "overlay-external", "key": "external", "onClick": () => props.onOpenExternal?.(), "type": "button" }, "overlay-external"),
    ]),
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

let rootSeq = 0;
function freshRoot(label: string) {
  rootSeq += 1;
  return `/ws/${label}-${rootSeq}`;
}

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

async function mount(props: { isWorkspace?: boolean; rootPath: string | null }) {
  const full = { isWorkspace: true, ...props };
  await act(async () => {
    root.render(createElement(FileTreePanel, full));
  });
  await flush();
}

async function flush(times = 4) {
  await act(async () => {
    for (let index = 0; index < times; index += 1)
      await Promise.resolve();
  });
}

function buttonByText(text: string): HTMLButtonElement | undefined {
  return [...container.querySelectorAll<HTMLButtonElement>("button")]
    .find(button => (button.textContent ?? "").trim() === text);
}

function checkbox(): HTMLInputElement {
  return container.querySelector<HTMLInputElement>("input[type='checkbox']")!;
}

function rowNames(): string[] {
  return [...container.querySelectorAll("li > div > button > span.truncate")].map(span => span.textContent ?? "");
}

function rowButton(name: string): HTMLButtonElement | undefined {
  return [...container.querySelectorAll<HTMLButtonElement>("li > div > button")]
    .find(button => button.textContent?.includes(name));
}

function menuItem(label: string): HTMLButtonElement | undefined {
  const panels = container.querySelectorAll<HTMLElement>("div.z-50");
  const panel = panels[panels.length - 1];
  return [...(panel?.querySelectorAll<HTMLButtonElement>("button") ?? [])]
    .find(button => (button.textContent ?? "").trim() === label);
}

async function openRowMenu(name: string) {
  await act(async () => {
    rowButton(name)?.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, clientX: 20, clientY: 30 }));
  });
}

beforeEach(() => {
  listDirectory.mockReset();
  openPath.mockReset();
  listDirectory.mockResolvedValue([entry("readme.md"), entry("src", true)]);
  openPath.mockResolvedValue(undefined);
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

describe("fileTreePanel states", () => {
  it("holds the loading line back briefly so a fast read never flashes it", async () => {
    vi.useFakeTimers();
    const rootPath = freshRoot("loading");
    listDirectory.mockReturnValue(new Promise<DirEntry[]>(() => {}));
    await mount({ rootPath });

    // Nothing on the first frames…
    expect(container.textContent).not.toContain("Loading…");
    await act(async () => {
      await vi.advanceTimersByTimeAsync(199);
    });
    expect(container.textContent).not.toContain("Loading…");

    // …then the delayed indicator for a genuinely slow read.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(container.textContent).toContain("Loading…");
  });

  it("renders the entries and never shows the delayed indicator for a warm root", async () => {
    vi.useFakeTimers();
    const rootPath = freshRoot("warm");
    await mount({ rootPath });
    expect(rowNames()).toEqual(["readme.md", "src"]);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(500);
    });
    expect(container.textContent).not.toContain("Loading…");
  });

  it("reports a root that could not be read", async () => {
    listDirectory.mockRejectedValue(new Error("EACCES"));
    await mount({ rootPath: freshRoot("error") });
    expect(container.textContent).toContain("Couldn't read this folder");
  });

  it("reports an empty root", async () => {
    listDirectory.mockResolvedValue([]);
    await mount({ rootPath: freshRoot("empty") });
    expect(container.textContent).toContain("This folder is empty");
  });

  it("says nothing at all when there is no workspace root", async () => {
    await mount({ rootPath: null });
    expect(listDirectory).not.toHaveBeenCalled();
    expect(container.textContent).not.toContain("Couldn't read this folder");
    expect(buttonByText("Open Folder")?.disabled).toBe(true);
    expect(buttonByText("Refresh")?.disabled).toBe(true);
  });
});

describe("fileTreePanel hidden files", () => {
  it("defaults to showing dotfiles in a workspace and hiding them in a chat", async () => {
    listDirectory.mockResolvedValue([entry(".env"), entry("src", true)]);
    await mount({ isWorkspace: true, rootPath: freshRoot("ws") });
    expect(checkbox().checked).toBe(true);
    expect(rowNames()).toEqual([".env", "src"]);

    act(() => root.unmount());
    root = createRoot(container);
    await mount({ isWorkspace: false, rootPath: freshRoot("chat") });
    expect(checkbox().checked).toBe(false);
    expect(rowNames()).toEqual(["src"]);
  });

  it("filters dotfiles on toggle without refetching", async () => {
    listDirectory.mockResolvedValue([entry(".env"), entry("src", true)]);
    await mount({ isWorkspace: true, rootPath: freshRoot("toggle") });
    const callsBefore = listDirectory.mock.calls.length;

    await act(async () => {
      checkbox().click();
    });
    expect(checkbox().checked).toBe(false);
    expect(rowNames()).toEqual(["src"]);
    expect(listDirectory.mock.calls.length).toBe(callsBefore);
  });

  it("re-applies the mode default when the root changes", async () => {
    listDirectory.mockResolvedValue([entry(".env"), entry("src", true)]);
    await mount({ isWorkspace: true, rootPath: freshRoot("switch-a") });
    await act(async () => {
      checkbox().click();
    });
    expect(checkbox().checked).toBe(false);

    const next = freshRoot("switch-b");
    await act(async () => {
      root.render(createElement(FileTreePanel, { isWorkspace: true, rootPath: next }));
    });
    await flush();
    // The manual toggle only sticks within one root.
    expect(checkbox().checked).toBe(true);
    expect(listDirectory).toHaveBeenCalledWith(next);
  });
});

describe("fileTreePanel refresh", () => {
  it("opens the workspace folder with the OS handler", async () => {
    const rootPath = freshRoot("open");
    await mount({ rootPath });
    await act(async () => {
      buttonByText("Open Folder")?.click();
    });
    await flush();
    expect(openPath).toHaveBeenCalledWith(rootPath);
  });

  it("swallows a failing OS open", async () => {
    openPath.mockRejectedValue(new Error("no handler"));
    await mount({ rootPath: freshRoot("open-fail") });
    await act(async () => {
      buttonByText("Open Folder")?.click();
    });
    await flush();
    expect(container.textContent).not.toContain("no handler");
  });

  it("re-reads the tree from the Refresh button and shows the spin while it runs", async () => {
    const rootPath = freshRoot("manual-refresh");
    let resolveRefresh!: (entries: DirEntry[]) => void;
    await mount({ rootPath });
    const initialCalls = listDirectory.mock.calls.length;
    listDirectory.mockImplementation(() => new Promise<DirEntry[]>((resolve) => {
      resolveRefresh = resolve;
    }));

    await act(async () => {
      buttonByText("Refresh")?.click();
    });
    const button = buttonByText("Refresh")!;
    expect(button.disabled).toBe(true);
    expect(button.querySelector("svg")?.getAttribute("class")).toContain("animate-spin");

    await act(async () => {
      resolveRefresh([entry("after-refresh.txt")]);
    });
    await flush();
    expect(listDirectory.mock.calls.length).toBe(initialCalls + 1);
    expect(rowNames()).toEqual(["after-refresh.txt"]);
    expect(buttonByText("Refresh")?.disabled).toBe(false);
  });

  it("coalesces a burst of file-tree-refresh events into one read", async () => {
    vi.useFakeTimers();
    const rootPath = freshRoot("coalesce");
    await mount({ rootPath });
    const initialCalls = listDirectory.mock.calls.length;

    await act(async () => {
      emitFutureEvent("file-tree-refresh", undefined);
      emitFutureEvent("file-tree-refresh", undefined);
      emitFutureEvent("file-tree-refresh", undefined);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1999);
    });
    expect(listDirectory.mock.calls.length).toBe(initialCalls);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(listDirectory.mock.calls.length).toBe(initialCalls + 1);

    // A later burst schedules another read.
    await act(async () => {
      emitFutureEvent("file-tree-refresh", undefined);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2000);
    });
    expect(listDirectory.mock.calls.length).toBe(initialCalls + 2);
  });

  it("cancels a pending refresh when the panel unmounts", async () => {
    vi.useFakeTimers();
    await mount({ rootPath: freshRoot("unmount") });
    const initialCalls = listDirectory.mock.calls.length;

    await act(async () => {
      emitFutureEvent("file-tree-refresh", undefined);
    });
    act(() => root.unmount());
    root = createRoot(container);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5000);
    });
    expect(listDirectory.mock.calls.length).toBe(initialCalls);
  });
});

describe("fileTreePanel file activation", () => {
  it("previews a previewable file in-app", async () => {
    const rootPath = freshRoot("preview");
    listDirectory.mockResolvedValue([entry("note.md", false, rootPath), entry("image.PNG", false, rootPath), entry("archive.bin", false, rootPath)]);
    await mount({ rootPath });

    await act(async () => {
      rowButton("note.md")?.click();
    });
    let overlay = container.querySelector("[data-testid='overlay']");
    expect(overlay?.getAttribute("data-kind")).toBe("markdown");
    expect(overlay?.getAttribute("data-path")).toBe(`${rootPath}/note.md`);
    expect(openPath).not.toHaveBeenCalled();

    await act(async () => {
      container.querySelector<HTMLButtonElement>("[data-testid='overlay-close']")?.click();
    });
    expect(container.querySelector("[data-testid='overlay']")).toBeNull();

    // An image takes the same in-app path…
    await act(async () => {
      rowButton("image.PNG")?.click();
    });
    overlay = container.querySelector("[data-testid='overlay']");
    expect(overlay?.getAttribute("data-kind")).toBe("image");

    // …while an un-previewable file goes to the OS.
    await act(async () => {
      rowButton("archive.bin")?.click();
    });
    await flush();
    expect(openPath).toHaveBeenCalledWith(`${rootPath}/archive.bin`);
  });

  it("opens the original file from the overlay", async () => {
    const rootPath = freshRoot("overlay-external");
    listDirectory.mockResolvedValue([entry("note.md", false, rootPath)]);
    await mount({ rootPath });
    await act(async () => {
      rowButton("note.md")?.click();
    });
    await act(async () => {
      container.querySelector<HTMLButtonElement>("[data-testid='overlay-external']")?.click();
    });
    await flush();
    expect(openPath).toHaveBeenCalledWith(`${rootPath}/note.md`);
  });

  it("toasts when the OS refuses to open a file", async () => {
    const rootPath = freshRoot("open-refused");
    listDirectory.mockResolvedValue([entry("archive.bin", false, rootPath)]);
    openPath.mockRejectedValue(new Error("no handler"));
    const toasts: Array<{ message: string; tone?: string }> = [];
    const listener = (event: Event) => {
      toasts.push((event as CustomEvent<{ message: string; tone?: string }>).detail);
    };
    window.addEventListener("futureos:toast", listener);
    try {
      await mount({ rootPath });
      await act(async () => {
        rowButton("archive.bin")?.click();
      });
      await flush();
      expect(toasts).toEqual([{ message: "Couldn't open this file", tone: "error" }]);
    }
    finally {
      window.removeEventListener("futureos:toast", listener);
    }
  });
});

describe("fileTreePanel context menu", () => {
  it("offers only Open for a directory", async () => {
    const rootPath = freshRoot("menu-dir");
    listDirectory.mockResolvedValue([entry("src", true, rootPath)]);
    await mount({ rootPath });
    await openRowMenu("src");

    expect(menuItem("Open")).toBeTruthy();
    expect(menuItem("Attach to context")).toBeUndefined();
    expect(menuItem("Preview")).toBeUndefined();

    await act(async () => {
      menuItem("Open")?.click();
    });
    await flush();
    expect(openPath).toHaveBeenCalledWith(`${rootPath}/src`);
  });

  it("offers attach/preview/open for a previewable file", async () => {
    const rootPath = freshRoot("menu-file");
    listDirectory.mockResolvedValue([entry("note.md", false, rootPath)]);
    const attached: Array<{ name: string; path: string }> = [];
    const listener = (event: Event) => {
      attached.push((event as CustomEvent<{ name: string; path: string }>).detail);
    };
    window.addEventListener("futureos:attach-file-to-context", listener);
    try {
      await mount({ rootPath });
      await openRowMenu("note.md");

      expect(menuItem("Attach to context")).toBeTruthy();
      expect(menuItem("Preview")).toBeTruthy();
      await act(async () => {
        menuItem("Attach to context")?.click();
      });
      // The pill wants a workspace-relative POSIX path.
      expect(attached).toEqual([{ name: "note.md", path: "note.md" }]);
    }
    finally {
      window.removeEventListener("futureos:attach-file-to-context", listener);
    }
  });

  it("omits Preview for a file it cannot render but still offers attach and open", async () => {
    const rootPath = freshRoot("menu-binary");
    listDirectory.mockResolvedValue([entry("archive.bin", false, rootPath)]);
    await mount({ rootPath });
    await openRowMenu("archive.bin");

    expect(menuItem("Attach to context")).toBeTruthy();
    expect(menuItem("Preview")).toBeUndefined();
    await act(async () => {
      menuItem("Preview")?.click();
    });
    expect(container.querySelector("[data-testid='overlay']")).toBeNull();

    await act(async () => {
      menuItem("Open")?.click();
    });
    await flush();
    expect(openPath).toHaveBeenCalledWith(`${rootPath}/archive.bin`);
  });

  it("opens the preview from the menu", async () => {
    const rootPath = freshRoot("menu-preview");
    listDirectory.mockResolvedValue([entry("note.md", false, rootPath)]);
    await mount({ rootPath });
    await openRowMenu("note.md");
    await act(async () => {
      menuItem("Preview")?.click();
    });
    expect(container.querySelector("[data-testid='overlay']")?.getAttribute("data-kind")).toBe("markdown");
  });
});

describe("fileTreePanel boundaries", () => {
  it("renders a large directory with CJK and long names", async () => {
    const rootPath = freshRoot("large");
    const entries = [
      ...Array.from({ length: 120 }, (_, index) => entry(`file-${index}.ts`, false, rootPath)),
      entry("工作区-テキスト.md", false, rootPath),
      entry(`${"长".repeat(120)}.rs`, false, rootPath),
    ];
    listDirectory.mockResolvedValue(entries);
    await mount({ rootPath });

    expect(container.querySelectorAll("li")).toHaveLength(122);
    const names = rowNames();
    expect(names[names.length - 2]).toBe("工作区-テキスト.md");
    expect(names[names.length - 1]?.length).toBe(123);
    // A DOM-heavy boundary case: give it room to finish on a loaded machine.
  }, 30_000);
});
