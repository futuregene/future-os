import type { TimelineItem } from "../../../remote/types";
import {
  EMPTY_ROWS,
  nextQuestion,
  previousQuestion,
  questionIndices,
  viewportRows,
} from "../questionNav";

const rows = (top: number | null, topClipped = false) => ({ top, topClipped });

// View order for a four-turn conversation: the newest question is index 2, the
// oldest is index 11.
const QUESTIONS = [2, 5, 8, 11];

describe("questionIndices", () => {
  test("keeps only user messages, in view order", () => {
    const items = [
      { id: "a", kind: "message", role: "assistant" },
      { id: "b", kind: "message", role: "user" },
      { id: "c", kind: "tool", role: "tool" },
      { id: "d", kind: "message", role: "user" },
    ] as unknown as TimelineItem[];
    expect(questionIndices(items)).toEqual([1, 3]);
  });

  test("an empty transcript has no questions", () => {
    expect(questionIndices([])).toEqual([]);
  });
});

describe("the ↑/↓ ladder over question starts", () => {
  test("↑ inside an answer goes to the question that starts it", () => {
    // Top edge is inside the newest answer, below the newest question.
    expect(previousQuestion(QUESTIONS, rows(0, true))).toBe(2);
    expect(previousQuestion(QUESTIONS, rows(1, true))).toBe(2);
  });

  test("↑ from a question already on screen steps to the one above", () => {
    expect(previousQuestion(QUESTIONS, rows(2))).toBe(5);
    expect(previousQuestion(QUESTIONS, rows(5))).toBe(8);
  });

  test("↑ is unavailable at the oldest question", () => {
    expect(previousQuestion(QUESTIONS, rows(11))).toBeNull();
  });

  test("↓ from inside an answer goes to the next question below", () => {
    // Inside the third answer: the next question down is the fourth.
    expect(nextQuestion(QUESTIONS, rows(4, true))).toBe(2);
    expect(nextQuestion(QUESTIONS, rows(7, true))).toBe(5);
  });

  test("↓ is unavailable inside the newest answer", () => {
    // Nothing newer to go to, even though ↑ still offers this question.
    expect(nextQuestion(QUESTIONS, rows(1, true))).toBeNull();
    expect(previousQuestion(QUESTIONS, rows(1, true))).toBe(2);
  });

  test("↓ never selects the row at the top edge", () => {
    expect(nextQuestion(QUESTIONS, rows(5))).toBe(2);
    expect(nextQuestion(QUESTIONS, rows(11))).toBe(8);
  });

  test("↓ is unavailable at the newest question", () => {
    expect(nextQuestion(QUESTIONS, rows(2))).toBeNull();
  });

  test("nothing is offered without a visible row", () => {
    expect(previousQuestion(QUESTIONS, EMPTY_ROWS)).toBeNull();
    expect(nextQuestion(QUESTIONS, EMPTY_ROWS)).toBeNull();
  });

  test("a landed question is neither direction's target, so a press always moves", () => {
    for (const question of QUESTIONS) {
      const landed = rows(question);
      expect(previousQuestion(QUESTIONS, landed)).not.toBe(question);
      expect(nextQuestion(QUESTIONS, landed)).not.toBe(question);
    }
  });

  test("↓ after ↑ returns exactly one question, and the other way round", () => {
    // Walk the ladder upward from the newest question, then back down: every
    // step is a single question, which is what "previous/next question" means.
    let question = 2;
    const climbed = [question];
    for (;;) {
      const up = previousQuestion(QUESTIONS, rows(question));
      if (up === null) break;
      question = up;
      climbed.push(question);
    }
    expect(climbed).toEqual([2, 5, 8, 11]);

    const descended: number[] = [];
    while (true) {
      const down = nextQuestion(QUESTIONS, rows(question));
      if (down === null) break;
      descended.push(down);
      question = down;
    }
    expect(descended).toEqual([8, 5, 2]);
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

  test("without a partly visible row there is no anchor", () => {
    expect(viewportRows({ partialTop: null, fullTop: null })).toEqual(EMPTY_ROWS);
  });
});
