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
    // Brackets without a target, an array index — neither is a link, and a
    // title must not eat them.
    expect(plainTitleText("[pending] 待办")).toBe("[pending] 待办");
    expect(plainTitleText("items[0] = 3")).toBe("items[0] = 3");
    // An unclosed link is not a link either.
    expect(plainTitleText("[标题](futureos://session/s-3")).toBe("[标题](futureos://session/s-3");
  });

  it("drops a composer placeholder, which names nothing", () => {
    expect(plainTitleText("[Image #1]现在 tui 中显示展示思考中")).toBe("现在 tui 中显示展示思考中");
    expect(plainTitleText("[Image #1][Image #2]")).toBe("");
    expect(plainTitleText("看 [Image #3] 这张图里的报错")).toBe("看 这张图里的报错");
    expect(plainTitleText("[Pasted Content 2000 chars] 帮我看看")).toBe("帮我看看");
    expect(plainTitleText("[Pasted Content 2000 chars #2] 帮我看看")).toBe("帮我看看");
    // Only the exact forms the composers write are placeholders.
    expect(plainTitleText("[Image #] 说明")).toBe("[Image #] 说明");
    expect(plainTitleText("[image #1] 说明")).toBe("[image #1] 说明");
  });

  it("flattens markdown syntax and keeps its text", () => {
    expect(plainTitleText("**加粗** 一下")).toBe("加粗 一下");
    expect(plainTitleText("__加粗__ 一下")).toBe("加粗 一下");
    expect(plainTitleText("*斜体* 与 ~~删除线~~")).toBe("斜体 与 删除线");
    expect(plainTitleText("看看 `cargo test` 的输出")).toBe("看看 cargo test 的输出");
    expect(plainTitleText("# 标题\n正文")).toBe("标题 正文");
    expect(plainTitleText("### 修复登录")).toBe("修复登录");
    expect(plainTitleText("> 引用的一句话")).toBe("引用的一句话");
    expect(plainTitleText("- 第一条\n- 第二条")).toBe("第一条 第二条");
    expect(plainTitleText("1. 第一步\n2) 第二步")).toBe("第一步 第二步");
    expect(plainTitleText("---\n正文")).toBe("正文");
    expect(plainTitleText("```rust\nfn main() {}\n```")).toBe("fn main() {}");
  });

  it("keeps unpaired or intraword delimiters as text", () => {
    expect(plainTitleText("a_b_c 命名")).toBe("a_b_c 命名");
    expect(plainTitleText("2 * 3 = 6")).toBe("2 * 3 = 6");
    expect(plainTitleText("未闭合的 **加粗")).toBe("未闭合的 **加粗");
    expect(plainTitleText("C# 与 ~/future-os")).toBe("C# 与 ~/future-os");
  });

  it("collapses whitespace to one line", () => {
    expect(plainTitleText("  第一行\n\n第二行  ")).toBe("第一行 第二行");
    expect(plainTitleText("   ")).toBe("");
    expect(plainTitleText("")).toBe("");
  });
});
