import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { JsonPreview, formatJsonForPreview, tokenizeJsonLine } from "../JsonPreview";

describe("JsonPreview", () => {
  test("formats nested JSON without changing numeric spelling or escapes", () => {
    const source =
      '{"sample":{"id":900719925474099312345,"ratio":1.2300e-08,"label":"a,b:{c}","empty":[]}}';
    const formatted = formatJsonForPreview(source);

    expect(formatted.limited).toBe(false);
    expect(formatted.lines).toEqual([
      "{",
      '  "sample": {',
      '    "id": 900719925474099312345,',
      '    "ratio": 1.2300e-08,',
      '    "label": "a,b:{c}",',
      '    "empty": []',
      "  }",
      "}",
    ]);
  });

  test("does not treat punctuation or escaped quotes inside strings as structure", () => {
    const formatted = formatJsonForPreview('{"value":"line\\n\\\"quoted\\\": [x]"}');
    expect(formatted.lines).toEqual(["{", '  "value": "line\\n\\\"quoted\\\": [x]"', "}"]);
  });

  test("classifies keys, strings, numbers and JSON literals", () => {
    expect(tokenizeJsonLine('  "key": ["value", -1.2e+3, true, false, null]')).toEqual([
      { text: "  ", kind: "plain" },
      { text: '"key"', kind: "key" },
      { text: ": [", kind: "plain" },
      { text: '"value"', kind: "string" },
      { text: ", ", kind: "plain" },
      { text: "-1.2e+3", kind: "number" },
      { text: ", ", kind: "plain" },
      { text: "true", kind: "literal" },
      { text: ", ", kind: "plain" },
      { text: "false", kind: "literal" },
      { text: ", ", kind: "plain" },
      { text: "null", kind: "literal" },
      { text: "]", kind: "plain" },
    ]);
  });

  test("tokenizes strings with many escaped quotes without backtracking", () => {
    const text = '"' + '\\"'.repeat(20_000) + '"';
    expect(tokenizeJsonLine(text)).toEqual([{ text, kind: "string" }]);
  });

  test("stops pathological nesting before indentation becomes unbounded", () => {
    const formatted = formatJsonForPreview("[".repeat(200));
    expect(formatted.limited).toBe(true);
    expect(formatted.lines.length).toBeLessThanOrEqual(128);
  });
});

/**
 * The component is the only consumer of the shared formatter, and it decides
 * which of the three notices a reader sees. Rendered through the real FlatList,
 * so what is asserted is the row/token layout rather than a helper's return.
 */
describe("the JSON preview surface", () => {
  let tree: ReactTestRenderer;

  afterEach(() => { if (tree) act(() => tree.unmount()); });

  function render(text: string, extra: Partial<Parameters<typeof JsonPreview>[0]> = {}) {
    const messages = {
      invalidMessage: (detail: string) => `invalid: ${detail}`,
      tooComplexMessage: "too complex to format",
      truncatedMessage: "source was truncated",
      ...extra,
    };
    act(() => {
      tree = create(createElement(JsonPreview, { sourceTruncated: false, text, ...messages }));
    });
    return messages;
  }
  /** Every string actually painted, in render order (row numbers are numbers). */
  function painted() {
    return tree.root
      .findAll(node => typeof node.type === "string"
        && (typeof node.props.children === "string" || typeof node.props.children === "number"))
      .map(node => String(node.props.children));
  }
  function has(text: string) { return painted().includes(text); }

  test("valid JSON numbers every row and paints its tokens", () => {
    render('{"a": 1}');
    // Three source lines → three numbered rows (1..3), and each token of the
    // single property line is painted with its own style class.
    expect(painted()).toEqual(expect.arrayContaining(["1", "2", "3", "{", "}", '"a"', ": ", "1"]));
  });

  test("a truncated source says so instead of reporting a parse error for the missing tail", () => {
    const messages = render('{"a": 1', { sourceTruncated: true });
    expect(has(messages.truncatedMessage)).toBe(true);
    expect(painted().some(line => line.startsWith("invalid: "))).toBe(false);
  });

  test("invalid JSON is reported with the parser's own complaint, not a generic error", () => {
    render('{"a": }');
    const notices = painted().filter(line => line.startsWith("invalid: "));
    expect(notices).toHaveLength(1);
    expect(notices[0]!.length).toBeGreaterThan("invalid: ".length);
    expect(has("too complex to format")).toBe(false);
  });

  test("a source deeper than the formatter allows says so and still renders lines", () => {
    render("[".repeat(200) + "]".repeat(200));
    expect(has("too complex to format")).toBe(true);
    expect(has("1")).toBe(true);
    expect(painted().filter(line => line === "[").length).toBeGreaterThan(0);
  });

  test("an invalid source falls back to raw lines rather than claiming complexity", () => {
    render("[".repeat(200));
    expect(has("too complex to format")).toBe(false);
    // Nothing is dropped on the fallback path: the whole malformed line is
    // painted verbatim rather than being justified into a formatted shape.
    expect(painted()).toContain("[".repeat(200));
  });

  test("a truncated and unformattable source shows both notices, in that order", () => {
    render("[".repeat(200) + "]".repeat(200), { sourceTruncated: true });
    const lines = painted();
    const truncated = lines.indexOf("source was truncated");
    const tooComplex = lines.indexOf("too complex to format");
    expect(truncated).toBeGreaterThanOrEqual(0);
    expect(tooComplex).toBeGreaterThan(truncated);
  });

  test("an empty source is reported as invalid, not silently blank", () => {
    render("");
    expect(painted().some(line => line.startsWith("invalid: "))).toBe(true);
    expect(has("1")).toBe(true);
    act(() => tree.unmount());
    render("   ");
    expect(painted().some(line => line.startsWith("invalid: "))).toBe(true);
  });
});
