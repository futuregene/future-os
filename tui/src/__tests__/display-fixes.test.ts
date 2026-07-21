/**
 * Regression tests for display-logic fixes:
 * - AutocompletePopup rows are all the same visible width (aligned right border)
 * - graphemeWidth: text-presentation emoji are 1 cell, emoji-presentation are 2
 * - Footer truncates both sides so the line never exceeds terminal width
 * - Thinking blocks render fully gray (markdown inline styles don't leak body color)
 * - Consecutive thinking blocks concatenate without injected separators
 * - Input.setValue moves the cursor to the end of the new value
 */

import { describe, it, expect } from "bun:test";
import { AutocompletePopup } from "../components/autocomplete.js";
import { ChatArea } from "../components/chat-area.js";
import { Footer } from "../components/footer.js";
import { Input } from "../components/input.js";
import { visibleWidth, stripAnsiCodes, graphemeWidth } from "../utils.js";

describe("AutocompletePopup", () => {
  it("renders all rows at the same visible width", () => {
    const pop = new AutocompletePopup();
    pop.show([
      { value: "/model", label: "/model", description: "select model" },
      { value: "/new", label: "/new", description: "new session" },
      { value: "/long", label: "/a-very-long-command-name-that-exceeds-the-popup-width-abcdefgh", description: "d" },
    ]);
    for (const width of [30, 60, 120]) {
      const lines = pop.render(width);
      const widths = lines.map(visibleWidth);
      expect(new Set(widths).size).toBe(1);
      expect(widths[0]!).toBeLessThanOrEqual(width);
    }
  });

  it("does not sever ANSI sequences when truncating long labels", () => {
    const pop = new AutocompletePopup();
    pop.show([{ value: "x", label: "l".repeat(200), description: "d".repeat(50) }]);
    for (const line of pop.render(40)) {
      // No dangling ESC without a terminator, and every row resets at the end
      expect(stripAnsiCodes(line)).not.toContain("\x1b");
    }
  });
});

describe("graphemeWidth", () => {
  it("measures text-presentation emoji as 1 cell", () => {
    // Emoji=Yes but Emoji_Presentation=No: 1 cell without VS16
    for (const ch of ["▶", "◀", "©", "®", "™", "↔", "⭐", "☀", "⌨", "⏯"]) {
      expect(graphemeWidth(ch)).toBe(1);
    }
  });

  it("measures emoji-presentation code points as 2 cells", () => {
    for (const ch of ["✅", "❌", "✨", "⌚", "⏰", "😀"]) {
      expect(graphemeWidth(ch)).toBe(2);
    }
  });

  it("measures VS16 sequences as 2 cells and VS15 as 1", () => {
    expect(graphemeWidth("☀️")).toBe(2); // + VS16
    expect(graphemeWidth("✅︎")).toBe(1); // + VS15
  });
});

describe("Footer", () => {
  it("never exceeds terminal width, even with a very long cwd", () => {
    const footer = new Footer(80);
    footer.setData({
      cwd: "/very/deep/path/" + "x".repeat(120),
      model: "provider/some-extremely-long-model-name",
      thinking: "high",
      streaming: true,
      spinnerFrame: 3,
      tokensIn: 12000,
      tokensOut: 3400,
      totalCost: 0.123,
      contextTokens: 50000,
      contextWindow: 128000,
      contextPercent: 39,
    });
    for (const width of [30, 60, 80, 120]) {
      const lines = footer.render(width);
      expect(lines.length).toBe(1);
      expect(visibleWidth(lines[0]!)).toBeLessThanOrEqual(width);
    }
  });

  it("keeps the right side (cost/context) visible when the cwd is long", () => {
    const footer = new Footer(80);
    footer.setData({
      cwd: "/very/deep/path/" + "x".repeat(120),
      model: "m",
      totalCost: 0.5,
      contextTokens: 50000,
      contextWindow: 128000,
    });
    const plain = stripAnsiCodes(footer.render(60)[0]!);
    expect(plain).toContain("128k");
  });
});

