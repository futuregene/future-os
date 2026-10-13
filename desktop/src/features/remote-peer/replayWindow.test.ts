import { describe, expect, it } from "vitest";
import {
  advanceCursor,
  applyReplayPage,
  cursorHighWater,
  highestWatermark,
  isPrefixComplete,
  newRunCursor,
  nextEvent,
  tailCoversRange,
} from "./replayWindow";

describe("replayWindow", () => {
  describe("nextEvent", () => {
    it("classifies the first event of a run with idx=0 as prefix-complete apply", () => {
      const cursor = newRunCursor();
      expect(nextEvent(cursor, "r1", 0)).toEqual({ kind: "apply", idx: 0 });
      expect(cursorHighWater(cursor, "r1")).toBe(0);
      expect(isPrefixComplete(cursor, "r1")).toBe(true);
    });

    it("classifies a first event with idx > 0 as apply but prefix-incomplete", () => {
      const cursor = newRunCursor();
      expect(nextEvent(cursor, "r1", 5)).toEqual({ kind: "apply", idx: 5 });
      expect(isPrefixComplete(cursor, "r1")).toBe(false);
    });

    it("classifies a duplicate event as dup", () => {
      const cursor = newRunCursor();
      nextEvent(cursor, "r1", 0);
      expect(nextEvent(cursor, "r1", 0)).toEqual({ kind: "dup" });
    });

    it("classifies an in-order continuation as apply", () => {
      const cursor = newRunCursor();
      nextEvent(cursor, "r1", 0);
      expect(nextEvent(cursor, "r1", 1)).toEqual({ kind: "apply", idx: 1 });
      expect(cursorHighWater(cursor, "r1")).toBe(1);
    });

    it("detects a gap", () => {
      const cursor = newRunCursor();
      nextEvent(cursor, "r1", 0);
      expect(nextEvent(cursor, "r1", 3)).toEqual({ kind: "gap", fromIdx: 0 });
    });

    it("detects an coalesced overlap", () => {
      const cursor = newRunCursor();
      nextEvent(cursor, "r1", 4);
      expect(nextEvent(cursor, "r1", 6, 3)).toEqual({ kind: "overlap", fromIdx: 4 });
    });

    it("advances a coalesced range that continues exactly after highWater", () => {
      const cursor = newRunCursor();
      nextEvent(cursor, "r1", 4);
      expect(nextEvent(cursor, "r1", 6, 2)).toEqual({ kind: "apply", idx: 6 });
      expect(cursorHighWater(cursor, "r1")).toBe(6);
    });

    it("advances over omitted indices when omitIndices is set", () => {
      const cursor = newRunCursor();
      nextEvent(cursor, "r1", 0);
      expect(nextEvent(cursor, "r1", 5, undefined, { omitIndices: true })).toEqual({
        kind: "apply",
        idx: 5,
      });
      expect(cursorHighWater(cursor, "r1")).toBe(5);
    });

    it("treats untracked events as applicable", () => {
      const cursor = newRunCursor();
      expect(nextEvent(cursor, undefined, 0)).toEqual({ kind: "untracked" });
      expect(nextEvent(cursor, "r1", undefined)).toEqual({ kind: "untracked" });
    });

    it("evicts the oldest run when MAX_RUNS is exceeded", () => {
      const cursor = newRunCursor();
      for (let i = 0; i < 9; i += 1) {
        nextEvent(cursor, `r${i}`, 0);
      }
      expect(cursor.has("r0")).toBe(false);
      expect(cursor.has("r8")).toBe(true);
    });
  });

  describe("advanceCursor", () => {
    it("marks a run prefix-complete when completePrefix is true", () => {
      const cursor = newRunCursor();
      advanceCursor(cursor, "r1", 10, true);
      expect(isPrefixComplete(cursor, "r1")).toBe(true);
      expect(cursorHighWater(cursor, "r1")).toBe(10);
    });
  });

  describe("tailCoversRange", () => {
    it("accepts a contiguous untrimmed tail", () => {
      const events = [{ idx: 1 }, { idx: 2 }, { idx: 3 }];
      expect(tailCoversRange(events, 0, 3, undefined)).toBe(true);
    });

    it("rejects a tail with missing indices in untrimmed mode", () => {
      const events = [{ idx: 1 }, { idx: 3 }];
      expect(tailCoversRange(events, 0, 3, undefined)).toBe(false);
    });

    it("accepts a trimmed tail when rawEvents matches the watermark", () => {
      const events = [{ idx: 1 }, { idx: 5 }];
      expect(tailCoversRange(events, 0, 5, 5)).toBe(true);
    });

    it("rejects a trimmed tail when rawEvents does not match", () => {
      const events = [{ idx: 1 }, { idx: 5 }];
      expect(tailCoversRange(events, 0, 5, 4)).toBe(false);
    });

    it("rejects a non-monotonic trimmed tail", () => {
      const events = [{ idx: 5 }, { idx: 1 }];
      expect(tailCoversRange(events, 0, 5, 5)).toBe(false);
    });
  });

  describe("applyReplayPage", () => {
    it("applies a contiguous replay page and updates cursors", () => {
      const cursor = newRunCursor();
      const events = [
        { id: "a", runId: "r1", idx: 0 },
        { id: "b", runId: "r1", idx: 1 },
        { id: "c", runId: "r1", idx: 2 },
      ];
      const result = applyReplayPage(cursor, events, 2, 3, -1);
      expect(result.apply).toEqual(events);
      expect(result.prefixComplete).toBe(true);
      expect(result.highWaters.r1).toBe(2);
    });

    it("falls back to null when rawEvents does not cover watermark", () => {
      const cursor = newRunCursor();
      const events = [{ id: "a", runId: "r1", idx: 0 }];
      const result = applyReplayPage(cursor, events, 3, 3, -1);
      expect(result.apply).toBeNull();
    });

    it("detects a gap inside a replay page", () => {
      const cursor = newRunCursor();
      const events = [
        { id: "a", runId: "r1", idx: 0 },
        { id: "b", runId: "r1", idx: 2 },
      ];
      // No rawEvents => untrimmed, so an index hole is a real gap.
      const result = applyReplayPage(cursor, events, 2, undefined, -1);
      expect(result.apply).toBeNull();
    });

    it("skips duplicate events without failing", () => {
      const cursor = newRunCursor();
      advanceCursor(cursor, "r1", 1, true);
      const events = [
        { id: "a", runId: "r1", idx: 0 },
        { id: "b", runId: "r1", idx: 1 },
        { id: "c", runId: "r1", idx: 2 },
      ];
      const result = applyReplayPage(cursor, events, 2, 3, -1);
      expect(result.apply?.map(e => e.id)).toEqual(["c"]);
    });

    it("rejects an overlap inside the replay page", () => {
      const cursor = newRunCursor();
      advanceCursor(cursor, "r1", 10, true);
      const events = [
        { id: "a", runId: "r1", idx: 12, coalescedCount: 4 },
      ];
      // covered.start = 9, which is below highWater 10 → overlap
      const result = applyReplayPage(cursor, events, 12, 13, 10);
      expect(result.apply).toBeNull();
    });

    it("accepts a trimmed page whose rawEvents prove coverage", () => {
      const cursor = newRunCursor();
      advanceCursor(cursor, "r1", 10, true);
      const events = [
        { id: "a", runId: "r1", idx: 12 },
        { id: "b", runId: "r1", idx: 15 },
      ];
      const result = applyReplayPage(cursor, events, 15, 5, 10);
      expect(result.apply).toHaveLength(2);
      expect(result.prefixComplete).toBe(true);
    });

    it("rejects a trimmed page whose rawEvents do not cover the watermark", () => {
      const cursor = newRunCursor();
      advanceCursor(cursor, "r1", 10, true);
      const events = [
        { id: "a", runId: "r1", idx: 12 },
        { id: "b", runId: "r1", idx: 15 },
      ];
      const result = applyReplayPage(cursor, events, 15, 4, 10);
      expect(result.apply).toBeNull();
    });
  });

  describe("highestWatermark", () => {
    it("returns the highest highWater across runs", () => {
      const cursor = newRunCursor();
      advanceCursor(cursor, "r1", 10);
      advanceCursor(cursor, "r2", 42);
      expect(highestWatermark(cursor)).toBe(42);
    });

    it("returns -1 for an empty cursor", () => {
      expect(highestWatermark(newRunCursor())).toBe(-1);
    });
  });
});
