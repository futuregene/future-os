/**
 * Targeted bug tests for multi-line Input component.
 */

import { describe, expect, test } from "bun:test";
import { Input } from "../components/input.js";

describe("Input multi-line: cursor navigation", () => {
  test("right arrow at end of line moves to next line start", () => {
    const input = new Input();
    input.setValue("abc\ndef", 3); // cursor at end of "abc" (before \n)
    input.handleKey("right");
    // After skip: should be at start of "def" (position 4)
    expect(input.getValue()).toBe("abc\ndef");
  });

  test("right arrow skip: symmetric skip over newlines", () => {
    const input = new Input();
    input.setValue("a\nb", 0);
    // Right from 'a': skips \n, lands on 'b'
    input.handleKey("right");
    expect((input as any).cursor).toBe(2); // on 'b', skipped \n
    // Left from 'b': skips \n, lands back on 'a'
    input.handleKey("left");
    expect((input as any).cursor).toBe(0); // back on 'a'
  });

  test("up/down on multi-line moves cursor between lines", () => {
    const input = new Input();
    input.setValue("hello\nworld", 1); // cursor on 'e' in "hello"
    input.handleKey("down");
    // Should move to position 1 in "world" (i.e., 'o')
    // If the cursor is still on "hello", the move failed
  });

  test("up at first line falls back to history", () => {
    const input = new Input();
    // Simulate submitting something first so history has an entry
    (input as any).onSubmit = () => {};
    input.setValue("line1\nline2", 0); // cursor at start of first line
    input.handleKey("enter"); // submits and adds to history

    // Now set a multi-line value, cursor at first line
    input.setValue("aaa\nbbb", 0);
    // Up should go to history, not stay on current line
    input.handleKey("up");
    // If history was navigated, value changed
  });
});

describe("Input multi-line: text manipulation", () => {
  test("Ctrl+U at line start joins with previous line", () => {
    const input = new Input();
    input.setValue("abc\ndef", 4); // cursor at start of "def" (after \n)
    input.handleKey("ctrl+u");
    expect(input.getValue()).toBe("abcdef");
  });

  test("Ctrl+U at very start does nothing", () => {
    const input = new Input();
    input.setValue("abc\ndef", 0); // cursor at very start
    input.handleKey("ctrl+u");
    expect(input.getValue()).toBe("abc\ndef");
  });

  test("backspace at line start joins lines", () => {
    const input = new Input();
    input.setValue("abc\ndef", 4); // cursor at start of "def"
    input.handleKey("backspace");
    expect(input.getValue()).toBe("abcdef");
  });
});

describe("Input multi-line: render", () => {
  test("render produces one line per \n in value", () => {
    const input = new Input();
    input.setValue("line1\nline2\nline3");
    const lines = input.render(80);
    expect(lines.length).toBe(3);
    expect(lines[0]).toMatch(/^> /);
    expect(lines[1]).toMatch(/^  /);
    expect(lines[2]).toMatch(/^  /);
  });

  test("empty value renders one line", () => {
    const input = new Input();
    const lines = input.render(80);
    expect(lines.length).toBe(1);
  });

  test("empty lines render correctly", () => {
    const input = new Input();
    input.setValue("a\n\nb");
    const lines = input.render(80);
    expect(lines.length).toBe(3);
    // Middle line should be just the prompt "  " with padding
    expect(lines[1]).toMatch(/^  /);
  });
});

describe("Input multi-line: paste", () => {
  test("paste preserves newlines", () => {
    const input = new Input();
    input.insertText("line1\r\nline2\rline3\nline4");
    expect(input.getValue()).toBe("line1\nline2\nline3\nline4");
  });

  test("paste replaces tabs with 4 spaces", () => {
    const input = new Input();
    input.insertText("a\tb");
    expect(input.getValue()).toBe("a    b");
  });
});

describe("Input multi-line: Home/End", () => {
  test("Home goes to line start, then value start", () => {
    const input = new Input();
    input.setValue("abc\ndef", 5); // cursor at 'e' in "def"
    input.handleKey("home");
    // Should go to start of "def" line (position 4)
    expect((input as any).cursor).toBe(4);

    input.handleKey("home");
    // Should go to start of entire value (position 0)
    expect((input as any).cursor).toBe(0);
  });

  test("End goes to line end, then value end", () => {
    const input = new Input();
    input.setValue("abc\ndef", 5); // cursor at 'e' in "def"
    input.handleKey("end");
    // Should go to end of "def" line (position 7)
    expect((input as any).cursor).toBe(7);

    input.handleKey("end");
    // Should go to end of entire value (position 7)
    expect((input as any).cursor).toBe(7);
  });
});

describe("Input: getLineBounds edge cases", () => {
  test("cursor at value start", () => {
    const input = new Input();
    input.setValue("a\nb", 0);
    // Should be on first line
    const bounds = (input as any).getLineBounds(0);
    expect(bounds).toEqual({ start: 0, end: 1 });
  });

  test("cursor at value end", () => {
    const input = new Input();
    input.setValue("a\nb", 3);
    // Should be on second line
    const bounds = (input as any).getLineBounds(3);
    expect(bounds).toEqual({ start: 2, end: 3 });
  });

  test("cursor on the newline character itself", () => {
    const input = new Input();
    input.setValue("a\nb", 1); // cursor ON the \n
    const bounds = (input as any).getLineBounds(1);
    // Should be on first line (line containing "a")
    expect(bounds.start).toBe(0);
    expect(bounds.end).toBe(1);
  });

  test("single line (no newlines)", () => {
    const input = new Input();
    input.setValue("hello", 2);
    const bounds = (input as any).getLineBounds(2);
    expect(bounds).toEqual({ start: 0, end: 5 });
  });
});

