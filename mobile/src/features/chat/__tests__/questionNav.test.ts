import type { TimelineItem } from "../../../remote/types";
import {
  EMPTY_ROWS,
  buildLadder,
  nextRung,
  previousRung,
  questionIndices,
  viewportRows,
  type QuestionRung,
} from "../questionNav";

const rows = (top: number | null, topClipped = false) => ({ top, topClipped });

const message = (
  id: string,
  role: "user" | "assistant",
  extra: Record<string, unknown> = {},
): TimelineItem =>
  ({ id, kind: "message", role, text: "", ...extra }) as unknown as TimelineItem;

const text = (id: string) => ({ id, kind: "text", text: "…" });
const thinking = (id: string) => ({ id, kind: "thinking", text: "…" });
const tool = (id: string) => ({
  id,
  kind: "tool",
  tool: { name: "run", complete: true },
});
const compaction = (id: string) => ({ id, kind: "compaction" });

/**
 * The fixture transcript, newest first:
 *
 *   A3 (text, thinking, text)              0
 *   Q3                                     1
 *   A2b (tool, thinking)                   2   ← thinking tail: skipped
 *   A2a (text)                             3   ← the turn's reply tail
 *   Q2                                     4
 *   A1 (text, thinking, tool, text)        5
 *   Q1                                     6
 *   A0 (thinking, compaction)              7   ← divider tail: the answer was
 *   Q0                                        compacted into this marker
 *   N0 (a notice row)                      9
 */
const ITEMS: TimelineItem[] = [
  message("a3", "assistant", {
    segments: [text("t1"), thinking("h1"), text("t2")],
  }),
  message("q3", "user"),
  message("a2b", "assistant", { segments: [tool("w1"), thinking("h1")] }),
  message("a2a", "assistant", { segments: [text("t1")] }),
  message("q2", "user"),
  message("a1", "assistant", {
    segments: [text("t1"), thinking("h1"), tool("w1"), text("t2")],
  }),
  message("q1", "user"),
  message("a0", "assistant", { segments: [thinking("h1"), compaction("c1")] }),
  message("q0", "user"),
  { id: "n0", kind: "notice", tone: "neutral", text: "…" } as TimelineItem,
];

const LADDER = buildLadder(ITEMS, questionIndices(ITEMS)).rungs;

const rungAt = (position: number): QuestionRung => LADDER[position]!;

describe("questionIndices", () => {
  test("keeps only user messages, in view order", () => {
    expect(questionIndices(ITEMS)).toEqual([1, 4, 6, 8]);
  });

  test("an empty transcript has no questions", () => {
    expect(questionIndices([])).toEqual([]);
  });
});

