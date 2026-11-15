import type { ChoiceAnswer } from "./choiceAnswer";
import { describe, expect, it } from "vitest";
import { answerFromPick, choiceAnswerText, parseChoiceAnswer } from "./choiceAnswer";

/** The card as a click hands it on: question, mode and every row it offers. */
const CARD = {
  mode: "single" as const,
  options: [
    { description: "按 Dreher 2025 的样本量重算", id: "forest", label: "重画森林图" },
    { id: "flow", label: "补一张实验流程图" },
  ],
  title: "接下来做哪一件？",
};

describe("answerFromPick", () => {
  it("learns the question from the first pick", () => {
    const answer = answerFromPick(null, CARD, "forest");
    expect(answer.selectedIds).toEqual(["forest"]);
    expect(answer.title).toBe("接下来做哪一件？");
    expect(answer.mode).toBe("single");
    // The pills only need labels: the explanations are the card's, not the
    // answer's (the popover drops them after they have been read once).
    expect(answer.options).toEqual([
      { id: "forest", label: "重画森林图" },
      { id: "flow", label: "补一张实验流程图" },
    ]);
    expect(choiceAnswerText(answer)).toBe("[接下来做哪一件？ → 重画森林图]");
  });

  it("replaces the pick on a single-choice card and toggles on a multi one", () => {
    const first = answerFromPick(null, CARD, "forest");
    expect(answerFromPick(first, CARD, "flow").selectedIds).toEqual(["flow"]);

    const multi = answerFromPick(null, { ...CARD, mode: "multi" }, "forest");
    expect(answerFromPick(multi, { ...CARD, mode: "multi" }, "flow").selectedIds).toEqual(["forest", "flow"]);
    // Picking a picked row takes it back out (§6 b).
    const both = answerFromPick(answerFromPick(multi, { ...CARD, mode: "multi" }, "flow"), { ...CARD, mode: "multi" }, "forest");
    expect(both.selectedIds).toEqual(["flow"]);
  });

  it("starts over when the pick comes from a different question", () => {
    const previous = answerFromPick(null, CARD, "forest");
    const other = answerFromPick(previous, { ...CARD, title: "换个问法？" }, "flow");
    // The old question's title must not ride into the new answer: it is what gets
    // printed on send, and a title nobody asked would misattribute the pick.
    expect(other.title).toBe("换个问法？");
    expect(other.selectedIds).toEqual(["flow"]);
  });

  it("produces an answer the pill can read back", () => {
    const answer = answerFromPick(null, CARD, "flow");
    expect(parseChoiceAnswer(JSON.stringify(answer))).toEqual(answer as ChoiceAnswer);
  });
});
