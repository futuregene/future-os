/**
 * Unit tests for terminal width/wrapping utilities.
 *
 * These guard the invariants the renderer relies on: CJK/emoji are 2 cells,
 * combining marks 0, ANSI sequences invisible, and no multi-byte grapheme is
 * ever split. Run with: bun test
 */
import { describe, test, expect } from "bun:test";
import {
  visibleWidth,
  graphemeWidth,
  stripAnsiCodes,
  replaceTabs,
  truncateToWidth,
  sliceWithWidth,
  sliceByColumn,
  wrapTextWithAnsi,
  applyBackgroundToLine,
} from "../utils.js";

// ─── graphemeWidth ─────────────────────────────────────────────────────────

describe("graphemeWidth", () => {
  test("ASCII is 1 cell", () => {
    expect(graphemeWidth("a")).toBe(1);
    expect(graphemeWidth(" ")).toBe(1);
  });

  test("CJK is 2 cells", () => {
    expect(graphemeWidth("中")).toBe(2);
    expect(graphemeWidth("あ")).toBe(2);
    expect(graphemeWidth("한")).toBe(2);
  });

  test("emoji is 2 cells", () => {
    expect(graphemeWidth("🦀")).toBe(2);
    expect(graphemeWidth("🎉")).toBe(2);
    expect(graphemeWidth("🚀")).toBe(2);
    expect(graphemeWidth("🔧")).toBe(2);
  });

  test("VS15 forces text presentation (narrow)", () => {
    // U+2705 + U+FE0E renders as a 1-cell text glyph, not an emoji.
    expect(graphemeWidth("✅\uFE0E")).toBe(1);
    // VS16 (emoji presentation) stays wide.
    expect(graphemeWidth("✅\uFE0F")).toBe(2);
  });

  test("combining marks are 0 cells", () => {
    expect(graphemeWidth("́")).toBe(0); // U+0301 combining acute
  });

  test("empty string is 0", () => {
    expect(graphemeWidth("")).toBe(0);
  });
});

// ─── visibleWidth ──────────────────────────────────────────────────────────

describe("visibleWidth", () => {
  test("plain ASCII", () => {
    expect(visibleWidth("hello")).toBe(5);
  });

  test("CJK counts double", () => {
    expect(visibleWidth("你好")).toBe(4);
    expect(visibleWidth("a中b")).toBe(4);
  });

  test("emoji counts double", () => {
    expect(visibleWidth("🦀🦀")).toBe(4);
  });

  test("ANSI escape codes are invisible", () => {
    expect(visibleWidth("\x1b[31mred\x1b[0m")).toBe(3);
    expect(visibleWidth("\x1b[1;42mbold green\x1b[m")).toBe(10);
  });

  test("OSC 8 hyperlinks are invisible", () => {
    const link = "\x1b]8;;https://example.com\x07click\x1b]8;;\x07";
    expect(visibleWidth(link)).toBe(5);
  });
});

// ─── stripAnsiCodes ────────────────────────────────────────────────────────

describe("stripAnsiCodes", () => {
  test("removes CSI sequences", () => {
    expect(stripAnsiCodes("\x1b[31mhi\x1b[0m")).toBe("hi");
    expect(stripAnsiCodes("\x1b[1;2;3mx")).toBe("x");
  });

  test("removes OSC sequences", () => {
    expect(stripAnsiCodes("\x1b]8;;https://x.dev\x07text\x1b]8;;\x07")).toBe("text");
  });

  test("passes plain text through", () => {
    expect(stripAnsiCodes("plain 中文 🦀")).toBe("plain 中文 🦀");
  });
});

// ─── replaceTabs ───────────────────────────────────────────────────────────

describe("replaceTabs", () => {
  test("default tab width is 3", () => {
    expect(replaceTabs("a\tb")).toBe("a   b");
  });

  test("custom tab width", () => {
    expect(replaceTabs("a\tb", 4)).toBe("a    b");
  });
});

// ─── truncateToWidth ───────────────────────────────────────────────────────

describe("truncateToWidth", () => {
  test("no-op when shorter than width", () => {
    expect(truncateToWidth("abc", 5)).toBe("abc");
  });

  test("truncates ASCII to width", () => {
    expect(truncateToWidth("abcdef", 3)).toBe("abc");
  });

  test("never splits a CJK character", () => {
    // 3 cells requested, but the second CJK char needs cells 3-4 → dropped.
    expect(truncateToWidth("中文中文", 3)).toBe("中");
    expect(truncateToWidth("中文", 4)).toBe("中文");
  });

  test("ellipsis replaces last char when truncated", () => {
    expect(truncateToWidth("abcdef", 3, { ellipsis: true })).toBe("ab…");
    // Not truncated → no ellipsis.
    expect(truncateToWidth("abc", 3, { ellipsis: true })).toBe("abc");
  });

  test("pad fills to width", () => {
    expect(truncateToWidth("ab", 4, { pad: true })).toBe("ab  ");
  });

  test("width <= 0 yields empty", () => {
    expect(truncateToWidth("abc", 0)).toBe("");
  });
});

