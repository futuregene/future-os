// @vitest-environment jsdom
import type { ReactElement } from "react";
import type { StoredThread } from "../../../integrations/storage/threadStore";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "../../../test/renderHook";
import { useCollapsedWorkspaces } from "./useCollapsedWorkspaces";
import { useDropUpMenu } from "./useDropUpMenu";
import { useLeftPanelWidth } from "./useLeftPanelWidth";
import { useRailSelection } from "./useRailSelection";
import { useWindowWidth } from "./useWindowWidth";

function setViewportWidth(px: number) {
  Object.defineProperty(window, "innerWidth", { configurable: true, value: px, writable: true });
}

/**
 * A window-level pointer event with the two fields the drag maths reads.
 * jsdom's `PointerEvent` init dict does not reliably carry `pointerId`, so both
 * are defined explicitly — the drag handlers branch on `pointerId` identity.
 */
function pointerEvent(type: string, pointerId: number, clientX = 0) {
  const event = new Event(type);
  Object.defineProperty(event, "pointerId", { value: pointerId });
  Object.defineProperty(event, "clientX", { value: clientX });
  return event;
}

function mount(node: ReactElement) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => root.render(node));
  return {
    container,
    rerender: (next: ReactElement) => act(() => root.render(next)),
    unmount: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

function thread(id: string, overrides: Partial<StoredThread> = {}): StoredThread {
  return {
    id,
    agentSessionId: id,
    title: id,
    mode: "chat",
    workspaceId: `w-${id}`,
    status: "active",
    pinned: false,
    readonly: false,
    createdAt: 0,
    updatedAt: 0,
    ...overrides,
  };
}

describe("useWindowWidth", () => {
  afterEach(() => setViewportWidth(1024));

  it("tracks window.resize and unsubscribes on unmount", () => {
    setViewportWidth(1024);
    const remove = vi.spyOn(window, "removeEventListener");
    const harness = renderHook(() => useWindowWidth());
    expect(harness.current).toBe(1024);

    setViewportWidth(1440);
    act(() => {
      window.dispatchEvent(new Event("resize"));
    });
    expect(harness.current).toBe(1440);

    harness.unmount();
    expect(remove).toHaveBeenCalledWith("resize", expect.any(Function));
    remove.mockRestore();
  });
});

describe("useDropUpMenu", () => {
  function Probe({ open }: { open: boolean }) {
    const { dropUp, menuRef } = useDropUpMenu(open);
    return createElement("div", { "data-dropup": String(dropUp), "ref": menuRef });
  }

  /**
   * Mount a probe inside an ancestor that clips its overflow, with both the
   * clip boundary and the menu's own bottom under explicit control.
   */
  function clippedMount(clipBottom: number, menuBottom: number) {
    const outer = document.createElement("div");
    outer.style.overflowY = "hidden";
    outer.getBoundingClientRect = () => ({ bottom: clipBottom } as DOMRect);
    document.body.appendChild(outer);

    const container = document.createElement("div");
    outer.appendChild(container);
    const root = createRoot(container);
    act(() => root.render(createElement(Probe, { open: false })));
    const menu = container.firstElementChild!;
    menu.getBoundingClientRect = () => ({ bottom: menuBottom } as DOMRect);
    const rerender = (open: boolean) => act(() => root.render(createElement(Probe, { open })));
    return {
      attribute: () => menu.getAttribute("data-dropup"),
      rerender,
      unmount: () => {
        act(() => root.unmount());
        outer.remove();
      },
    };
  }

  it("flips up when the menu would spill past its clipping ancestor", () => {
    const view = clippedMount(100, 200);
    expect(view.attribute()).toBe("false");
    view.rerender(true);
    expect(view.attribute()).toBe("true");
    view.unmount();
  });

  it("stays open downward when the menu fits inside the clip", () => {
    const view = clippedMount(300, 120);
    view.rerender(true);
    expect(view.attribute()).toBe("false");
    view.unmount();
  });

  it("does nothing while closed, even with a ref attached", () => {
    const view = clippedMount(10, 900);
    view.rerender(false);
    expect(view.attribute()).toBe("false");
    view.unmount();
  });

  it("skips measurement when no element is attached to the ref", () => {
    function NullProbe() {
      const { dropUp } = useDropUpMenu(true);
      return createElement("div", { "data-dropup": String(dropUp) });
    }
    const view = mount(createElement(NullProbe));
    expect(view.container.firstElementChild!.getAttribute("data-dropup")).toBe("false");
    view.unmount();
  });
});

describe("useCollapsedWorkspaces", () => {
  const KEY = "future.collapsedWorkspaces";

  beforeEach(() => localStorage.clear());
  afterEach(() => localStorage.clear());

  it("starts expanded when the stored value is not an array", () => {
    localStorage.setItem(KEY, JSON.stringify({ oops: true }));
    const harness = renderHook(() => useCollapsedWorkspaces());
    expect([...harness.current.collapsedWorkspaces]).toEqual([]);
  });

  it("starts expanded when the stored value is corrupt JSON", () => {
    localStorage.setItem(KEY, "{not json");
    const harness = renderHook(() => useCollapsedWorkspaces());
    expect([...harness.current.collapsedWorkspaces]).toEqual([]);
  });

  it("drops non-string entries from a stored array", () => {
    localStorage.setItem(KEY, JSON.stringify(["ws-a", 7, null, "ws-b"]));
    const harness = renderHook(() => useCollapsedWorkspaces());
    expect([...harness.current.collapsedWorkspaces].sort()).toEqual(["ws-a", "ws-b"]);
  });

  it("toggles a workspace and round-trips the set through storage", () => {
    const harness = renderHook(() => useCollapsedWorkspaces());
    act(() => harness.current.toggleWorkspaceCollapsed("ws-a"));
    expect([...harness.current.collapsedWorkspaces]).toEqual(["ws-a"]);
    expect(JSON.parse(localStorage.getItem(KEY)!)).toEqual(["ws-a"]);

    act(() => harness.current.toggleWorkspaceCollapsed("ws-b"));
    expect([...harness.current.collapsedWorkspaces].sort()).toEqual(["ws-a", "ws-b"]);

    act(() => harness.current.toggleWorkspaceCollapsed("ws-a"));
    expect([...harness.current.collapsedWorkspaces]).toEqual(["ws-b"]);
    harness.unmount();

    const reloaded = renderHook(() => useCollapsedWorkspaces());
    expect([...reloaded.current.collapsedWorkspaces]).toEqual(["ws-b"]);
    reloaded.unmount();
  });
});

describe("useLeftPanelWidth", () => {
  beforeEach(() => {
    localStorage.clear();
    setViewportWidth(1280);
  });
  afterEach(() => {
    localStorage.clear();
    setViewportWidth(1024);
    vi.restoreAllMocks();
  });

  it("clamps the stored preference to the hard ceiling", () => {
    localStorage.setItem("future.leftPanelWidth", "600");
    const harness = renderHook(() => useLeftPanelWidth(false));
    expect(harness.current.width).toBe(480);
    expect(harness.current.maxWidth).toBe(480);
  });

  it("reserves the right panel floor when the panel is visible", () => {
    setViewportWidth(1000);
    localStorage.setItem("future.leftPanelWidth", "400");
    const harness = renderHook(() => useLeftPanelWidth(true));
    expect(harness.current.maxWidth).toBeLessThan(400);
    expect(harness.current.width).toBe(harness.current.maxWidth);
  });

  it("nudges within the clamp and persists the new width", () => {
    const harness = renderHook(() => useLeftPanelWidth(false));
    const before = harness.current.width;
    expect(before).toBe(288);
    act(() => harness.current.nudge(16));
    expect(harness.current.width).toBe(before + 16);
    expect(localStorage.getItem("future.leftPanelWidth")).toBe(String(before + 16));
  });

  it("falls back to a viewport-derived default when storage is unreadable", () => {
    vi.spyOn(localStorage, "getItem").mockImplementation(() => {
      throw new Error("storage disabled");
    });
    const harness = renderHook(() => useLeftPanelWidth(false));
    expect(harness.current.width).toBe(288);
  });

  it("survives a storage write failure", () => {
    vi.spyOn(localStorage, "setItem").mockImplementation(() => {
      throw new Error("quota exceeded");
    });
    const harness = renderHook(() => useLeftPanelWidth(false));
    act(() => harness.current.nudge(16));
    // The in-memory width still moved even though persistence failed.
    expect(harness.current.width).toBe(304);
  });

  it("ignores a drag that did not start with the primary button", () => {
    const harness = renderHook(() => useLeftPanelWidth(false));
    const preventDefault = vi.fn();
    act(() => {
      harness.current.startResize({ button: 2, clientX: 100, pointerId: 1, preventDefault } as never);
    });
    expect(harness.current.resizing).toBe(false);
    expect(preventDefault).not.toHaveBeenCalled();

    act(() => {
      window.dispatchEvent(new Event("pointermove"));
    });
    expect(harness.current.width).toBe(288);
    harness.unmount();
  });

  // The default width is a three-branch threshold chain, so each side of both
  // thresholds is asserted — 1280 and 768 are the edges, not the middles.
  it.each([
    { expected: 288, label: "very wide", width: 1920 },
    { expected: 288, label: "at the 1280 threshold", width: 1280 },
    { expected: 256, label: "one pixel below 1280", width: 1279 },
    { expected: 256, label: "at the 768 threshold", width: 768 },
    { expected: 224, label: "one pixel below 768", width: 767 },
    { expected: 224, label: "narrow", width: 480 },
  ])("derives the default rail width from a $width px viewport ($label)", ({ expected, width }) => {
    setViewportWidth(width);
    const harness = renderHook(() => useLeftPanelWidth(false));
    expect(harness.current.width).toBe(expected);
    harness.unmount();
  });

  it("falls through to the viewport default for a stored value that is not a number", () => {
    setViewportWidth(1000);
    localStorage.setItem("future.leftPanelWidth", "wide");
    const harness = renderHook(() => useLeftPanelWidth(false));
    expect(harness.current.width).toBe(256);
    harness.unmount();
  });

  it("treats an empty stored value as zero and clamps it to the floor", () => {
    // `Number("")` is 0, so an empty string is a *width*, not "unset": it clamps
    // up to the minimum rather than falling back to the viewport default.
    setViewportWidth(1000);
    localStorage.setItem("future.leftPanelWidth", "");
    const harness = renderHook(() => useLeftPanelWidth(false));
    expect(harness.current.width).toBe(224);
    harness.unmount();
  });

  it("keeps dragging until the pointer that started it is released", () => {
    const harness = renderHook(() => useLeftPanelWidth(false));
    act(() => {
      harness.current.startResize({ button: 0, clientX: 100, pointerId: 7, preventDefault: vi.fn() } as never);
    });
    expect(harness.current.resizing).toBe(true);

    // A second pointer (another finger, a stylus, a synthesised event) releasing
    // must not end this drag.
    act(() => {
      window.dispatchEvent(pointerEvent("pointerup", 8));
    });
    expect(harness.current.resizing).toBe(true);

    // Nor may it resize the rail.
    act(() => {
      window.dispatchEvent(pointerEvent("pointermove", 8, 500));
    });
    expect(harness.current.width).toBe(288);

    // The owning pointer's move is followed, and only its release ends the drag.
    act(() => {
      window.dispatchEvent(pointerEvent("pointermove", 7, 140));
    });
    expect(harness.current.width).toBe(328);
    act(() => {
      window.dispatchEvent(pointerEvent("pointerup", 7));
    });
    expect(harness.current.resizing).toBe(false);
    harness.unmount();
  });

  it("clamps a drag past the ceiling to the maximum width", () => {
    const harness = renderHook(() => useLeftPanelWidth(false));
    act(() => {
      harness.current.startResize({ button: 0, clientX: 0, pointerId: 1, preventDefault: vi.fn() } as never);
    });
    act(() => {
      window.dispatchEvent(pointerEvent("pointermove", 1, 10_000));
    });
    expect(harness.current.width).toBe(480);
    expect(harness.current.width).toBe(harness.current.maxWidth);
    act(() => {
      window.dispatchEvent(pointerEvent("pointerup", 1));
    });
    harness.unmount();
  });
});

describe("useRailSelection", () => {
  const threadScopes = new Map([["a", "chat"], ["b", "chat"], ["p", "chat"], ["w", "workspace"]]);
  const visibleThreads = [thread("a"), thread("b"), thread("p", { pinned: true }), thread("w")];

  function mountHook() {
    const onBatchDeleteThreads = vi.fn();
    const onSelectThread = vi.fn();
    const harness = renderHook(() => useRailSelection({
      onBatchDeleteThreads,
      onSelectThread,
      threadScopes,
      visibleThreads,
    }));
    return { harness, onBatchDeleteThreads, onSelectThread };
  }

  it("selects a row normally when not in selection mode", () => {
    const { harness, onSelectThread } = mountHook();
    act(() => harness.current.handleRowSelect(visibleThreads[0]!));
    expect(onSelectThread).toHaveBeenCalledWith(visibleThreads[0]);
    expect(harness.current.selectedThreadIds.size).toBe(0);
  });

  it("excludes pinned threads from the scoped set and the row toggle", () => {
    const { harness, onSelectThread } = mountHook();
    act(() => harness.current.enterSelectionMode("chat"));
    expect(harness.current.scopedThreads.map(item => item.id)).toEqual(["a", "b"]);
    expect(harness.current.isThreadInScope(thread("p", { pinned: true }))).toBe(false);
    expect(harness.current.isThreadInScope(visibleThreads[1]!)).toBe(true);
    expect(harness.current.isThreadInScope(visibleThreads[3]!)).toBe(false);

    act(() => harness.current.handleRowSelect(thread("p", { pinned: true })));
    expect(harness.current.selectedThreadIds.size).toBe(0);
    expect(onSelectThread).not.toHaveBeenCalled();

    act(() => harness.current.handleRowSelect(visibleThreads[1]!));
    expect([...harness.current.selectedThreadIds]).toEqual(["b"]);
    expect(onSelectThread).not.toHaveBeenCalled();
  });

  it("selects and deselects every scoped thread", () => {
    const { harness } = mountHook();
    act(() => harness.current.enterSelectionMode("chat"));
    act(() => harness.current.selectAll());
    expect([...harness.current.selectedThreadIds].sort()).toEqual(["a", "b"]);
    act(() => harness.current.deselectAll());
    expect(harness.current.selectedThreadIds.size).toBe(0);
  });

  it("toggles a selected thread back off", () => {
    const { harness } = mountHook();
    act(() => harness.current.enterSelectionMode("chat"));
    act(() => harness.current.toggleThreadSelection(thread("a")));
    expect([...harness.current.selectedThreadIds]).toEqual(["a"]);
    act(() => harness.current.toggleThreadSelection(thread("a")));
    expect([...harness.current.selectedThreadIds]).toEqual([]);
  });

  it("hands the selected threads to the batch delete and exits selection mode", () => {
    const { harness, onBatchDeleteThreads } = mountHook();
    act(() => harness.current.enterSelectionMode("chat"));
    act(() => harness.current.selectAll());
    act(() => harness.current.deleteSelected());
    expect(onBatchDeleteThreads).toHaveBeenCalledWith([visibleThreads[0], visibleThreads[1]]);
    expect(harness.current.selectionMode).toBe(false);
    expect(harness.current.selectedThreadIds.size).toBe(0);
  });

  it("does not open a batch delete with an empty selection", () => {
    const { harness, onBatchDeleteThreads } = mountHook();
    act(() => harness.current.enterSelectionMode("chat"));
    act(() => harness.current.deleteSelected());
    expect(onBatchDeleteThreads).not.toHaveBeenCalled();
    // An empty batch keeps selection mode open so the user can still pick rows.
    expect(harness.current.selectionMode).toBe(true);
  });

  it("exits selection mode on Escape only while it is open", () => {
    const { harness } = mountHook();
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(harness.current.selectionMode).toBe(false);

    act(() => harness.current.enterSelectionMode("chat"));
    act(() => harness.current.selectAll());
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(harness.current.selectionMode).toBe(false);
    expect(harness.current.selectedThreadIds.size).toBe(0);
  });

  it("prunes threads that disappear while selection mode is open", () => {
    function SelectionProbe({ threads }: { threads: StoredThread[] }) {
      const selection = useRailSelection({
        onBatchDeleteThreads: () => {},
        onSelectThread: () => {},
        threadScopes,
        visibleThreads: threads,
      });
      return createElement("div", {
        "data-mode": String(selection.selectionMode),
        "data-selected": [...selection.selectedThreadIds].sort().join(","),
      }, [
        createElement("button", { key: "enter", onClick: () => selection.enterSelectionMode("chat"), type: "button" }, "enter"),
        createElement("button", { key: "all", onClick: () => selection.selectAll(), type: "button" }, "all"),
      ]);
    }

    const view = mount(createElement(SelectionProbe, { threads: [thread("a"), thread("b")] }));
    const [enterButton, allButton] = [...view.container.querySelectorAll("button")] as HTMLButtonElement[];
    act(() => enterButton!.click());
    act(() => allButton!.click());
    const root = () => view.container.firstElementChild!;
    expect(root().getAttribute("data-selected")).toBe("a,b");

    view.rerender(createElement(SelectionProbe, { threads: [thread("b")] }));
    expect(root().getAttribute("data-selected")).toBe("b");
    expect(root().getAttribute("data-mode")).toBe("true");
    view.unmount();
  });
});
