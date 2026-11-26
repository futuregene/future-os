import { describe, expect, it } from "vitest";
import { parseMdast, parseFutureMarkdown } from "./parseFutureMarkdown";

/**
 * The reply that failed on 2026-11-24 (`future/deepseek-flash`, desktop thread
 * `a8d60739`): a closed `<futureos-choice>` pair where the fence belongs, then a
 * bare fence line it meant as a closer. Verbatim — the point of the fix is that
 * *this* text draws a card.
 */
const TAGGED_REPLY = "我这条也犯了同样的错，而且错误更明显：这已经是我第二次犯，同模型的第二次，不能再算「偶发」。\n\n## 查证\n\n我自己这条回复的原文就存在会话记录里（本会话 `entry 100`），末尾是：\n\n```\n<futureos-choice>\ntitle: 这个失误要处理吗？\n…\n</futureos-choice>\n```\n\n它跟我上一条分析的、那个会话里的原文**一模一样**：都是标签写法，一个反引号都没有。所以你刚才看到的依然是一段挤在一起、带标签的纯文本。\n\n## 这条错误改变了我的诊断\n\n同一段回复里，我在表格中写 ```` ```futureos-choice ```` 时写得完全正确，而卡片本身却写成了标签。这说明问题不在于「不知道正确写法」，而在于输出时实际用了另一种表示。提示词里那段带示例的说明，对我这个模型（`future/deepseek-flash`）显然没有约束力，两次都如此。\n\n由此可以判断：\n\n- 「提示词里加一句禁止标签」只能降低出错的概率，起不到决定性作用，而两个同一模型上的样本都已经证明，它读得懂围栏写法，只是不会照着写。\n- 真正有效的做法，是让解析器接受这种写法：`parseFutureMarkdown` 里，`<futureos-choice>…</futureos-choice>` 目前会被处理成一个 `paragraph`；只要识别成卡片，再把这套接受标签的逻辑同步到 `agent/src/prompt/choice_card.rs::reply_offers_choice` 的徽章判定，就能同时解决卡片和待回答标记两个问题。\n- 「先不动」这个选项已经不成立，因为它不是偶发问题。\n\n另外，这里还有一个容错缺口值得单独记一笔：设计文档 §3 承诺「畸形块降级为可读的代码块」，但标签写法现在降级成了一段普通文本，比代码块还差。\n\n<futureos-choice>\ntitle: 这两处要现在改吗？\ncontext: 同模型两次都把围栏写成 XML 标签，提示词写法本身没问题，所以真正的修复点在解析层。\nmode: multi\noption: parser | 解析器和徽章都接受标签形式 | parseFutureMarkdown 加一条识别，reply_offers_choice 同步\noption: prompt | 提示词里补一句「必须围栏」 | 成本一行字，只当止血，不当修复\n</futureos-choice>\n```";

/** The cards a reply draws, parsed outside the shared cache. */
function cards(raw: string) {
  return parseFutureMarkdown(raw, undefined, false).nodes.filter(node => node.type === "choiceCard");
}

