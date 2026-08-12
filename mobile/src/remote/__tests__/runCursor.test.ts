import {
  advanceCursor,
  cursorHighWater,
  isPrefixComplete,
  newCursor,
  nextEvent,
  rebuildCursorFromEvents,
} from "../runCursor";

describe("runCursor", () => {
  test("consecutive idx → apply and advance", () => {
    const cursor = newCursor();
    expect(nextEvent(cursor, "run1", 0)).toEqual({ kind: "apply", idx: 0 });
    expect(nextEvent(cursor, "run1", 1)).toEqual({ kind: "apply", idx: 1 });
    expect(nextEvent(cursor, "run1", 2)).toEqual({ kind: "apply", idx: 2 });
    expect(cursor.get("run1")?.highWater).toBe(2);
  });

  test("gap detection: idx 0,1,3 → gap(fromIdx=1)", () => {
    const cursor = newCursor();
    nextEvent(cursor, "run1", 0);
    nextEvent(cursor, "run1", 1);
    expect(nextEvent(cursor, "run1", 3)).toEqual({ kind: "gap", fromIdx: 1 });
    // Cursor unchanged — gap event not applied.
    expect(cursor.get("run1")?.highWater).toBe(1);
  });

  test("old idx → dup", () => {
    const cursor = newCursor();
    nextEvent(cursor, "run1", 0);
    nextEvent(cursor, "run1", 1);
    nextEvent(cursor, "run1", 2);
    expect(nextEvent(cursor, "run1", 1)).toEqual({ kind: "dup" });
    expect(nextEvent(cursor, "run1", 0)).toEqual({ kind: "dup" });
    expect(nextEvent(cursor, "run1", 2)).toEqual({ kind: "dup" });
  });

  test("no runId or idx → untracked", () => {
    const cursor = newCursor();
    expect(nextEvent(cursor, undefined, 5)).toEqual({ kind: "untracked" });
    expect(nextEvent(cursor, "run1", undefined)).toEqual({ kind: "untracked" });
    expect(nextEvent(cursor, null, null)).toEqual({ kind: "untracked" });
    // Cursor not modified.
    expect(cursor.size).toBe(0);
  });

  test("new runId → independent cursor", () => {
    const cursor = newCursor();
    nextEvent(cursor, "runA", 0);
    nextEvent(cursor, "runA", 1);
    expect(nextEvent(cursor, "runB", 0)).toEqual({ kind: "apply", idx: 0 });
    expect(nextEvent(cursor, "runB", 1)).toEqual({ kind: "apply", idx: 1 });
    expect(cursor.get("runA")?.highWater).toBe(1);
    expect(cursor.get("runB")?.highWater).toBe(1);
  });

  test("cursor evicts oldest runs beyond capacity (8)", () => {
    const cursor = newCursor();
    for (let i = 0; i < 10; i++) {
      nextEvent(cursor, `run${i}`, 0);
    }
    // Oldest two should be evicted.
    expect(cursor.has("run0")).toBe(false);
    expect(cursor.has("run1")).toBe(false);
    expect(cursor.has("run2")).toBe(true);
    expect(cursor.has("run9")).toBe(true);
    expect(cursor.size).toBe(8);
  });

  test("advanceCursor only moves forward", () => {
    const cursor = newCursor();
    advanceCursor(cursor, "run1", 5);
    advanceCursor(cursor, "run1", 3); // should not regress
    expect(cursor.get("run1")?.highWater).toBe(5);
    advanceCursor(cursor, "run1", 7);
    expect(cursor.get("run1")?.highWater).toBe(7);
  });

  test("rebuildCursorFromEvents sets max idx per run and marks prefix complete", () => {
    const cursor = newCursor();
    rebuildCursorFromEvents(cursor, [
      { runId: "run1", idx: 0 },
      { runId: "run1", idx: 3 },
      { runId: "run1", idx: 1 },
      { runId: "run2", idx: 10 },
      { runId: null, idx: 99 },
      { runId: "run3", idx: null },
    ]);
    expect(cursor.get("run1")?.highWater).toBe(3);
    expect(cursor.get("run1")?.prefixComplete).toBe(true);
    expect(cursor.get("run2")?.highWater).toBe(10);
    expect(cursor.has("run3")).toBe(false);
    expect(cursor.size).toBe(2);
  });

  test("first event idx 0 → prefix complete (H3)", () => {
    const cursor = newCursor();
    nextEvent(cursor, "run1", 0);
    expect(cursor.get("run1")).toEqual({ highWater: 0, prefixComplete: true });
    nextEvent(cursor, "run1", 1);
    expect(cursor.get("run1")?.prefixComplete).toBe(true);
  });

  test("first event idx > 0 → prefix incomplete (H3)", () => {
    const cursor = newCursor();
    nextEvent(cursor, "run1", 5);
    expect(cursor.get("run1")).toEqual({ highWater: 5, prefixComplete: false });
    // Consecutive events keep it incomplete.
    nextEvent(cursor, "run1", 6);
    expect(cursor.get("run1")?.prefixComplete).toBe(false);
  });

  test("advanceCursor with completePrefix upgrades an incomplete run", () => {
    const cursor = newCursor();
    nextEvent(cursor, "run1", 5); // prefix incomplete
    advanceCursor(cursor, "run1", 5, true);
    expect(cursor.get("run1")?.prefixComplete).toBe(true);
  });

  test("advanceCursor without completePrefix never regresses completeness", () => {
    const cursor = newCursor();
    rebuildCursorFromEvents(cursor, [{ runId: "run1", idx: 3 }]);
    advanceCursor(cursor, "run1", 5); // tail top-up, no completion flag
    expect(cursor.get("run1")?.prefixComplete).toBe(true);
  });

  test("helpers expose high-water and completeness", () => {
    const cursor = newCursor();
    expect(cursorHighWater(cursor, "run1")).toBe(-1);
    expect(isPrefixComplete(cursor, "run1")).toBe(false);
    nextEvent(cursor, "run1", 0);
    expect(cursorHighWater(cursor, "run1")).toBe(0);
    expect(isPrefixComplete(cursor, "run1")).toBe(true);
    expect(cursorHighWater(cursor, undefined)).toBe(-1);
    expect(isPrefixComplete(cursor, undefined)).toBe(true);
  });
});
