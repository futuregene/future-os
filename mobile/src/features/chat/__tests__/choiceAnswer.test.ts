import type { ChoiceCardNode } from "@future-os/markdown";
import {
  choiceAnswerFromCard,
  choiceAnswerLabel,
  choiceAnswerText,
  parseChoiceAnswer,
  pickChoiceOption,
} from "../choiceAnswer";

const card: ChoiceCardNode = {
  mode: "single",
  options: [
    { description: "按 Dreher 2025 的样本量重算置信区间", id: "forest", label: "重画森林图" },
    { id: "flow", label: "补一张实验流程图" },
    { id: "table", label: "表里加一列样本量" },
  ],
  title: "接下来做哪一件？",
  type: "choiceCard",
};

describe("the answer a choice card produces", () => {
  test("carries the card's question and every option, with nothing picked yet", () => {
    expect(choiceAnswerFromCard(card)).toEqual({
      mode: "single",
      options: [
        { id: "forest", label: "重画森林图" },
        { id: "flow", label: "补一张实验流程图" },
        { id: "table", label: "表里加一列样本量" },
      ],
      selectedIds: [],
      title: "接下来做哪一件？",
    });
  });

  test("single choice replaces the pick, multi choice toggles it (§6 b)", () => {
    const single = pickChoiceOption(choiceAnswerFromCard(card), "forest");
    expect(single.selectedIds).toEqual(["forest"]);
    expect(pickChoiceOption(single, "flow").selectedIds).toEqual(["flow"]);

    const multi = { ...choiceAnswerFromCard({ ...card, mode: "multi" }) };
    const first = pickChoiceOption(multi, "forest");
    const both = pickChoiceOption(first, "flow");
    expect(both.selectedIds).toEqual(["forest", "flow"]);
    expect(pickChoiceOption(both, "forest").selectedIds).toEqual(["flow"]);
  });

  test("the label joins picks in the order they were made, the text names the question", () => {
    const multi = choiceAnswerFromCard({ ...card, mode: "multi" });
    const answer = pickChoiceOption(pickChoiceOption(multi, "forest"), "flow");
    expect(choiceAnswerLabel(answer)).toBe("重画森林图、补一张实验流程图");
    // §7: plain text, so every degraded client can read it.
    expect(choiceAnswerText(answer)).toBe("[接下来做哪一件？ → 重画森林图、补一张实验流程图]");
  });

  test("a stored answer is read back, and a malformed one is dropped whole", () => {
    const answer = pickChoiceOption(choiceAnswerFromCard(card), "forest");
    expect(parseChoiceAnswer(JSON.parse(JSON.stringify(answer)))).toEqual(answer);
    // Unknown option ids are dropped rather than restored: an answer that cannot
    // print itself would send a sentence nobody asked for.
    expect(parseChoiceAnswer({ ...answer, selectedIds: ["gone"] })).toBeNull();
    expect(parseChoiceAnswer({ ...answer, title: "  " })).toBeNull();
    expect(parseChoiceAnswer({ ...answer, mode: "several" })).toBeNull();
    expect(parseChoiceAnswer({ ...answer, options: [] })).toBeNull();
    expect(parseChoiceAnswer({ ...answer, options: [{ id: "forest" }] })).toBeNull();
    expect(parseChoiceAnswer(null)).toBeNull();
    expect(parseChoiceAnswer("{}")).toBeNull();
  });
});
