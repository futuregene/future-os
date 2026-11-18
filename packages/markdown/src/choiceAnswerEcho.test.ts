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

  it("reads an answer the reader typed around, and closes the seam it leaves", () => {
    // The composer writes the pill at the caret, so a cursor left mid-sentence
    // puts the answer inside the reader's own words. The tag is no less right
    // there, and the words keep their order on both sides of it.
    expect(splitChoiceAnswerEcho("另外 asrbench 可以删除[删哪些？ → 删 A 类全部、删未在用的 ~/omlx 副本]"))
      .toEqual({
        echoes: [{ answer: "删 A 类全部、删未在用的 ~/omlx 副本", question: "删哪些？" }],
        rest: "另外 asrbench 可以删除",
      });
    expect(splitChoiceAnswerEcho("先说[顺手做？ → 全部做]然后再说")).toEqual({
      echoes: [{ answer: "全部做", question: "顺手做？" }],
      rest: "先说然后再说",
    });
  });

  it("takes an answer out of two clauses without joining them or double-spacing", () => {
    expect(splitChoiceAnswerEcho("我想想 [先做哪块？ → 归因] 再说下一步")).toEqual({
      echoes: [{ answer: "归因", question: "先做哪块？" }],
      rest: "我想想 再说下一步",
    });
    expect(splitChoiceAnswerEcho("其一\n[先做哪块？ → 归因]\n其二")).toEqual({
      echoes: [{ answer: "归因", question: "先做哪块？" }],
      rest: "其一\n其二",
    });
  });

  it("reads several answers whatever they sit between", () => {
    const split = splitChoiceAnswerEcho("甲[q1 → a1]乙[q2 → a2]丙");
    expect(split.echoes).toEqual([{ answer: "a1", question: "q1" }, { answer: "a2", question: "q2" }]);
    expect(split.rest).toBe("甲乙丙");
  });

  it("leaves prose that merely starts with a bracket alone", () => {
    const text = "[更新日志] 修了那个归因";
    expect(splitChoiceAnswerEcho(text)).toEqual({ echoes: [], rest: text });
  });

  it("leaves prose that brackets a path alone — no arrow, no answer", () => {
    // The two halves of the same rule: a normal prose bracket is not a tag here,
    // and `bracketMentionPath` keeps it out of the file-reference promotion in
    // the parser. A path inside the brackets does not make either happen.
    const text = "另外 [见 docs/crawl.md] 那段";
    expect(splitChoiceAnswerEcho(text)).toEqual({ echoes: [], rest: text });
  });

  it("leaves a markdown link or a nested bracket unread", () => {
    const link = "[标题 → 答案](https://example.com)";
    expect(splitChoiceAnswerEcho(link)).toEqual({ echoes: [], rest: link });
    const nested = "[a → b] [c]";
    expect(splitChoiceAnswerEcho(nested).echoes).toEqual([{ answer: "b", question: "a" }]);
    expect(splitChoiceAnswerEcho("[a → b] [c").rest).toBe("[c");
    // A link in the middle does not stop the scan: the answer after it is still
    // an answer, and the link is left where the reader put it.
    expect(splitChoiceAnswerEcho("见 [文档 → 链接](https://x.com) 然后[q → a]"))
      .toEqual({
        echoes: [{ answer: "a", question: "q" }],
        rest: "见 [文档 → 链接](https://x.com) 然后",
      });
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

  it("keeps a bracketed arrow inside a fenced block as the reader's own example", () => {
    // Cutting one out would leave the fence unbalanced and turn the code above
    // it into prose, so a pasted example is never an answer.
    const fenced = "看看这个：\n\n```ts\nconst m = [a → b];\n```\n\n用哪种写法？";
    expect(splitChoiceAnswerEcho(fenced)).toEqual({ echoes: [], rest: fenced });
    // The answer after the fence is still read, and the block is untouched.
    const both = "```\n[x → y]\n```\n[q1 → a1] 你觉得呢";
    expect(splitChoiceAnswerEcho(both)).toEqual({
      echoes: [{ answer: "a1", question: "q1" }],
      rest: "```\n[x → y]\n```\n你觉得呢",
    });
  });

  it("keeps a bracketed arrow inside an inline code span as code", () => {
    const text = "把 `[q → a]` 这种写法改成卡片";
    expect(splitChoiceAnswerEcho(text)).toEqual({ echoes: [], rest: text });
    // An unpaired backtick is literal, so it does not hide what follows.
    expect(splitChoiceAnswerEcho("一个 ` 号 [q → a]").echoes).toEqual([{ answer: "a", question: "q" }]);
  });
});
