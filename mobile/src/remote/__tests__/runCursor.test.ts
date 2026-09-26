import {
  advanceCursor,
  cursorHighWater,
  isPrefixComplete,
  newCursor,
  nextEvent,
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

  // A feed that omits indices on purpose (the client declared a trimmed lane)
  // must advance over the hole: the peer is not sending those slices, so a gap
  // verdict would send the engine into a reconcile that can never recover them.
  test("a feed that omits indices applies across the hole", () => {
    const cursor = newCursor();
    nextEvent(
      cursor,
      "run1",
      0,
      undefined,
      { omitIndices: true },
    );
    nextEvent(
      cursor,
      "run1",
      1,
      undefined,
      { omitIndices: true },
    );
    expect(
      nextEvent(cursor, "run1", 9, undefined, { omitIndices: true }),
    ).toEqual({ kind: "apply", idx: 9 });
    expect(cursor.get("run1")?.highWater).toBe(9);
    // Without the flag the same jump is still a gap: the flag is what says the
    // hole is by design, not the jump itself.
    const strict = newCursor();
    nextEvent(strict, "run1", 0);
    nextEvent(strict, "run1", 1);
    expect(nextEvent(strict, "run1", 9)).toEqual({ kind: "gap", fromIdx: 1 });
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
    advanceCursor(cursor, "run1", 3, true);
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

  // The desktop may merge a run's text fragments into one event
  // (`event_coalescing_v1`). That event carries the newest index and declares
  // how many sources it stands for, so the jump is covered content rather than
  // a gap — treating it as a gap would send the client into a backfill loop
  // that re-downloads exactly what was just merged.
  describe("coalesced ranges", () => {
    test("a declared range is applied, not treated as a gap", () => {
      const cursor = newCursor();
      for (const idx of [0, 1, 2]) nextEvent(cursor, "run1", idx);
      // Covers 3..5 exactly where the raw fragments would have landed.
      const verdict = nextEvent(cursor, "run1", 5, 3);
      expect(verdict.kind).toBe("apply");
      expect(cursorHighWater(cursor, "run1")).toBe(5);
    });

    test("an undeclared jump is still a gap", () => {
      const cursor = newCursor();
      nextEvent(cursor, "run1", 0);
      expect(nextEvent(cursor, "run1", 5).kind).toBe("gap");
      // And a range that starts beyond the high-water is a gap too.
      expect(nextEvent(cursor, "run1", 9, 2).kind).toBe("gap");
    });

    test("a range that continues the high-water is applied", () => {
      const cursor = newCursor();
      for (const idx of [0, 1, 2, 3]) nextEvent(cursor, "run1", idx);
      // Covers 4..6: exactly the sources that follow the high-water.
      expect(nextEvent(cursor, "run1", 6, 3).kind).toBe("apply");
      expect(cursorHighWater(cursor, "run1")).toBe(6);
      // A range that leaves a hole is still a gap.
      expect(nextEvent(cursor, "run1", 12, 3).kind).toBe("gap");
    });

    test("a range overlapping what we applied is not appended verbatim", () => {
      const cursor = newCursor();
      for (const idx of [0, 1, 2, 3]) nextEvent(cursor, "run1", idx);
      // Covers 2..5: a reconcile landed while the merge window was still open,
      // so indices 2 and 3 were already applied from the journal. The merged
      // text is one opaque concatenation, so its already-seen head cannot be
      // trimmed — appending it would duplicate indices 2..3 in the rendered
      // reply. The caller recovers the range from replay instead.
      expect(nextEvent(cursor, "run1", 5, 4)).toEqual({ kind: "overlap", fromIdx: 3 });
      expect(cursorHighWater(cursor, "run1")).toBe(3);
    });

    test("a nonsensical count cannot invent a range past the run start", () => {
      const cursor = newCursor();
      nextEvent(cursor, "run1", 0);
      // count larger than idx+1 clamps to covering from index 0 — an overlap,
      // never a negative start and never a silent append.
      expect(nextEvent(cursor, "run1", 4, 99)).toEqual({ kind: "overlap", fromIdx: 0 });
      const fresh = newCursor();
      // A first coalesced event still records the run as prefix-incomplete
      // unless it begins at zero.
      nextEvent(fresh, "run2", 4, 2);
      expect(fresh.get("run2")?.prefixComplete).toBe(false);
      const fromStart = newCursor();
      nextEvent(fromStart, "run3", 3, 4);
      expect(fromStart.get("run3")).toEqual({ highWater: 3, prefixComplete: true });
    });
  });
});
