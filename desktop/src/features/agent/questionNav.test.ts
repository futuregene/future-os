import type { TranscriptMessage } from "./questionNav";
import { describe, expect, it } from "vitest";
import { nextQuestion, previousQuestion, questionIndices } from "./questionNav";

function user(id: string): TranscriptMessage {
  return { id, role: "user" };
}

function assistant(id: string): TranscriptMessage {
  return { id, role: "assistant" };
}

/**
 * The fixture transcript, in the order it is drawn (oldest first):
 *
 *   0 u1     the first prompt
 *   1 a1     …its (long) reply
 *   2 u2     the second prompt
 *   3 a2     …its reply
 *   4 u3     the third prompt
 *   5 a3     …its reply
 *   6 u4     the fourth prompt, still unanswered
 */
const MESSAGES: TranscriptMessage[] = [
  user("u1"),
  assistant("a1"),
  user("u2"),
  assistant("a2"),
  user("u3"),
  assistant("a3"),
  user("u4"),
];

const QUESTIONS = questionIndices(MESSAGES);

describe("questionIndices", () => {
  it("keeps the reader's own prompts and nothing else", () => {
    expect(QUESTIONS).toEqual([0, 2, 4, 6]);
  });

  it("is empty for an empty transcript", () => {
    expect(questionIndices([])).toEqual([]);
  });
});

describe("the ↑/↓ walk over prompts", () => {
  it("offers nothing without a reader position", () => {
    expect(previousQuestion(QUESTIONS, null)).toBeNull();
    expect(nextQuestion(QUESTIONS, null)).toBeNull();
  });

  it("↑ offers the closest prompt above the reader", () => {
    // From inside the second turn's reply, the press reaches the prompt that
    // produced it — not the first prompt, and not the answer around it.
    expect(previousQuestion(QUESTIONS, 3)).toBe(2);
    // From the second prompt itself, the first one.
    expect(previousQuestion(QUESTIONS, 2)).toBe(0);
    // From inside the first turn's reply, its own prompt.
    expect(previousQuestion(QUESTIONS, 1)).toBe(0);
  });

  it("↓ offers the closest prompt below the reader, one answer skipped", () => {
    // The transcript as asked: prompt, prompt, prompt — the reply between two
    // of them is passed in a single press, however tall it is.
    expect(nextQuestion(QUESTIONS, 0)).toBe(2); // u1 → u2, past a1
    expect(nextQuestion(QUESTIONS, 1)).toBe(2); // inside a1 → u2
    expect(nextQuestion(QUESTIONS, 2)).toBe(4); // u2 → u3, past a2
    expect(nextQuestion(QUESTIONS, 3)).toBe(4); // inside a2 → u3
    expect(nextQuestion(QUESTIONS, 4)).toBe(6); // u3 → u4, past a3
    expect(nextQuestion(QUESTIONS, 5)).toBe(6); // inside a3 → u4
  });

  it("the row at the top edge is the reader's, not a destination", () => {
    // A prompt landed at the top edge (the press before this one put it there)
    // is not re-selected, in either direction.
    expect(previousQuestion(QUESTIONS, 2)).toBe(0);
    expect(nextQuestion(QUESTIONS, 2)).toBe(4);
  });

  it("runs out at either end of the walk", () => {
    expect(previousQuestion(QUESTIONS, 0)).toBeNull();
    expect(nextQuestion(QUESTIONS, 6)).toBeNull();
    // A reply row after the newest prompt is not a stop either.
    expect(nextQuestion(QUESTIONS, 7)).toBeNull();
  });

  it("is one walk read both ways", () => {
    // ↓ then ↑ returns to where the reader came from, because both directions
    // land on the same prompts.
    const down = nextQuestion(QUESTIONS, 1);
    expect(down).toBe(2);
    expect(previousQuestion(QUESTIONS, down!)).toBe(0);
  });
});
