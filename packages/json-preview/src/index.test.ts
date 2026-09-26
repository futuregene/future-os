import { describe, expect, it } from "vitest";
import {
  MAX_JSON_DEPTH,
  MAX_JSON_LINES,
  MAX_RAW_JSON_LINE_CHARS,
  formatJsonForPreview,
  rawJsonLines,
  tokenizeJsonLine,
} from "./index";

describe("formatJsonForPreview", () => {
  it("pretty-prints nested objects, arrays and scalars", () => {
    const source = '{"a":1,"b":[true,null,"x"],"c":{"d":1.5e3}}';
    expect(formatJsonForPreview(source)).toEqual({
      limited: false,
      lines: [
        "{",
        '  "a": 1,',
        '  "b": [',
        "    true,",
        "    null,",
        '    "x"',
        "  ],",
        '  "c": {',
        "    \"d\": 1.5e3",
        "  }",
        "}",
      ],
    });
  });

  it("keeps empty containers on one line", () => {
    expect(formatJsonForPreview("{}").lines).toEqual(["{}"]);
    expect(formatJsonForPreview("[]").lines).toEqual(["[]"]);
    expect(formatJsonForPreview("[ ]").lines).toEqual(["[]"]);
    expect(formatJsonForPreview('{"a":{}}').lines).toEqual(["{", '  "a": {}', "}"]);
  });

  it("returns a single empty line for empty or whitespace-only input", () => {
    expect(formatJsonForPreview("")).toEqual({ limited: false, lines: [""] });
    expect(formatJsonForPreview("  \n\t ")).toEqual({ limited: false, lines: [""] });
  });

  it("preserves escape sequences and braces inside strings verbatim", () => {
    const source = String.raw`{"k":"a\"b\\{c}","n":"line\nbreak"}`;
    expect(formatJsonForPreview(source).lines).toEqual([
      "{",
      String.raw`  "k": "a\"b\\{c}",`,
      String.raw`  "n": "line\nbreak"`,
      "}",
    ]);
  });

  it("tolerates a trailing unterminated string and does not verify JSON syntax", () => {
    const result = formatJsonForPreview('{"a": "unterminated');
    expect(result.limited).toBe(false);
    expect(result.lines.join("")).toContain("unterminated");
  });

  it("stops after MAX_JSON_LINES and reports the truncation", () => {
    // 49 999 commas → 50 000 pushed lines, then the closing bracket tries to
    // push one more and the limit trips.
    const source = `[${"1,".repeat(MAX_JSON_LINES - 1)}1]`;
    const result = formatJsonForPreview(source);
    expect(result.limited).toBe(true);
    expect(result.lines).toHaveLength(MAX_JSON_LINES);
  });

  it("trips the depth guard on nesting deeper than MAX_JSON_DEPTH", () => {
    const deep = `${"[".repeat(MAX_JSON_DEPTH + 1)}${"]".repeat(MAX_JSON_DEPTH + 1)}`;
    const result = formatJsonForPreview(deep);
    expect(result.limited).toBe(true);
    expect(result.lines.length).toBeGreaterThan(0);

    const atLimit = `${"[".repeat(MAX_JSON_DEPTH)}${"]".repeat(MAX_JSON_DEPTH)}`;
    expect(formatJsonForPreview(atLimit).limited).toBe(false);
  });

  it("treats a lone opening brace followed by whitespace as a normal line", () => {
    // Exercises nextNonWhitespace running to the end of the source (no
    // closing bracket to compare against) and a whitespace-only tail.
    expect(formatJsonForPreview("{   ")).toEqual({ limited: false, lines: ["{"] });
  });
});

