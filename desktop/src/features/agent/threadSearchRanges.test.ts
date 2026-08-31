// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { findThreadTextRanges } from "./threadSearchRanges";

describe("findThreadTextRanges", () => {
  it("finds case-insensitive matches across rendered inline nodes", () => {
    const root = document.createElement("div");
    root.innerHTML = "<p>Hello <strong>Future</strong>OS</p><p>futureos again</p>";

    const { hasMore, ranges } = findThreadTextRanges(root, "futureos");

    expect(hasMore).toBe(false);
    expect(ranges).toHaveLength(2);
    expect(ranges.map(range => range.toString())).toEqual(["FutureOS", "futureos"]);
  });

  it("does not search ignored floating controls", () => {
    const root = document.createElement("div");
    root.innerHTML = "visible needle<span data-thread-search-ignore>needle</span>";

    const { ranges } = findThreadTextRanges(root, "needle");

    expect(ranges).toHaveLength(1);
    expect(ranges[0]?.toString()).toBe("needle");
  });

  it("returns no ranges for an empty query", () => {
    const root = document.createElement("div");
    root.textContent = "anything";
    expect(findThreadTextRanges(root, "")).toEqual({ hasMore: false, ranges: [] });
  });

  it("caps low-specificity searches and reports additional matches", () => {
    const root = document.createElement("div");
    root.textContent = "a a a a";

    const result = findThreadTextRanges(root, "a", 3);

    expect(result.ranges).toHaveLength(3);
    expect(result.hasMore).toBe(true);
  });

  it("treats special characters literally and does not join separate messages", () => {
    const root = document.createElement("div");
    root.innerHTML = `
      <div data-message-id="one">ends.</div>
      <div data-message-id="two">starts</div>
    `;

    expect(findThreadTextRanges(root, ".").ranges).toHaveLength(1);
    expect(findThreadTextRanges(root, ".starts").ranges).toHaveLength(0);
  });
});
