import { codePreviewRows } from "../codePreviewRows";

test.each(["x".repeat(200000), "\n".repeat(100000), "中🙂\n".repeat(10000), "a\r\nb\n", ""]) (
  "bounded code chunks preserve the entire original source", source => {
    const rows = codePreviewRows(source);
    expect(rows.map(row => row.text).join("")).toBe(source);
    expect(rows.every(row => row.text.length <= 2049)).toBe(true);
    expect(rows.every(row => !/[\uD800-\uDBFF]$/.test(row.text))).toBe(true);
    if (source.length > 10000) expect(rows.length).toBeLessThan(source.length / 20);
  },
);