describe("Input: left/right arrow symmetry", () => {
  test("right arrow skips newlines to land on visible text", () => {
    const input = new Input();
    input.setValue("a\nb", 0);
    input.handleKey("right");
    expect((input as any).cursor).toBe(2); // skipped \n, landed on 'b'
  });

  test("right arrow does not skip past end", () => {
    const input = new Input();
    input.setValue("a\nb", 2); // on 'b'
    input.handleKey("right");
    expect((input as any).cursor).toBe(3); // after 'b', at end
  });

  test("left arrow skips newlines to land on visible text", () => {
    const input = new Input();
    input.setValue("a\nb", 2); // on 'b'
    input.handleKey("left");
    expect((input as any).cursor).toBe(0); // skipped \n, landed on 'a'
  });

  test("right then left returns to origin (skipping newlines)", () => {
    const input = new Input();
    input.setValue("a\n\nb", 0);
    // Right: skips both \n, lands on 'b' (position 3)
    input.handleKey("right");
    expect((input as any).cursor).toBe(3); // on 'b'
    input.handleKey("right");
    expect((input as any).cursor).toBe(4); // past 'b', at end
    // Left: skips both \n, back to 'a'
    input.handleKey("left");
    expect((input as any).cursor).toBe(3); // back on 'b'
    input.handleKey("left");
    expect((input as any).cursor).toBe(0); // back to 'a', skipped both \n
  });
});

describe("Input soft-wrap (auto line wrapping)", () => {
  test("long single line wraps to multiple visual lines", () => {
    const input = new Input();
    const longText = "a".repeat(50);
    input.setValue(longText);
    // At width 20 (availableWidth = 18 after prompt), should produce multiple lines
    // First, trigger layout build via getCursorVisualInfo (render uses it)
    const lines = input.render(20);
    expect(lines.length).toBeGreaterThan(1);
    // First line has "> " prefix
    expect(lines[0]).toMatch(/^> /);
    // Subsequent lines have "  " prefix
    expect(lines[1]).toMatch(/^  /);
  });

  test("soft-wrap: up/down moves between wrapped visual lines", () => {
    const input = new Input();
    // 10 chars, width 6 → availableWidth=4, wraps to multiple lines
    input.setValue("abcdefghij", 8); // cursor near end
    // Build layout first
    input.render(6);
    const infoBefore = (input as any).getCursorVisualInfo();
    expect(infoBefore.visualLine).toBeGreaterThan(0); // cursor not on first visual line

    // Move up
    input.handleKey("up");
    const infoAfter = (input as any).getCursorVisualInfo();
    expect(infoAfter.visualLine).toBe(infoBefore.visualLine - 1);

    // Move back down
    input.handleKey("down");
    const infoDown = (input as any).getCursorVisualInfo();
    expect(infoDown.visualLine).toBe(infoBefore.visualLine);
  });

  test("soft-wrap: up at first visual line falls back to history", () => {
    const input = new Input();
    // Submit something first to populate history
    (input as any).onSubmit = () => {};
    input.setValue("history-entry");
    input.handleKey("enter");

    // Now set a long value, cursor at start → first visual line
    input.setValue("abcdefghij", 0); // cursor at position 0
    input.render(6); // build layout with narrow width
    const info = (input as any).getCursorVisualInfo();
    expect(info.visualLine).toBe(0); // on first visual line

    // Up should go to history, not stay on current line
    input.handleKey("up");
    expect(input.getValue()).toBe("history-entry");
  });

  test("soft-wrap: down from last visual line stays in content, history only from bounds", () => {
    const input = new Input();
    (input as any).onSubmit = () => {};
    input.setValue("history-line");
    input.handleKey("enter");

    // Long value, cursor at start → first visual line
    input.setValue("abcdefghij", 0);
    input.render(6);

    // Up at first visual line → goes to history
    input.handleKey("up");
    expect(input.getValue()).toBe("history-line");

    // Now the visual layout has changed (single-line history entry)
    // Press down at last visual line → return to draft
    input.handleKey("down");
    expect(input.getValue()).toBe("abcdefghij");
  });

  test("soft-wrap: cursor position preserved across visual line navigation", () => {
    const input = new Input();
    // 5 chars per visual line at width 7 (prompt=2, available=5)
    // "0123456789" wraps as: "01234", "56789"
    input.setValue("0123456789", 7); // cursor on '7' (second visual line, col 2)
    input.render(7);

    // Move up → should land on first visual line, roughly same column
    input.handleKey("up");
    // Cursor should be on '2' (col 2 from start of first visual line)
    const cursorAfterUp = (input as any).cursor;
    expect(input.getValue()[cursorAfterUp]).toBe("2");

    // Move down → should return to '7'
    input.handleKey("down");
    const cursorAfterDown = (input as any).cursor;
    expect(input.getValue()[cursorAfterDown]).toBe("7");
  });

  test("soft-wrap: single short line renders as one line", () => {
    const input = new Input();
    input.setValue("hi");
    const lines = input.render(80);
    expect(lines.length).toBe(1);
    expect(lines[0]).toMatch(/^> /);
  });

  test("soft-wrap: hard newline + soft wrap combined", () => {
    const input = new Input();
    // Two logical lines: first is short, second is long (wraps)
    input.setValue("short\n" + "x".repeat(30));
    const lines = input.render(20);
    // Should have: "short" (1 line) + wrapped "xxx..." (2+ lines)
    expect(lines.length).toBeGreaterThanOrEqual(3);
    // First line has "> "
    expect(lines[0]).toMatch(/^> short/);
    // Lines after hard \n have "  " prefix
    expect(lines[1]).toMatch(/^  x/);
  });
});
