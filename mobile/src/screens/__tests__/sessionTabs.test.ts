import { adjacentTab, swipeTarget, TABS } from "../sessionTabs";

describe("the row of pages", () => {
  it("is recency, workspaces, conversations — left to right", () => {
    // The leftmost page is the one the app opens on: a row whose default sits in
    // the middle would start the reader in the middle of the row.
    expect(TABS).toEqual(["recent", "workspace", "chat"]);
  });

  it("puts the neighbour a gesture opens beside the page it started on", () => {
    expect(adjacentTab("recent", "next")).toBe("workspace");
    expect(adjacentTab("workspace", "next")).toBe("chat");
    expect(adjacentTab("workspace", "prev")).toBe("recent");
    expect(adjacentTab("chat", "prev")).toBe("workspace");
    // Both ends of the row have no neighbour, which is what `swipeTarget` is for.
    expect(adjacentTab("chat", "next")).toBeNull();
    expect(adjacentTab("recent", "prev")).toBeNull();
  });

  it("wraps at both ends, one page per gesture either way", () => {
    // Left from the conversations page, and right from the recency page: each
    // ends the row, and each closes it into a loop.
    expect(swipeTarget("chat", "next")).toBe("recent");
    expect(swipeTarget("recent", "prev")).toBe("chat");
    // Everywhere else a swipe moves exactly one page, so the loop never skips
    // the middle.
    expect(swipeTarget("recent", "next")).toBe("workspace");
    expect(swipeTarget("workspace", "next")).toBe("chat");
    expect(swipeTarget("workspace", "prev")).toBe("recent");
    expect(swipeTarget("chat", "prev")).toBe("workspace");
  });

  it("never answers with the page it was given", () => {
    for (const tab of TABS)
      for (const direction of ["next", "prev"] as const)
        expect(swipeTarget(tab, direction)).not.toBe(tab);
  });
});