describe("a `<futureos-choice>` tag pair", () => {
  it("draws no card as written", () => {
    // Before this change the parser fed these children straight to the card
    // parser, and that parser only reads a `code` node carrying the fence's
    // language: the tag pair arrives as an `html` node holding the raw lines, so
    // there was nothing to read and the reply printed its source.
    const blocks = parseMdast(TAGGED_REPLY).children;
    expect(blocks.some(block => block.type === "code" && block.lang === "futureos-choice")).toBe(false);
    expect(
      blocks.some(block => block.type === "html" && block.value.startsWith("<futureos-choice>")),
    ).toBe(true);
  });

  it("draws the card it stands for, as the reply's last block", () => {
    const document = parseFutureMarkdown(TAGGED_REPLY, undefined, false);
    const last = document.nodes[document.nodes.length - 1];
    expect(last?.type).toBe("choiceCard");
    if (last?.type !== "choiceCard") throw new Error("expected a choice card");
    expect(last.title).toBe("这两处要现在改吗？");
    expect(last.mode).toBe("multi");
    expect(last.options.map(option => option.id)).toEqual(["parser", "prompt"]);
    expect(last.options[0]?.label).toBe("解析器和徽章都接受标签形式");
    // The bare fence after the pair is the same emission's attempted closer: it
    // holds nothing and is dropped, so the card is not read-only behind an empty
    // code block. (The reply also *quotes* the tag form earlier, inside its own
    // fence — that stays a code block, which is why only empty blocks are
    // checked here.)
    expect(document.nodes.filter(node => node.type === "code" && node.code.trim() === "")).toEqual([]);
    expect(cards(TAGGED_REPLY)).toHaveLength(1);
  });

  it("keeps the fence form working", () => {
    const fenced = "```futureos-choice\ntitle: T\noption: a | one\noption: b | two\n```";
    expect(cards(fenced)).toHaveLength(1);
  });

  it("does not accept a closing tag without its opening one", () => {
    expect(cards("title: T\noption: a | one\noption: b | two\n</futureos-choice>")).toEqual([]);
  });

  it("does not accept the word alone", () => {
    expect(cards("A `futureos-choice` block is a fence.")).toEqual([]);
    expect(cards("Use the futureos-choice form.")).toEqual([]);
  });

  it("does not read a quoted pair inside a fence as a card", () => {
    const quoted = "```markdown\n<futureos-choice>\ntitle: T\noption: a | one\noption: b | two\n</futureos-choice>\n```";
    const nodes = parseFutureMarkdown(quoted, undefined, false).nodes;
    expect(nodes.map(node => node.type)).toEqual(["code"]);
  });

  it("splits a pair off the prose it shares a paragraph with", () => {
    // The 2026-11-25 reply wrote `…再问一遍：` and the tag on the next line, no
    // blank line between. A fence interrupts a paragraph there, so this is the
    // same attempt: the sentence stays a sentence and the pair becomes a card.
    const attached = "还有一点：\n<futureos-choice>\ntitle: T\noption: a | one\noption: b | two\n</futureos-choice>";
    const nodes = parseFutureMarkdown(attached, undefined, false).nodes;
    expect(nodes.map(node => node.type)).toEqual(["paragraph", "choiceCard"]);
    expect(JSON.stringify(nodes[0])).toContain("还有一点：");
    expect(cards(attached)).toHaveLength(1);
  });

  it("keeps the prose's own marks when it splits the paragraph", () => {
    // The split keeps the paragraph's inline children instead of rebuilding it
    // from the raw lines, so a marked-up sentence does not lose its emphasis on
    // the way to the card behind it.
    const attached = "这里有 **加粗** 的一点：\n<futureos-choice>\ntitle: T\noption: a | one\noption: b | two\n</futureos-choice>";
    const document = parseFutureMarkdown(attached, undefined, false);
    expect(JSON.stringify(document.nodes[0])).toContain("strong");
    expect(document.nodes[1]?.type).toBe("choiceCard");
  });

  it("reads an opening tag that ends the reply as a card", () => {
    // No closer, nothing after it: there is nothing left to swallow, and a
    // writer that forgot the closing tag meant a card.
    const unclosed = "还有一点：\n\n<futureos-choice>\ntitle: T\noption: a | one\noption: b | two";
    expect(cards(unclosed)).toHaveLength(1);
  });

  it("leaves an unclosed tag that has prose after it, or no rows, as text", () => {
    // Mid-reply a missing closer would swallow what follows, so it stays text;
    // and a body that does not look like option rows is not a card attempt.
    const midReply = "<futureos-choice>\ntitle: T\noption: a | one\noption: b | two\n\n还有别的吗？";
    expect(cards(midReply)).toEqual([]);
    const proseOnly = "<futureos-choice>\ntitle: T\n这里只是一段话";
    expect(cards(proseOnly)).toEqual([]);
  });

  it("folds the ways a writer spells the language", () => {
    // Case, the separator and surrounding space are spelling, not meaning. The
    // agent's badge folds them the same way (`choice_card.rs`).
    for (const lang of ["FUTUREOS-CHOICE", "futureos_choice", " futureos-choice "]) {
      expect(cards(`\`\`\`${lang}\ntitle: T\noption: a | one\noption: b | two\n\`\`\``)).toHaveLength(1);
    }
    // Anything past those still has to match: this is another language.
    expect(cards("```futureos-choice-x\ntitle: T\noption: a | one\noption: b | two\n```")).toEqual([]);
  });

  it("drops only an empty closer, never content after the pair", () => {
    const withContent = "<futureos-choice>\ntitle: T\noption: a | one\noption: b | two\n</futureos-choice>\n```\n\n还有别的吗？";
    const nodes = parseFutureMarkdown(withContent, undefined, false).nodes;
    expect(nodes[0]?.type).toBe("choiceCard");
    expect(JSON.stringify(nodes.slice(1))).toContain("还有别的吗？");
  });

  it("reads the pair through the same field rules as the fence", () => {
    // A row list the fence rules reject is rejected here too: one writer, one
    // parser, no second set of bounds.
    const oneRow = "<futureos-choice>\ntitle: T\noption: a | one\n</futureos-choice>";
    expect(cards(oneRow)).toEqual([]);
    const sevenRows = "<futureos-choice>\ntitle: T\n" +
      Array.from({ length: 7 }, (_, index) => `option: id${index} | label ${index}`).join("\n") +
      "\n</futureos-choice>";
    expect(cards(sevenRows)).toEqual([]);
  });
});
