// @vitest-environment jsdom
import { act } from "react";
import { beforeEach, describe, expect, it } from "vitest";
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
