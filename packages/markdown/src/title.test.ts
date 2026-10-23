import { describe, expect, it } from "vitest";
import { plainTitleText } from "./title";

describe("plainTitleText", () => {
  it("drops a reference and keeps the question", () => {
    expect(
      plainTitleText(
        "[手机截图镜像修复](futureos://session/20261016-142101-2a944dead697482d8a6adad9942fe403) 这个会话在流式回复中",
      ),
    ).toBe("这个会话在流式回复中");
  });

  it("names a lone reference after the conversation it points at", () => {
    expect(plainTitleText("[手机截图镜像修复](futureos://session/s-1)")).toBe("手机截图镜像修复");
  });

  it("leaves no gap where a reference sat inside a sentence", () => {
    expect(plainTitleText("先看 [登录修复](futureos://session/s-9) 再说")).toBe("先看 再说");
  });

  it("keeps other links' labels and images' alt text", () => {
    expect(plainTitleText("修一下 [这一行](https://example.com/x)")).toBe("修一下 这一行");
    expect(plainTitleText("![截图](./a.png) 崩了")).toBe("截图 崩了");
  });

  it("accepts an angle-wrapped target or one with parentheses", () => {
    expect(plainTitleText("[看这个](<futureos://session/s-2>)")).toBe("看这个");
    expect(plainTitleText("[手册](docs(1).md) 怎么看")).toBe("手册 怎么看");
  });

  it("leaves text that is not a link untouched", () => {
    // Brackets without a target, an array index, an emphasis pair — none of
    // these are links, and a title must not eat them.
    expect(plainTitleText("[pending] 待办")).toBe("[pending] 待办");
    expect(plainTitleText("items[0] = 3")).toBe("items[0] = 3");
    expect(plainTitleText("**加粗** 一下")).toBe("**加粗** 一下");
    expect(plainTitleText("a_b_c 命名")).toBe("a_b_c 命名");
    // An unclosed link is not a link either.
    expect(plainTitleText("[标题](futureos://session/s-3")).toBe("[标题](futureos://session/s-3");
  });

  it("collapses whitespace to one line", () => {
    expect(plainTitleText("  第一行\n\n第二行  ")).toBe("第一行 第二行");
    expect(plainTitleText("   ")).toBe("");
    expect(plainTitleText("")).toBe("");
  });
});
