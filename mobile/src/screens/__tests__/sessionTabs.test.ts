import { adjacentTab, swipeTarget, TABS } from "../sessionTabs";

describe("the row of pages", () => {
  it("is workspaces, conversations, recency — left to right", () => {
    expect(TABS).toEqual(["workspace", "chat", "recent"]);
  });

  it("puts the neighbour a gesture opens beside the page it started on", () => {
    expect(adjacentTab("workspace", "next")).toBe("chat");
    expect(adjacentTab("chat", "next")).toBe("recent");
    expect(adjacentTab("chat", "prev")).toBe("workspace");
    expect(adjacentTab("recent", "prev")).toBe("chat");
    // Both ends of the row have no neighbour, which is what `swipeTarget` is for.
    expect(adjacentTab("recent", "next")).toBeNull();
    expect(adjacentTab("workspace", "prev")).toBeNull();
  });

  it("wraps at both ends, one page per gesture either way", () => {
    // Left from the recency page, and right from the workspaces page: each had
    // nothing behind it before the row closed into a loop.
    expect(swipeTarget("recent", "next")).toBe("workspace");
    expect(swipeTarget("workspace", "prev")).toBe("recent");
    // Everywhere else a swipe moves exactly one page, so the loop never skips
    // the middle.
    expect(swipeTarget("workspace", "next")).toBe("chat");
    expect(swipeTarget("chat", "next")).toBe("recent");
    expect(swipeTarget("chat", "prev")).toBe("workspace");
    expect(swipeTarget("recent", "prev")).toBe("chat");
  });

  it("never answers with the page it was given", () => {
    for (const tab of TABS)
      for (const direction of ["next", "prev"] as const)
        expect(swipeTarget(tab, direction)).not.toBe(tab);
  });
});