// ─── sliceWithWidth / sliceByColumn ────────────────────────────────────────

describe("sliceWithWidth", () => {
  test("reports consumed width", () => {
    expect(sliceWithWidth("hello", 3)).toEqual({ text: "hel", width: 3 });
  });

  test("CJK consumes 2 cells", () => {
    expect(sliceWithWidth("中文x", 2)).toEqual({ text: "中", width: 2 });
  });

  test("ANSI codes travel with the slice but cost no width", () => {
    const { text, width } = sliceWithWidth("\x1b[31mabc\x1b[0m", 2);
    expect(stripAnsiCodes(text)).toBe("ab");
    expect(width).toBe(2);
  });
});

describe("sliceByColumn", () => {
  test("slices [start, end) in cells", () => {
    expect(sliceByColumn("hello world", 0, 5)).toBe("hello");
    expect(sliceByColumn("hello world", 6)).toBe("world");
  });

  test("CJK columns", () => {
    expect(sliceByColumn("a中b", 1, 3)).toBe("中");
  });

  test("negative start clamps to 0", () => {
    expect(sliceByColumn("abc", -5, 2)).toBe("ab");
  });
});

// ─── wrapTextWithAnsi ──────────────────────────────────────────────────────

describe("wrapTextWithAnsi", () => {
  test("wraps at width with hard break", () => {
    const lines = wrapTextWithAnsi("abcdefgh", 4);
    expect(lines.map(stripAnsiCodes)).toEqual(["abcd", "efgh"]);
  });

  test("prefers word boundary over hard break", () => {
    const lines = wrapTextWithAnsi("hello world", 6);
    expect(lines.map(stripAnsiCodes)).toEqual(["hello", "world"]);
  });

  test("splits on newlines first", () => {
    const lines = wrapTextWithAnsi("ab\ncd", 10);
    expect(lines.map(stripAnsiCodes)).toEqual(["ab", "cd"]);
  });

  test("CJK wrapping respects double width", () => {
    const lines = wrapTextWithAnsi("中文中文", 4);
    expect(lines.map(stripAnsiCodes)).toEqual(["中文", "中文"]);
  });

  test("carries active style onto the next line", () => {
    // Use 256-color escape which the tracker models natively.
    const lines = wrapTextWithAnsi("\x1b[38;5;1mabcdefgh\x1b[0m", 4);
    expect(lines).toHaveLength(2);
    expect(lines[1]).toContain("\x1b[38;5;1m");
    expect(lines[1]).toContain("efgh");
  });

  test("standard and 256-color styles both survive wrapping", () => {
    // SGR 31 (standard red) and 38;5;45 (256-color) must both be re-opened
    // on continuation lines — the tracker previously dropped 30-37/40-47.
    const std = wrapTextWithAnsi("\x1b[31mabcdefgh", 4);
    expect(std[1]).toContain("\x1b[31m");

    const ext = wrapTextWithAnsi("\x1b[38;5;45mabcdefgh", 4);
    expect(ext[1]).toContain("38;5;45");

    const bright = wrapTextWithAnsi("\x1b[91mabcdefgh", 4);
    expect(bright[1]).toContain("\x1b[91m");
  });

  test("width <= 0 returns no lines", () => {
    expect(wrapTextWithAnsi("abc", 0)).toEqual([]);
  });
});

// ─── applyBackgroundToLine ─────────────────────────────────────────────────

describe("applyBackgroundToLine", () => {
  test("pads to width with bg color", () => {
    const out = applyBackgroundToLine("ab", 5, 42);
    expect(out).toContain("\x1b[48;5;42m");
    expect(visibleWidth(out)).toBe(5);
  });

  test("bg < 0 pads with terminal default (no bg code)", () => {
    const out = applyBackgroundToLine("ab", 5, -1);
    expect(out).not.toContain("48;5");
    expect(out.endsWith("   ")).toBe(true);
  });

  test("mid-line resets are re-armed with the bg color", () => {
    const out = applyBackgroundToLine("\x1b[31mab\x1b[0m", 5, 42);
    // The plain \x1b[0m from the content must be upgraded to \x1b[0m + bg.
    expect(out).toContain("\x1b[0m\x1b[48;5;42m");
  });
});
