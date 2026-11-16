import { describe, expect, it } from "vitest";
import { splitChoiceAnswerEcho } from "./choiceAnswerEcho";

describe("splitChoiceAnswerEcho", () => {
  it("reads the answer a pick prints and leaves the rest alone", () => {
    expect(splitChoiceAnswerEcho("[接下来先做哪一块？ → 把归因搬进页面] 那个英伟达的归因做好了吗？"))
      .toEqual({
        echoes: [{ answer: "把归因搬进页面", question: "接下来先做哪一块？" }],
        rest: "那个英伟达的归因做好了吗？",
      });
  });

  it("reads an answer that is the whole prompt, with nothing typed after it", () => {
    expect(splitChoiceAnswerEcho("[重画森林图 → 按 Dreher 2025 重算]")).toEqual({
      echoes: [{ answer: "按 Dreher 2025 重算", question: "重画森林图" }],
      rest: "",
    });
  });

  it("keeps a multi-choice answer as the printer joined it", () => {
    expect(splitChoiceAnswerEcho("[这三项都要？ → 重画森林图、补一张流程图]")).toEqual({
      echoes: [{ answer: "重画森林图、补一张流程图", question: "这三项都要？" }],
      rest: "",
    });
  });

  it("reads a run of answers and stops at the first thing that is not one", () => {
    const split = splitChoiceAnswerEcho("[先做哪一块？ → 归因]\n[再呢？ → 采集] 然后再说");
    expect(split.echoes.map(echo => echo.answer)).toEqual(["归因", "采集"]);
    expect(split.rest).toBe("然后再说");
  });

  it("leaves prose that merely starts with a bracket alone", () => {
    const text = "[更新日志] 修了那个归因";
    expect(splitChoiceAnswerEcho(text)).toEqual({ echoes: [], rest: text });
  });

  it("leaves a markdown link or a nested bracket unread", () => {
    const link = "[标题 → 答案](https://example.com)";
    expect(splitChoiceAnswerEcho(link)).toEqual({ echoes: [], rest: link });
    const nested = "[a → b] [c]";
    expect(splitChoiceAnswerEcho(nested).echoes).toEqual([{ answer: "b", question: "a" }]);
    expect(splitChoiceAnswerEcho("[a → b] [c").rest).toBe("[c");
  });

  it("takes no answer from the middle of a prompt", () => {
    const text = "那个英伟达的归因做好了吗？[接下来先做哪一块？ → 把归因搬进页面]";
    expect(splitChoiceAnswerEcho(text)).toEqual({ echoes: [], rest: text });
  });

  it("rejects an empty half, so `[a → ]` stays text", () => {
    const text = "[a → ]";
    expect(splitChoiceAnswerEcho(text)).toEqual({ echoes: [], rest: text });
  });

  it("reads at most the four answers a card can offer", () => {
    const run = Array.from({ length: 6 }, (_, index) => `[q${index} → a${index}]`).join(" ");
    const split = splitChoiceAnswerEcho(run);
    expect(split.echoes).toHaveLength(4);
    expect(split.rest).toBe("[q4 → a4] [q5 → a5]");
  });
});
