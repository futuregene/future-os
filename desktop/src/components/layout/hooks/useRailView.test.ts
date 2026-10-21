// @vitest-environment jsdom
import { act } from "react";
import { beforeEach, describe, expect, it } from "vitest";
import { renderHook } from "../../../test/renderHook";
import { useRailView } from "./useRailView";

beforeEach(() => {
  localStorage.clear();
});

describe("the rail's view", () => {
  it("opens on the workspace list", () => {
    // The workspace tree is the product's primary shape; the recent list is a
    // second answer to "what did I last touch" and must not take over the first
    // screen.
    const hook = renderHook(() => useRailView());
    expect(hook.current.view).toBe("workspace");
    hook.unmount();
  });

  it("remembers each of the three lists across mounts", () => {
    for (const view of ["recent", "chat", "workspace"] as const) {
      const first = renderHook(() => useRailView());
      act(() => first.current.select(view));
      expect(first.current.view).toBe(view);
      expect(localStorage.getItem("future.activityRail.view")).toBe(view);
      first.unmount();

      // A restart: a new mount reads what the last one stored.
      const second = renderHook(() => useRailView());
      expect(second.current.view).toBe(view);
      second.unmount();
    }
  });

  /**
   * The difference the two setters exist for.
   *
   * `select` is the user choosing a list, and the choice is what the next launch
   * opens. `follow` is the shell taking the reader somewhere — they opened a
   * conversation that lives in another tab — and remembering *that* would mean
   * the app reopening wherever they were last dragged instead of where they chose.
   */
  it("moves the tab without remembering it when the shell follows a conversation", () => {
    const first = renderHook(() => useRailView());
    act(() => first.current.select("workspace"));
    act(() => first.current.follow("chat"));

    // The tab moved for this run…
    expect(first.current.view).toBe("chat");
    // …and the remembered choice did not.
    expect(localStorage.getItem("future.activityRail.view")).toBe("workspace");
    first.unmount();

    const second = renderHook(() => useRailView());
    expect(second.current.view).toBe("workspace");
    second.unmount();
  });

  it("falls back to the workspace list for a value it does not recognise", () => {
    // An older build's value, or a corrupt write: an unknown string must not
    // leave the reader with no list at all. (`tree` is the pre-tabs name for the
    // same list, and is not recognised either — the default is that list.)
    for (const stored of ["something-else", "tree", ""]) {
      localStorage.setItem("future.activityRail.view", stored);
      const hook = renderHook(() => useRailView());
      expect(hook.current.view).toBe("workspace");
      hook.unmount();
    }
  });

  it("reads a value stored by the two-tab build as what it meant", () => {
    // `recent` meant the recent list before this change and still does; a user
    // who was there when they quit opens there. No migration needed.
    localStorage.setItem("future.activityRail.view", "recent");
    const hook = renderHook(() => useRailView());
    expect(hook.current.view).toBe("recent");
    hook.unmount();
  });
});
