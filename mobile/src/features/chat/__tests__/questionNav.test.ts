import type { TimelineItem } from "../../../remote/types";
import {
  buildLadder,
  nextRung,
  previousQuestion,
  questionIndices,
  rungIndex,
  rungPosition,
  type QuestionRung,
} from "../questionNav";

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
 *   A3 (text, thinking, text)              0   ← the newest turn's reply tail
 *   Q3                                     1
 *   A2b (tool, thinking)                   2   ← thinking tail: skipped
 *   A2a (text)                             3   ← that turn's reply tail
 *   Q2                                     4
 *   A1 (text, thinking, tool, text)        5
 *   Q1                                     6
 *   A0 (thinking, compaction)              7   ← divider tail: the answer was
 *                                               compacted into this marker
 *   Q0                                     8
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

const LADDER = buildLadder(ITEMS, questionIndices(ITEMS));

/** Rung positions in LADDER: 0 = A3 tail, 1 = Q3, 2 = A2a tail, 3 = Q2,
 *  4 = A1 tail, 5 = Q1, 6 = A0 tail, 7 = Q0. */
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
  test("each turn is its reply's end then its question, newest first", () => {
    expect(LADDER).toEqual([
      { kind: "replyTail", index: 0 },
      { kind: "question", question: 1 },
      { kind: "replyTail", index: 3 },
      { kind: "question", question: 4 },
      { kind: "replyTail", index: 5 },
      { kind: "question", question: 6 },
      { kind: "replyTail", index: 7 },
      { kind: "question", question: 8 },
    ]);
  });

  test("a text slice after a tool still makes the message the reply tail", () => {
    // A1's stream ends in prose after the tool; the tail rung stands on the
    // message either way — the slice order is the renderer's business.
    expect(rungAt(4)).toEqual({ kind: "replyTail", index: 5 });
  });

  test("every turn with a reply gets exactly one tail rung", () => {
    // Four questions, four answers — and A2b's thinking tail does not add one.
    expect(LADDER.filter(rung => rung.kind === "replyTail")).toHaveLength(4);
  });

  test("a reply that ends on folded thinking is a pause, not a tail", () => {
    // A2b ends on thinking: skipped. The turn's tail is A2a's prose.
    expect(rungAt(2)).toEqual({ kind: "replyTail", index: 3 });
  });

  test("a reply compacted down to a divider still gets a tail rung", () => {
    // A0's prose is gone, but the compaction marker is where the answer was:
    // the turn keeps a tail rung rather than collapsing into the question.
    expect(rungAt(6)).toEqual({ kind: "replyTail", index: 7 });
  });

  test("a turn with no reply at all has no tail rung", () => {
    expect(buildLadder([message("q1", "user")], [0])).toEqual([
      { kind: "question", question: 0 },
    ]);
  });

  test("a legacy reply with plain text counts as output", () => {
    const items = [
      message("a1", "assistant", { text: "the whole reply" }),
      message("q1", "user"),
    ];
    expect(buildLadder(items, [1])[0]).toEqual({
      kind: "replyTail",
      index: 0,
    });
  });

  test("an empty transcript is an empty ladder", () => {
    expect(buildLadder([], [])).toEqual([]);
  });
});

describe("where a rung lands", () => {
  test("a rung lands on its own row, question and reply alike", () => {
    // The landing is the row's *start* for everything, questions and replies
    // (the inverted list's `viewPosition: 1`; the hook turns this into the
    // scroll). A reply is then lifted inside its row to its last output block by
    // the offset the bubble measured there — pinned in useQuestionNav's suite.
    expect(rungIndex(rungAt(1))).toBe(1);
    expect(rungIndex(rungAt(2))).toBe(3);
    expect(rungIndex(rungAt(4))).toBe(5);
  });

  test("the transcript's newest row is not special", () => {
    // View index 0 is the newest row — the one a freshly opened conversation is
    // resting at — and it lands like any other: its own top edge, with the
    // list's tail (and the composer's own "back to latest") below it.
    expect(rungIndex(rungAt(0))).toBe(0);
    expect(rungIndex({ kind: "question", question: 0 })).toBe(0);
  });

  test("a rung is found by what it is, not by where it sat", () => {
    // A rebuilt ladder (a new turn, a paged page) keeps its rungs but not their
    // positions, so the walk re-anchors on the rung itself.
    expect(rungPosition(LADDER, rungAt(4))).toBe(4);
    expect(rungPosition(LADDER, { kind: "question", question: 99 })).toBeNull();
    expect(rungPosition(LADDER, null)).toBeNull();
    expect(
      rungPosition(LADDER, { kind: "replyTail", index: 4 }),
    ).toBeNull();
  });
});

