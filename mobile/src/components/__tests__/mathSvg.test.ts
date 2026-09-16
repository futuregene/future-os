import { renderMathSvg } from "../mathSvg";

// Real offline layout, not a mocked TeX parser or a snapshot of the source.
test.each([
  String.raw`\sqrt[n]{a}=a^{\frac{1}{n}}`,
  String.raw`\sqrt{9}=9^{1/2}=3`,
  String.raw`\frac{a+b}{c}`,
  String.raw`\sum_{i=1}^{n} x_i`,
  String.raw`\begin{pmatrix}a&b\\c&d\end{pmatrix}`,
  String.raw`\begin{aligned}x&=1\\y&=2\end{aligned}`,
])("renders %s to bounded self-contained vector glyphs", code => {
  const result = renderMathSvg(code, true);
  expect(result).not.toBeNull();
  expect(result!.xml).toContain("<path");
  expect(result!.xml).not.toMatch(/<script|<foreignObject|<image|<use|href=/i);
  expect(result!.width).toBeGreaterThan(0);
  expect(result!.height).toBeGreaterThan(0);
  expect(result!.width).toBeLessThan(100);
  expect(result!.height).toBeLessThan(100);
  expect(renderMathSvg(code, true)).toBe(result);
});

test.each(["", "x".repeat(8_193), String.raw`\frac{a}{`, String.raw`\unknown{a}`, String.raw`\href{https://example.com}{x}`])(
  "unsupported, incomplete or excessive TeX falls back to source", code => {
    expect(renderMathSvg(code, false)).toBeNull();
  },
);

test("display and inline formulas use separate layouts", () => {
  const code = String.raw`\frac{1}{2}`;
  expect(renderMathSvg(code, true)!.height).toBeGreaterThan(renderMathSvg(code, false)!.height);
});