describe("buildLadder", () => {
  test("each turn is reply tail, question, question end, newest first", () => {
    expect(LADDER).toEqual([
      { kind: "replyTail", index: 0, nextQuestion: null },
      { kind: "question", question: 1, tail: 0 },
      { kind: "questionEnd", question: 1, tail: 0 },
      { kind: "replyTail", index: 3, nextQuestion: 1 },
      { kind: "question", question: 4, tail: 3 },
      { kind: "questionEnd", question: 4, tail: 3 },
      { kind: "replyTail", index: 5, nextQuestion: 4 },
      { kind: "question", question: 6, tail: 5 },
      { kind: "questionEnd", question: 6, tail: 5 },
      { kind: "replyTail", index: 7, nextQuestion: 6 },
      { kind: "question", question: 8, tail: 7 },
      { kind: "questionEnd", question: 8, tail: 7 },
    ]);
  });

  test("a text slice after a tool still makes the message the reply tail", () => {
    // A1's stream ends in prose after the tool; the tail rung stands on the
    // message either way — the slice order is the renderer's business.
    expect(rungAt(6)).toEqual({ kind: "replyTail", index: 5, nextQuestion: 4 });
  });

  test("every turn with a reply gets exactly one tail rung", () => {
    // Four questions, four answers — and A2b's thinking tail does not add one.
    expect(LADDER.filter(rung => rung.kind === "replyTail")).toHaveLength(4);
  });

  test("a reply that ends on folded thinking is a pause, not a tail", () => {
    // A2b ends on thinking: skipped. The turn's tail is A2a's prose.
    expect(rungAt(3)).toEqual({ kind: "replyTail", index: 3, nextQuestion: 1 });
  });

  test("a reply compacted down to a divider still gets a tail rung", () => {
    // A0's prose is gone, but the compaction marker is where the answer was:
    // the turn keeps a tail rung rather than collapsing into the question.
    expect(rungAt(9)).toEqual({ kind: "replyTail", index: 7, nextQuestion: 6 });
  });

  test("a turn with no reply at all has no tail rung", () => {
    const items = [message("q1", "user")];
    const { rungs, lastIndex } = buildLadder(items, [0]);
    expect(rungs).toEqual([
      { kind: "question", question: 0, tail: null },
      { kind: "questionEnd", question: 0, tail: null },
    ]);
    expect(lastIndex).toBe(0);
  });

  test("a legacy reply with plain text counts as output", () => {
    const items = [
      message("a1", "assistant", { text: "the whole reply" }),
      message("q1", "user"),
    ];
    const { rungs } = buildLadder(items, [1]);
    expect(rungs[0]).toEqual({ kind: "replyTail", index: 0, nextQuestion: null });
  });

  test("an empty transcript is an empty ladder", () => {
    expect(buildLadder([], []).rungs).toEqual([]);
  });
});

