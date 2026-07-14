/**
 * Unit tests for tab-order reconciliation.
 */
import { describe, test, expect } from "bun:test";
import { reconcilePageOrder, insertNewPage, removePage, resolveActivePage } from "../../tab-order.js";

describe("reconcilePageOrder", () => {
  test("no stored order returns current order", () => {
    expect(reconcilePageOrder(undefined, ["a", "b", "c"])).toEqual(["a", "b", "c"]);
  });

  test("empty stored order returns current order", () => {
    expect(reconcilePageOrder([], ["a", "b"])).toEqual(["a", "b"]);
  });

  test("surviving pages keep stored order", () => {
    // Stored: [a, b, c], Current: [b, a, d] → [a, b, d]
    expect(reconcilePageOrder(["a", "b", "c"], ["b", "a", "d"])).toEqual(["a", "b", "d"]);
  });

  test("closed pages removed, new pages appended", () => {
    expect(reconcilePageOrder(["a", "b"], ["b", "c"])).toEqual(["b", "c"]);
  });

  test("reorder preserved for surviving pages", () => {
    // If stored was set in order [c, b, a], then a and b closed, c remains first
    expect(reconcilePageOrder(["c", "b", "a"], ["c"])).toEqual(["c"]);
  });
});

describe("insertNewPage", () => {
  test("appends new page", () => {
    expect(insertNewPage(["a"], "b")).toEqual(["a", "b"]);
  });

  test("no-op if already present", () => {
    expect(insertNewPage(["a", "b"], "a")).toEqual(["a", "b"]);
  });
});

describe("removePage", () => {
  test("removes existing page", () => {
    expect(removePage(["a", "b", "c"], "b")).toEqual(["a", "c"]);
  });

  test("no-op if not present", () => {
    expect(removePage(["a"], "b")).toEqual(["a"]);
  });
});

describe("resolveActivePage", () => {
  test("returns activePageId if present", () => {
    expect(resolveActivePage(["a", "b", "c"], "b")).toBe("b");
  });

  test("defaults to last page if activePageId not found", () => {
    expect(resolveActivePage(["a", "b", "c"], "d")).toBe("c");
  });

  test("defaults to last page if no activePageId", () => {
    expect(resolveActivePage(["a", "b"], undefined)).toBe("b");
  });

  test("returns undefined for empty list", () => {
    expect(resolveActivePage([], undefined)).toBeUndefined();
  });
});