describe("tokenizeJsonLine", () => {
  it("classifies keys, strings, numbers and literals", () => {
    expect(tokenizeJsonLine('  "a": 1,')).toEqual([
      { kind: "plain", text: "  " },
      { kind: "key", text: '"a"' },
      { kind: "plain", text: ": " },
      { kind: "number", text: "1" },
      { kind: "plain", text: "," },
    ]);
    expect(tokenizeJsonLine('"value"')).toEqual([{ kind: "string", text: '"value"' }]);
    expect(tokenizeJsonLine("true")).toEqual([{ kind: "literal", text: "true" }]);
    expect(tokenizeJsonLine("false")).toEqual([{ kind: "literal", text: "false" }]);
    expect(tokenizeJsonLine("null")).toEqual([{ kind: "literal", text: "null" }]);
  });

  it.each([
    ["-1", "-1"],
    ["0", "0"],
    ["42", "42"],
    ["1.25", "1.25"],
    ["1e5", "1e5"],
    ["1E+5", "1E+5"],
    ["2e-3", "2e-3"],
    ["-0.5e10", "-0.5e10"],
  ])("scans the JSON number %s", (line, expected) => {
    expect(tokenizeJsonLine(line)).toEqual([{ kind: "number", text: expected }]);
  });

  it("leaves malformed number spellings as plain text", () => {
    expect(tokenizeJsonLine("1e")).toEqual([{ kind: "plain", text: "1e" }]);
    expect(tokenizeJsonLine("a1")).toEqual([{ kind: "plain", text: "a1" }]);
    expect(tokenizeJsonLine("1abc")).toEqual([{ kind: "plain", text: "1abc" }]);
    expect(tokenizeJsonLine(".5")).toEqual([
      { kind: "plain", text: "." },
      { kind: "number", text: "5" },
    ]);
  });

  it("requires literal word boundaries", () => {
    expect(tokenizeJsonLine("truex")).toEqual([{ kind: "plain", text: "truex" }]);
    expect(tokenizeJsonLine("atrue")).toEqual([{ kind: "plain", text: "atrue" }]);
    expect(tokenizeJsonLine("1null2")).toEqual([{ kind: "plain", text: "1null2" }]);
  });

  it("treats an underscore as a word character", () => {
    // `_` is neither a digit nor a letter, so it is only kept out of numbers and
    // literals because the word-character check names it explicitly.
    expect(tokenizeJsonLine("1_b")).toEqual([{ kind: "plain", text: "1_b" }]);
    expect(tokenizeJsonLine("_1")).toEqual([{ kind: "plain", text: "_1" }]);
    expect(tokenizeJsonLine("true_x")).toEqual([{ kind: "plain", text: "true_x" }]);
    expect(tokenizeJsonLine("null_1")).toEqual([{ kind: "plain", text: "null_1" }]);
    // A number followed by a non-word character is still a number.
    expect(tokenizeJsonLine("1-")).toEqual([
      { kind: "number", text: "1" },
      { kind: "plain", text: "-" },
    ]);
  });

  it("keeps an unterminated string as plain text", () => {
    expect(tokenizeJsonLine('"abc')).toEqual([{ kind: "plain", text: '"abc' }]);
  });

  it("scans an escaped string including the closing quote", () => {
    expect(tokenizeJsonLine('"a\\"b": 1')).toEqual([
      { kind: "key", text: '"a\\"b"' },
      { kind: "plain", text: ": " },
      { kind: "number", text: "1" },
    ]);
  });

  it("scans a string containing an emoji (surrogate pair) as one token", () => {
    expect(tokenizeJsonLine('"🙂": 1')).toEqual([
      { kind: "key", text: '"🙂"' },
      { kind: "plain", text: ": " },
      { kind: "number", text: "1" },
    ]);
  });

  it("treats a mismatched closing bracket as data instead of popping the stack", () => {
    // `{` closed by `]`: the depth bookkeeping must not pop on a mismatch, and
    // the malformed text still renders line-by-line.
    expect(formatJsonForPreview('{"a"]')).toEqual({
      limited: false,
      lines: ["{", '  "a"', "  ]"],
    });
    // A stray closer with no opener at all is data too.
    expect(formatJsonForPreview("]}")).toEqual({ limited: false, lines: ["]", "}"] });
  });

  it("keeps a trailing comma and whitespace inside containers", () => {
    expect(formatJsonForPreview("{ }").lines).toEqual(["{}"]);
    expect(formatJsonForPreview("[1,]").lines).toEqual(["[", "  1,", "]"]);
  });

  it("recognises a key whose colon is separated by whitespace", () => {
    expect(tokenizeJsonLine('"a" :')).toEqual([
      { kind: "key", text: '"a"' },
      { kind: "plain", text: " :" },
    ]);
    // No colon at all → a string, not a key. Also covers nextNonWhitespace
    // running off the end of the line.
    expect(tokenizeJsonLine('"a" ')).toEqual([
      { kind: "string", text: '"a"' },
      { kind: "plain", text: " " },
    ]);
  });
});

describe("rawJsonLines", () => {
  it("splits on LF and CRLF and keeps empty lines", () => {
    expect(rawJsonLines("a\r\nb\n\nc")).toEqual({
      limited: false,
      lines: ["a", "b", "", "c"],
    });
  });

  it("returns one empty line for empty input", () => {
    expect(rawJsonLines("")).toEqual({ limited: false, lines: [""] });
  });

  it("chunks a minified line so one text node cannot monopolise layout", () => {
    const long = "x".repeat(MAX_RAW_JSON_LINE_CHARS + 5);
    const result = rawJsonLines(long);
    expect(result.limited).toBe(false);
    expect(result.lines).toHaveLength(2);
    expect(result.lines[0]).toHaveLength(MAX_RAW_JSON_LINE_CHARS);
    expect(result.lines[1]).toHaveLength(5);
    expect(result.lines.join("")).toBe(long);
  });

  it("caps the reported lines at MAX_JSON_LINES", () => {
    const source = `${"a\n".repeat(MAX_JSON_LINES + 1)}tail`;
    const result = rawJsonLines(source);
    expect(result.limited).toBe(true);
    expect(result.lines).toHaveLength(MAX_JSON_LINES);
  });

  it("caps chunked output of a single oversized line", () => {
    const huge = "y".repeat(MAX_RAW_JSON_LINE_CHARS * (MAX_JSON_LINES + 2));
    const result = rawJsonLines(huge);
    expect(result.limited).toBe(true);
    expect(result.lines).toHaveLength(MAX_JSON_LINES);
  });
});
