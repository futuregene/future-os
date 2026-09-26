import { codePreviewRows, codeRowText } from "../codePreviewRows";

test.each(["x".repeat(200000), "\n".repeat(100000), "中🙂\n".repeat(10000), "a\r\nb\n", ""]) (
  "bounded code chunks preserve the entire original source", source => {
    const rows = codePreviewRows(source);
    expect(rows.map(row => row.text).join("")).toBe(source);
    expect(rows.every(row => row.text.length <= 2049)).toBe(true);
    expect(rows.every(row => !/[\uD800-\uDBFF]$/.test(row.text))).toBe(true);
    if (source.length > 10000) expect(rows.length).toBeLessThan(source.length / 20);
  },
);

test("a chunk boundary inside a surrogate pair moves past the pair instead of splitting it", () => {
  // The 2048-char chunk ends exactly on the emoji's high surrogate. Splitting
  // there would render a replacement glyph and, worse, leave the file's bytes
  // unrecoverable from the rows.
  const source = "a".repeat(2047) + "🙂" + "b".repeat(10);
  const rows = codePreviewRows(source);
  expect(rows[0]!.text).toBe("a".repeat(2047) + "🙂");
  expect(rows).toHaveLength(2);
  expect(rows[1]!.text).toBe("b".repeat(10));
  expect(rows.map(row => row.text).join("")).toBe(source);
});

test("a continuation row is marked only when it starts mid-line", () => {
  const wrapped = codePreviewRows("x".repeat(3000));
  expect(wrapped.map(row => row.continuation)).toEqual([false, true]);
  const atBreaks = codePreviewRows(("y".repeat(200) + "\n").repeat(20));
  expect(atBreaks.every(row => row.continuation === false)).toBe(true);
});

test.each([
  ["a\n", "a"],
  ["a", "a"],
  ["\n", ""],
  ["", ""],
  ["a\n\n", "a\n"],
  ["中\n", "中"],
])("a painted row drops exactly one trailing newline: %j", (text, expected) => {
  expect(codeRowText(text)).toBe(expected);
});