describe("ChatArea thinking", () => {
  function renderFirstAssistantThinking(chat: ChatArea, width = 60): string[] {
    chat.render(width); // establish width
    return chat.render(width);
  }

  it("renders thinking entirely in the thinking color, even around inline code/bold", () => {
    const chat = new ChatArea(60);
    chat.addMessage({
      id: "1",
      role: "assistant",
      content: "answer",
      thinking: "let me check `some code` and **bold text** first",
    });
    const lines = renderFirstAssistantThinking(chat);
    const thinkingLines = lines.filter((l) => l.includes("let me check") || l.includes("first"));
    expect(thinkingLines.length).toBeGreaterThan(0);
    for (const line of thinkingLines) {
      // Every ANSI reset in a thinking line must be immediately followed by
      // the thinking style prefix (italic + gray 244), so no segment falls
      // back to the default body color.
      const resets = line.match(/\x1b\[0?m/g) ?? [];
      for (const r of resets) {
        const idx = line.indexOf(r);
        const after = line.slice(idx + r.length);
        const isLineEnd = stripAnsiCodes(after).trim() === "";
        if (!isLineEnd) {
          expect(after.startsWith("\x1b[3m\x1b[38;5;244m")).toBe(true);
        }
      }
      expect(stripAnsiCodes(line)).toContain("let me check `some code` and **bold text** first".replace(/[`*]/g, ""));
    }
  });

  it("never leaks markdown accent colors into thinking (code, link, heading, list, fence)", () => {
    const chat = new ChatArea(60);
    chat.addMessage({
      id: "1",
      role: "assistant",
      content: "answer",
      thinking: [
        "# Plan",
        "check `some code` and **bold** plus [a link](https://example.com) here",
        "- first item",
        "```js",
        "const x = 1;",
        "```",
      ].join("\n"),
    });
    const lines = renderFirstAssistantThinking(chat);
    const needles = ["Plan", "some code", "a link", "first item", "const x = 1;"];
    const thinkingLines = lines.filter((l) => needles.some((n) => stripAnsiCodes(l).includes(n)));
    for (const n of needles) {
      expect(thinkingLines.some((l) => stripAnsiCodes(l).includes(n))).toBe(true);
    }
    for (const line of thinkingLines) {
      // Every SGR foreground color in a thinking line must be the thinking
      // gray (244) — no markdown accent colors (151 code, 117 link, 221
      // heading, 143 code block) may survive.
      const colors = [...line.matchAll(/38;5;(\d+)/g)].map((m) => Number(m[1]));
      expect(colors.length).toBeGreaterThan(0);
      for (const c of colors) {
        expect(c).toBe(244);
      }
    }
  });

  it("concatenates consecutive thinking blocks directly (no injected separator)", () => {
    const chat = new ChatArea(60);
    chat.addMessage({ id: "1", role: "assistant", content: "" });
    chat.startThinking();
    chat.appendThinkingDelta("first block");
    chat.endThinking();
    chat.startThinking();
    chat.appendThinkingDelta("second block");
    chat.endThinking();
    const lines = chat.render(60);
    const plain = lines.map(stripAnsiCodes).join("\n");
    expect(plain).toContain("first blocksecond block");
    expect(plain).not.toContain("first block\n\nsecond block");
  });

  it("shows an interrupted marker for stopped messages", () => {
    const chat = new ChatArea(60);
    chat.addMessage({ id: "1", role: "assistant", content: "partial answer" });
    chat.markLastAssistantStopped();
    const plain = chat.render(60).map(stripAnsiCodes).join("\n");
    expect(plain).toContain("interrupted");
  });

  it("preserves blank lines in user messages", () => {
    const chat = new ChatArea(60);
    chat.addMessage({ id: "1", role: "user", content: "para one\n\npara two" });
    const lines = chat.render(60);
    const texts = lines.map(stripAnsiCodes);
    const first = texts.findIndex((t) => t.includes("para one"));
    const second = texts.findIndex((t) => t.includes("para two"));
    expect(first).toBeGreaterThanOrEqual(0);
    expect(second).toBeGreaterThan(first + 1); // blank line between paragraphs
  });
});

describe("Input.setValue", () => {
  it("moves the cursor to the end of the new value by default", () => {
    const input = new Input();
    input.insertText("/mo");
    input.setValue("/model");
    // Typing after setValue must append at the end, not mid-word
    input.insertText(" x");
    expect(input.getValue()).toBe("/model x");
  });

  it("honours an explicit cursor position", () => {
    const input = new Input();
    input.setValue("hello world", 5);
    input.insertText(",");
    expect(input.getValue()).toBe("hello, world");
  });
});
