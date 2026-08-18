import { formatJsonForPreview, tokenizeJsonLine } from "../JsonPreview";

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

  test("stops pathological nesting before indentation becomes unbounded", () => {
    const formatted = formatJsonForPreview("[".repeat(200));
    expect(formatted.limited).toBe(true);
    expect(formatted.lines.length).toBeLessThanOrEqual(128);
  });
});
