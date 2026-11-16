import { describe, expect, it } from "vitest";
import { parseFutureMarkdown } from "./parseFutureMarkdown";

function first(source: string) {
  return parseFutureMarkdown(source).nodes[0];
}

const fence = (body: string) => `\`\`\`futureos-choice\n${body}\n\`\`\`\n`;

const threeOptions = [
  "title: 接下来做哪一件？",
  "mode: single",
  "option: forest | 重画森林图 | 按 Dreher 2025 的样本量重算两篇的置信区间",
  "option: flow | 补一张实验流程图 | 把两篇的被试流程画成一张对照图",
  "option: table | 表里加一列样本量 | Frank 42 / Dreher 186，标出测量手段",
].join("\n");

describe("futureos-choice fence", () => {
  it("parses a single-choice card into a choiceCard node", () => {
    expect(first(fence(threeOptions))).toEqual({
      mode: "single",
      options: [
        { description: "按 Dreher 2025 的样本量重算两篇的置信区间", id: "forest", label: "重画森林图" },
        { description: "把两篇的被试流程画成一张对照图", id: "flow", label: "补一张实验流程图" },
        { description: "Frank 42 / Dreher 186，标出测量手段", id: "table", label: "表里加一列样本量" },
      ],
      title: "接下来做哪一件？",
      type: "choiceCard",
    });
  });

  it("keeps option order and accepts the 2-option and 4-option bounds", () => {
    const two = first(fence("title: T\noption: a | A\noption: b | B"));
    expect(two?.type === "choiceCard" && two.options.map(option => option.id)).toEqual(["a", "b"]);

    const four = first(fence("title: T\noption: a | A\noption: b | B\noption: c | C\noption: d | D"));
    expect(four?.type === "choiceCard" && four.options.map(option => option.id)).toEqual(["a", "b", "c", "d"]);
  });

  it("reads mode: multi and an explicit hint", () => {
    const node = first(fence("title: 这三项都要？\nmode: multi\nhint: 点几个都行\noption: a | A\noption: b | B"));
    expect(node).toMatchObject({ hint: "点几个都行", mode: "multi", type: "choiceCard" });
  });

  it("defaults mode to single and the hint to absent", () => {
    const node = first(fence("title: T\noption: a | A\noption: b | B"));
    expect(node).not.toHaveProperty("hint");
    expect(node).toMatchObject({ mode: "single" });
  });

  /**
   * Every spelling of "several" the model has been seen (or could plausibly be)
   * to write for `multi`. Reading only the exact word turned the rest into a
   * silent `single` — a card that lets one tick where the reply asked for
   * several, and nothing in the UI to show it had been misread.
   */
  it("reads the spellings of multi, and only those", () => {
    for (const spelling of ["multi", "MULTI", "multiple", "multi-select", "multi_select", "多选", " Multi "]) {
      const node = first(fence(`title: T\nmode: ${spelling}\noption: a | A\noption: b | B`));
      expect(node, spelling).toMatchObject({ mode: "multi" });
    }
    // Anything that is not one of those stays the default, including a typo.
    for (const spelling of ["single", "单选", "multii", "both", ""]) {
      const node = first(fence(`title: T\nmode: ${spelling}\noption: a | A\noption: b | B`));
      expect(node, spelling).toMatchObject({ mode: "single" });
    }
  });

  it("ignores unknown fields", () => {
    const node = first(fence("title: T\nfooter: nope\noption: a | A | x\noption: b | B"));
    expect(node?.type).toBe("choiceCard");
    expect(node).not.toHaveProperty("footer");
  });

  it("keeps a `|` inside the description instead of dropping the row", () => {
    const node = first(fence("title: T\noption: a | A | one | two\noption: b | B"));
    expect(node).toMatchObject({ options: [{ description: "one | two", id: "a", label: "A" }, { id: "b", label: "B" }] });
  });

  it("degrades when a row carries an id but no label", () => {
    const node = first(fence("title: T\noption: a | A\noption: b"));
    // `b` has no label, so only one usable row remains: the block degrades.
    expect(node).toMatchObject({ language: "futureos-choice" });
    expect(node?.type).toBe("code");
  });

  describe("degrades to the original code block (§3 tolerance table)", () => {
    const expectCode = (source: string) => {
      const node = first(source);
      expect(node?.type).toBe("code");
      expect(node).toMatchObject({ language: "futureos-choice" });
      return node as { code: string };
    };

    it("fewer than two options", () => {
      const node = expectCode(fence("title: T\noption: a | A"));
      expect(node.code).toContain("option: a | A");
    });

    it("more than four options", () => {
      expectCode(fence("title: T\noption: a | A\noption: b | B\noption: c | C\noption: d | D\noption: e | E"));
    });

    it("no option lines at all", () => {
      expectCode(fence("title: T"));
    });

    it("illegal id characters, leaving one usable row", () => {
      expectCode(fence("title: T\noption: Forest | A\noption: b | B"));
      expectCode(fence("title: T\noption: forest plot | A\noption: b | B"));
    });

    it("an id longer than 24 characters", () => {
      expectCode(fence("title: T\noption: abcdefghijklmnopqrstuvwxy | A\noption: b | B"));
    });

    it("duplicate ids, leaving one usable row", () => {
      expectCode(fence("title: T\noption: a | A\noption: a | A again"));
    });

    it("an empty title with no usable label to fall back to", () => {
      expectCode(fence("title:\noption: a | \noption: b | "));
    });
  });

  it("drops duplicate and illegal rows while two usable rows remain", () => {
    const node = first(
      fence("title: T\noption: a | A\noption: a | A again\noption: BAD | dropped\noption: b | B"),
    );
    expect(node).toMatchObject({ options: [{ id: "a", label: "A" }, { id: "b", label: "B" }] });
  });

  it("falls back to the first option's label when the title is missing", () => {
    const node = first(fence("option: a | 重画森林图\noption: b | 补一张实验流程图"));
    expect(node).toMatchObject({ title: "重画森林图" });
  });

  it("renders a card mid-reply and keeps the surrounding blocks", () => {
    const document = parseFutureMarkdown(`先做这一步。\n\n${fence(threeOptions)}\n再说一句。\n`);
    expect(document.nodes.map(node => node.type)).toEqual(["paragraph", "choiceCard", "paragraph"]);
  });

  it("leaves a non-choice fence alone", () => {
    expect(first("```js\nconst a = 1\n```\n")).toMatchObject({ language: "js", type: "code" });
    expect(first("```futureos-choice-x\ntitle: T\noption: a | A\noption: b | B\n```\n")).toMatchObject({
      language: "futureos-choice-x",
      type: "code",
    });
  });

  it("recognizes the language case-insensitively and ignores trailing spaces", () => {
    const node = first("```FUTUREOS-CHOICE \ntitle: T\noption: a | A\noption: b | B\n```\n");
    expect(node?.type).toBe("choiceCard");
  });

  it("never throws on malformed input", () => {
    for (const body of ["", "| | |", "option:\noption:", "title: T\noption: | A", "::::"]) {
      expect(() => parseFutureMarkdown(fence(body))).not.toThrow();
    }
  });
});
