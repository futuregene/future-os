import type { TimelineItem } from "../../../remote/types";
import { nextQuestion, previousQuestion, questionIndices } from "../questionNav";

const message = (
  id: string,
  role: "user" | "assistant",
  extra: Record<string, unknown> = {},
): TimelineItem =>
  ({ id, kind: "message", role, text: "", ...extra }) as unknown as TimelineItem;

/**
 * The fixture transcript, newest first:
 *
 *   A3 (a reply)   0
 *   Q3             1
 *   A2 (a reply)   2
 *   Q2             3
 *   A1 (a reply)   4
 *   Q1             5
 *   Q0             6
 *   N0 (a notice)  7
 */
const ITEMS: TimelineItem[] = [
  message("a3", "assistant"),
  message("q3", "user"),
  message("a2", "assistant"),
  message("q2", "user"),
  message("a1", "assistant"),
  message("q1", "user"),
  message("q0", "user"),
  { id: "n0", kind: "notice", tone: "neutral", text: "…" } as TimelineItem,
];

const QUESTIONS = questionIndices(ITEMS);

describe("questionIndices", () => {
  test("keeps only user messages, in view order", () => {
    expect(QUESTIONS).toEqual([1, 3, 5, 6]);
  });

  test("an empty transcript has no questions", () => {
    expect(questionIndices([])).toEqual([]);
  });
});

describe("the ↑/↓ walk over prompts", () => {
  test("↑ at the tail offers the prompt of the turn on screen", () => {
    // The newest reply fills the screen: the reader is inside it, so the first
    // press climbs to what started that turn.
    expect(previousQuestion(QUESTIONS, 0)).toBe(1);
  });

  test("↑ offers the reader's own prompts, never the answers", () => {
    // With Q3 at the top edge the answer below it (A2, the reply to Q2) is not a
    // destination: the press reaches the prompt above the reader instead.
    expect(previousQuestion(QUESTIONS, 1)).toBe(3); // Q3 → Q2
    expect(previousQuestion(QUESTIONS, 3)).toBe(5); // Q2 → Q1
    expect(previousQuestion(QUESTIONS, 5)).toBe(6); // Q1 → Q0
    // The same from inside an answer: the prompt that produced it is up there.
    expect(previousQuestion(QUESTIONS, 2)).toBe(3); // inside A2 → Q2
    expect(previousQuestion(QUESTIONS, 4)).toBe(5); // inside A1 → Q1
  });

  test("the row at the top edge is the reader's, not a destination", () => {
    // A prompt landed at the top edge (the press before this one put it there)
    // is not re-selected, in either direction.
    expect(previousQuestion(QUESTIONS, 3)).toBe(5); // Q2 → Q1
    expect(nextQuestion(QUESTIONS, 3)).toBe(1); // Q2 → Q3
  });

  test("↑ is unavailable at the oldest row", () => {
    expect(previousQuestion(QUESTIONS, 6)).toBeNull();
    expect(previousQuestion(QUESTIONS, 7)).toBeNull();
  });

  test("↓ never selects the row at the top edge", () => {
    // The row whose own row is at the top edge is where the reader is, so ↓
    // steps to the newest prompt strictly below it.
    expect(nextQuestion(QUESTIONS, 3)).toBe(1);
    expect(nextQuestion(QUESTIONS, 1)).toBeNull();
    expect(nextQuestion(QUESTIONS, 0)).toBeNull();
  });

  test("nothing is offered without a visible row", () => {
    expect(previousQuestion(QUESTIONS, null)).toBeNull();
    expect(nextQuestion(QUESTIONS, null)).toBeNull();
  });

  test("↑ walks prompt to prompt down to the oldest one", () => {
    // The user-facing contract of the ↑ button, pinned step by step: every press
    // reaches a user input, and the answers in between are never a stop.
    expect(previousQuestion(QUESTIONS, 0)).toBe(1); // Q3
    expect(previousQuestion(QUESTIONS, 1)).toBe(3); // Q2, past A2
    expect(previousQuestion(QUESTIONS, 3)).toBe(5); // Q1
    expect(previousQuestion(QUESTIONS, 5)).toBe(6); // Q0
    expect(previousQuestion(QUESTIONS, 6)).toBeNull(); // the oldest is the reader's
  });

  test("↓ walks prompt to prompt, skipping every answer in between", () => {
    // From the oldest prompt downwards, each press is the next thing the reader
    // *asked*: the answers between two prompts are passed in one press.
    expect(nextQuestion(QUESTIONS, 6)).toBe(5); // Q0 → Q1, past A1
    expect(nextQuestion(QUESTIONS, 5)).toBe(3); // Q1 → Q2, past A2
    expect(nextQuestion(QUESTIONS, 4)).toBe(3); // inside A1 → Q2
    expect(nextQuestion(QUESTIONS, 3)).toBe(1); // Q2 → Q3, past A3
    expect(nextQuestion(QUESTIONS, 2)).toBe(1); // inside A2 → Q3
    expect(nextQuestion(QUESTIONS, 1)).toBeNull(); // the newest prompt is the last stop
  });
});
