// @vitest-environment jsdom
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "../../../test/renderHook";
import { useRailView } from "./useRailView";

beforeEach(() => {
  localStorage.clear();
});

describe("the rail's view", () => {
  it("opens on the tree", () => {
    // The tree is the product's primary shape; the recent view is a second
    // answer to "what did I last touch" and must not take over the first screen.
    const hook = renderHook(() => useRailView());
    expect(hook.current.view).toBe("workspace");
    hook.unmount();
  });

  it("remembers the view across mounts", () => {
    const first = renderHook(() => useRailView());
    act(() => first.current.setView("recent"));
    expect(first.current.view).toBe("recent");
    expect(localStorage.getItem("future.activityRail.view")).toBe("recent");
    first.unmount();

    // A restart: a new mount reads what the last one stored.
    const second = renderHook(() => useRailView());
    expect(second.current.view).toBe("recent");
    second.unmount();
  });

  it("falls back to the tree for a value it does not recognise", () => {
    // An older build's value, or a corrupt write: an unknown string must not
    // leave the reader with no list at all.
    localStorage.setItem("future.activityRail.view", "something-else");
    const hook = renderHook(() => useRailView());
    expect(hook.current.view).toBe("workspace");
    hook.unmount();
  });
});

describe("a rail whose storage refuses to answer", () => {
  it("opens on the tree instead of failing to render", () => {
    // A locked-down webview throws on `getItem` rather than returning null: the rail
    // must still draw, on the default view, exactly as on a first launch.
    const getItem = vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("denied");
    });
    try {
      const hook = renderHook(() => useRailView());
      expect(hook.current.view).toBe("workspace");
      expect(hook.current.order).toEqual(["workspace", "recent"]);
      hook.unmount();
    }
    finally {
      getItem.mockRestore();
    }
  });
});

describe("the switch's order", () => {
  it("opens on workspaces, then recent", () => {
    const hook = renderHook(() => useRailView());
    expect(hook.current.order).toEqual(["workspace", "recent"]);
    hook.unmount();
  });

  it("exchanges the two segments, and keeps them that way", () => {
    const first = renderHook(() => useRailView());
    act(() => first.current.moveView("workspace", "recent"));
    expect(first.current.order).toEqual(["recent", "workspace"]);
    // The exchange is a preference, not a one-off: it is stored as soon as it
    // happens, so a restart draws the switch the reader left.
    expect(JSON.parse(localStorage.getItem("future.activityRail.viewOrder")!)).toEqual(["recent", "workspace"]);
    first.unmount();

    const second = renderHook(() => useRailView());
    expect(second.current.order).toEqual(["recent", "workspace"]);
    second.unmount();
  });

  it("leaves the showing view alone", () => {
    // Rearranging the switch is not picking a list: the reader dragged a
    // control, they did not ask for the other view's contents.
    const hook = renderHook(() => useRailView());
    act(() => hook.current.moveView("workspace", "recent"));
    expect(hook.current.view).toBe("workspace");
    hook.unmount();
  });

  it("ignores a move that is not between the two views", () => {
    const hook = renderHook(() => useRailView());
    act(() => hook.current.moveView("workspace", "workspace"));
    expect(hook.current.order).toEqual(["workspace", "recent"]);
    hook.unmount();
  });

  /** Stored values that would lose a segment must not take the switch with them. */
  it.each([
    { label: "one view twice", stored: ["recent", "recent"] },
    { label: "a single segment", stored: ["recent"] },
    { label: "an unknown id", stored: ["recent", "tasks"] },
    { label: "not an array", stored: "recent,workspace" },
  ])("falls back to the default for $label", ({ stored }) => {
    localStorage.setItem("future.activityRail.viewOrder", JSON.stringify(stored));
    const hook = renderHook(() => useRailView());
    expect(hook.current.order).toEqual(["workspace", "recent"]);
    hook.unmount();
  });

  it("falls back to the default for a corrupt write", () => {
    localStorage.setItem("future.activityRail.viewOrder", "{not json");
    const hook = renderHook(() => useRailView());
    expect(hook.current.order).toEqual(["workspace", "recent"]);
    hook.unmount();
  });
});