describe("the ↑/↓ walk over rungs", () => {
  test("↑ at the tail offers the prompt of the turn on screen", () => {
    // The newest reply fills the screen: the reader is inside it, so the first
    // press climbs to what started that turn.
    expect(previousQuestion(LADDER, 0)).toEqual(rungAt(1));
  });

  test("↑ offers the reader's own prompts, never the answers", () => {
    // With Q3 at the top edge the answer below it (A2a, the reply to Q2) is not
    // a destination: the press reaches the prompt above the reader instead.
    expect(previousQuestion(LADDER, 1)).toEqual(rungAt(3)); // Q3 → Q2
    expect(previousQuestion(LADDER, 4)).toEqual(rungAt(5)); // Q2 → Q1
    expect(previousQuestion(LADDER, 6)).toEqual(rungAt(7)); // Q1 → Q0
    // The same from inside an answer: the prompt that produced it (or, for a
    // row with no prompt of its own seen yet, the nearest one above) is up
    // there, so that is where the press lands.
    expect(previousQuestion(LADDER, 2)).toEqual(rungAt(3)); // inside A2b → Q2
    expect(previousQuestion(LADDER, 4)).toEqual(rungAt(5)); // inside A1 → Q1
  });

  test("the row at the top edge is the reader's, not a destination", () => {
    // A prompt landed at the top edge (the press before this one put it there)
    // is not re-selected: the press climbs to the prompt above it.
    expect(previousQuestion(LADDER, 1)).toEqual(rungAt(3));
    expect(previousQuestion(LADDER, 6)).toEqual(rungAt(7));
  });

  test("↑ is unavailable at the oldest row", () => {
    // Top edge at the last row: every rung is on or below the reader.
    expect(previousQuestion(LADDER, 8)).toBeNull();
    expect(previousQuestion(LADDER, 9)).toBeNull();
  });

  test("↓ from inside a turn goes to the next rung below, of either kind", () => {
    expect(nextRung(LADDER, 8)).toEqual(rungAt(6));
    expect(nextRung(LADDER, 7)).toEqual(rungAt(5));
    expect(nextRung(LADDER, 6)).toEqual(rungAt(4));
    expect(nextRung(LADDER, 5)).toEqual(rungAt(3));
    expect(nextRung(LADDER, 4)).toEqual(rungAt(2));
    expect(nextRung(LADDER, 3)).toEqual(rungAt(1));
    expect(nextRung(LADDER, 2)).toEqual(rungAt(1));
  });

  test("↓ never selects the rung at the top edge", () => {
    // The rung whose own row is at the top edge is where the reader is, so ↓
    // steps to the newest rung strictly below.
    expect(nextRung(LADDER, 1)).toEqual(rungAt(0));
    expect(nextRung(LADDER, 0)).toBeNull();
  });

  test("nothing is offered without a visible row", () => {
    expect(previousQuestion(LADDER, null)).toBeNull();
    expect(nextRung(LADDER, null)).toBeNull();
  });

  test("↑ walks prompt to prompt down to the oldest one", () => {
    // The user-facing contract of the ↑ button, pinned step by step: every press
    // reaches a user input, and the answers in between are never a stop.
    expect(previousQuestion(LADDER, 0)).toEqual(rungAt(1)); // Q3
    expect(previousQuestion(LADDER, 1)).toEqual(rungAt(3)); // Q2, past A2a
    expect(previousQuestion(LADDER, 4)).toEqual(rungAt(5)); // Q1
    expect(previousQuestion(LADDER, 6)).toEqual(rungAt(7)); // Q0
    expect(previousQuestion(LADDER, 8)).toBeNull(); // the oldest prompt is the reader's
  });

  test("↓ walks the transcript as written, input and output alternating", () => {
    // From the oldest prompt downwards, each press is the next thing the
    // conversation contains: that prompt's answer, then the next prompt, then
    // its answer…
    expect(nextRung(LADDER, 8)).toEqual(rungAt(6)); // A0 — the reply to Q0
    expect(nextRung(LADDER, 7)).toEqual(rungAt(5)); // Q1
    expect(nextRung(LADDER, 6)).toEqual(rungAt(4)); // A1 — the reply to Q1
    expect(nextRung(LADDER, 5)).toEqual(rungAt(3)); // Q2
    expect(nextRung(LADDER, 4)).toEqual(rungAt(2)); // A2a — the reply to Q2
    expect(nextRung(LADDER, 3)).toEqual(rungAt(1)); // Q3
    expect(nextRung(LADDER, 2)).toEqual(rungAt(1)); // Q3, reached from the answer
    expect(nextRung(LADDER, 1)).toEqual(rungAt(0)); // A3 — the newest reply
    expect(nextRung(LADDER, 0)).toBeNull();
  });
});