describe("the ↑/↓ ladder over rungs", () => {
  // Rung positions in LADDER: 0 = A3 tail, 1 = Q3, 2 = Q3 end, 3 = A2a tail,
  // 4 = Q2, 5 = Q2 end, 6 = A1 tail, 7 = Q1, 8 = Q1 end, 9 = A0 tail, …
  test("↑ at the very newest row goes to the newest reply tail first", () => {
    // Reading at the tail itself: the tail is behind the reader, and the
    // press goes straight to the question.
    expect(previousRung(LADDER, rows(0))).toEqual(rungAt(1));
    // The top edge cutting the newest row — the jump's own approximate
    // landing included — means the answer's end is not necessarily seen: the
    // press (re-)aligns the tail before anything above it.
    expect(previousRung(LADDER, rows(0, true))).toEqual(rungAt(0));
  });

  test("↑ from a rung already on screen steps to the rung above", () => {
    expect(previousRung(LADDER, rows(1))).toEqual(rungAt(3));
    expect(previousRung(LADDER, rows(2, true))).toEqual(rungAt(3));
    expect(previousRung(LADDER, rows(3))).toEqual(rungAt(4));
  });

  test("↑ skips a questionEnd whose question is the row at the top", () => {
    // The topmost row IS Q3, cut or not: its own end is not a destination.
    expect(previousRung(LADDER, rows(1))).toEqual(rungAt(3));
    // Cut, it is already behind the reader too — the press climbs straight
    // to the reply tail above.
    expect(previousRung(LADDER, rows(1, true))).toEqual(rungAt(3));
  });

  test("↑ is unavailable at the oldest rung", () => {
    // Top edge at the last row: every rung is on or below the reader.
    expect(previousRung(LADDER, rows(9, true))).toBeNull();
    expect(previousRung(LADDER, rows(9))).toBeNull();
  });

  test("↓ from inside a turn goes to the next rung below", () => {
    // Inside A2b (top edge row 2, cut): the questionEnd below stands on Q3's
    // own row, so it is still on the way — the press after it reaches A3's
    // tail.
    expect(nextRung(LADDER, rows(2, true))).toEqual(rungAt(2));
    expect(nextRung(LADDER, rows(1, true))).toEqual(rungAt(0));
    // Top edge at Q2: Q2's own end is satisfied by the visible question, so
    // the press goes straight to the turn's reply tail.
    expect(nextRung(LADDER, rows(4))).toEqual(rungAt(3));
  });

  test("↓ never selects the rung at the top edge", () => {
    expect(nextRung(LADDER, rows(5))).toEqual(rungAt(5));
    expect(nextRung(LADDER, rows(4, true))).toEqual(rungAt(3));
    expect(nextRung(LADDER, rows(6))).toEqual(rungAt(6));
  });

  test("↓ is unavailable inside the newest turn", () => {
    expect(nextRung(LADDER, rows(0, true))).toBeNull();
    expect(previousRung(LADDER, rows(0, true))).not.toBeNull();
  });

  test("nothing is offered without a visible row", () => {
    expect(previousRung(LADDER, EMPTY_ROWS)).toBeNull();
    expect(nextRung(LADDER, EMPTY_ROWS)).toBeNull();
  });

  test("the ladder's individual steps are all selectable", () => {
    // The user-facing contract, pinned step by step: from the newest turn
    // down to the oldest, ↑ offers reply tail, question, question end…
    expect(previousRung(LADDER, rows(0, true))).toEqual(rungAt(0)); // A3 tail
    expect(previousRung(LADDER, rows(0))).toEqual(rungAt(1)); // Q3
    expect(previousRung(LADDER, rows(1, true))).toEqual(rungAt(3)); // cut: climbs
    expect(previousRung(LADDER, rows(1))).toEqual(rungAt(3)); // A2a tail
    expect(previousRung(LADDER, rows(3, true))).toEqual(rungAt(3)); // re-align
    expect(previousRung(LADDER, rows(3))).toEqual(rungAt(4)); // Q2
    expect(previousRung(LADDER, rows(4, true))).toEqual(rungAt(6)); // cut: climbs
    expect(previousRung(LADDER, rows(4))).toEqual(rungAt(6)); // A1 tail
    expect(previousRung(LADDER, rows(5, true))).toEqual(rungAt(6)); // re-align
    expect(previousRung(LADDER, rows(6))).toEqual(rungAt(9)); // A0 tail
    expect(previousRung(LADDER, rows(7, true))).toEqual(rungAt(9)); // re-align
    expect(previousRung(LADDER, rows(7))).toEqual(rungAt(10)); // Q0
    expect(previousRung(LADDER, rows(8, true))).toBeNull(); // cut at the oldest

    // …and ↓ offers the mirror sequence back down.
    expect(nextRung(LADDER, rows(9))).toEqual(rungAt(11)); // Q0's own end
    expect(nextRung(LADDER, rows(8))).toEqual(rungAt(9)); // A0 tail
    expect(nextRung(LADDER, rows(7))).toEqual(rungAt(8)); // Q1 — the question
    //   itself is still below the reader parked on its tail's row
    expect(nextRung(LADDER, rows(6))).toEqual(rungAt(6)); // A1 tail
    expect(nextRung(LADDER, rows(5))).toEqual(rungAt(5)); // Q2's own end
    expect(nextRung(LADDER, rows(4))).toEqual(rungAt(3)); // A2a tail
    expect(nextRung(LADDER, rows(3))).toEqual(rungAt(2)); // Q3's own end
    expect(nextRung(LADDER, rows(1))).toEqual(rungAt(0)); // A3 tail
  });
});

describe("viewportRows", () => {
  test("a topmost row that is not fully visible is cut by the top edge", () => {
    expect(viewportRows({ partialTop: 4, fullTop: 3 }).topClipped).toBe(true);
  });

  test("a topmost row that is fully visible has its start on screen", () => {
    expect(viewportRows({ partialTop: 4, fullTop: 4 })).toEqual({
      top: 4,
      topClipped: false,
    });
  });

  test("no fully visible row means the only visible row is cut", () => {
    expect(viewportRows({ partialTop: 7, fullTop: null }).topClipped).toBe(true);
  });
});
