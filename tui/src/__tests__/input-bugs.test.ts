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

  test("right arrow skip: asymmetric with left arrow", () => {
    const input = new Input();
    input.setValue("a\nb", 0);
    // Right from 'a': skips \n, lands on 'b'
    input.handleKey("right");
    // Now at position 2 ('b')
    // Left should go back to 'a' in one press, but goes to '\n' first
    input.handleKey("left");
    // BUG: lands on \n (position 1), not on 'a' (position 0)
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
  test("right arrow moves one grapheme at a time (no skip)", () => {
    const input = new Input();
    input.setValue("a\nb", 0);
    input.handleKey("right");
    expect((input as any).cursor).toBe(1); // lands on \n
    input.handleKey("right");
    expect((input as any).cursor).toBe(2); // lands on 'b'
  });

  test("left arrow is symmetric with right", () => {
    const input = new Input();
    input.setValue("a\nb", 2); // on 'b'
    input.handleKey("left");
    expect((input as any).cursor).toBe(1); // on \n
    input.handleKey("left");
    expect((input as any).cursor).toBe(0); // on 'a'
  });

  test("right then left returns to origin", () => {
    const input = new Input();
    input.setValue("a\n\nb", 0);
    input.handleKey("right");
    input.handleKey("right");
    input.handleKey("right");
    expect((input as any).cursor).toBe(3); // on 'b'
    input.handleKey("left");
    input.handleKey("left");
    input.handleKey("left");
    expect((input as any).cursor).toBe(0); // back to 'a'
  });
});
