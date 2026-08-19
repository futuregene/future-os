import { splitUserTextSegments } from "../userTextSegments";

describe("splitUserTextSegments", () => {
  test("recognizes a file mention and strips the leading ./", () => {
    expect(splitUserTextSegments("[poem.txt](./poem.txt) 里面内容是什么")).toEqual([
      { text: "poem.txt", kind: "mention", href: "poem.txt", key: 0 },
      { text: " 里面内容是什么", kind: "plain", key: 22 },
    ]);
  });

  test("recognizes an external link", () => {
    expect(splitUserTextSegments("see [docs](https://example.com/a) now")).toEqual([
      { text: "see ", kind: "plain", key: 0 },
      { text: "docs", kind: "link", href: "https://example.com/a", key: 4 },
      { text: " now", kind: "plain", key: 33 },
    ]);
  });

  test("supports angle-bracket mention paths with spaces", () => {
    expect(splitUserTextSegments("[my file](<./my file.md>)")).toEqual([
      { text: "my file", kind: "mention", href: "my file.md", key: 0 },
    ]);
  });

  test("leaves everything else literal", () => {
    const text = "a * b # c 1. [x](not-a-link) [y](ftp://z)";
    expect(splitUserTextSegments(text)).toEqual([{ text, kind: "plain", key: 0 }]);
  });
});
