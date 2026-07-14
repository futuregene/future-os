/**
 * Unit tests for TargetSessionRegistry.
 */
import { describe, test, expect } from "bun:test";
import { TargetSessionRegistry } from "../../chromium/chromium-target-registry.js";

describe("TargetSessionRegistry", () => {
  test("add and retrieve by both keys", () => {
    const r = new TargetSessionRegistry();
    r.add({ targetId: "t1", sessionId: "s1", type: "page" });

    expect(r.getByTargetId("t1")).toEqual({ targetId: "t1", sessionId: "s1", type: "page" });
    expect(r.getBySessionId("s1")).toEqual({ targetId: "t1", sessionId: "s1", type: "page" });
  });

  test("detachBySessionId is idempotent", () => {
    const r = new TargetSessionRegistry();
    r.add({ targetId: "t1", sessionId: "s1", type: "page" });

    expect(r.detachBySessionId("s1")).toBeTruthy();
    // Second call returns undefined
    expect(r.detachBySessionId("s1")).toBeUndefined();
    // Both lookups now return undefined
    expect(r.getByTargetId("t1")).toBeUndefined();
    expect(r.getBySessionId("s1")).toBeUndefined();
  });

  test("detachByTargetId is idempotent", () => {
    const r = new TargetSessionRegistry();
    r.add({ targetId: "t1", sessionId: "s1", type: "page" });

    expect(r.detachByTargetId("t1")).toBeTruthy();
    expect(r.detachByTargetId("t1")).toBeUndefined();
  });

  test("detach by one key removes both mappings", () => {
    const r = new TargetSessionRegistry();
    r.add({ targetId: "t1", sessionId: "s1", type: "page" });

    r.detachBySessionId("s1");

    // Removing by targetId should also be a no-op now
    expect(r.detachByTargetId("t1")).toBeUndefined();
  });

  test("getAttachedPageIds returns all target IDs", () => {
    const r = new TargetSessionRegistry();
    r.add({ targetId: "t1", sessionId: "s1", type: "page" });
    r.add({ targetId: "t2", sessionId: "s2", type: "page" });

    expect(r.getAttachedPageIds()).toEqual(["t1", "t2"]);
  });

  test("clear removes everything", () => {
    const r = new TargetSessionRegistry();
    r.add({ targetId: "t1", sessionId: "s1", type: "page" });
    r.clear();

    expect(r.getByTargetId("t1")).toBeUndefined();
    expect(r.getAttachedPageIds()).toEqual([]);
  });
});
