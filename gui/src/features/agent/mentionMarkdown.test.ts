import { describe, expect, it } from "vitest";
import { parseMentionSegments } from "./mentionMarkdown";

describe("parseMentionSegments", () => {
  it("returns a single text segment when there is no mention", () => {
    expect(parseMentionSegments("just text")).toEqual([
      { text: "just text", mention: false, key: 0 },
    ]);
  });

  it("splits text around a mention and captures its path", () => {
    const segments = parseMentionSegments("see [a.ts](./src/a.ts) here");
    expect(segments).toEqual([
      { text: "see ", mention: false, key: 0 },
      { text: "a.ts", mention: true, path: "./src/a.ts", key: 4 },
      { text: " here", mention: false, key: 22 },
    ]);
  });

  it("captures the angle-wrapped path form (spaces/parens in path)", () => {
    const segments = parseMentionSegments("[my file.ts](<./a b/my file.ts>)");
    expect(segments).toEqual([
      { text: "my file.ts", mention: true, path: "./a b/my file.ts", key: 0 },
    ]);
  });

  it("handles adjacent mentions and mentions touching text", () => {
    const segments = parseMentionSegments("[a](./a)[b](./b)x");
    expect(segments).toEqual([
      { text: "a", mention: true, path: "./a", key: 0 },
      { text: "b", mention: true, path: "./b", key: 8 },
      { text: "x", mention: false, key: 16 },
    ]);
  });

  it("leaves a non-relative link as plain text (only ./ paths are mentions)", () => {
    const segments = parseMentionSegments("[site](https://x.com)");
    expect(segments).toEqual([
      { text: "[site](https://x.com)", mention: false, key: 0 },
    ]);
  });
});
