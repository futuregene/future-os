import { describe, expect, it } from "vitest";
import { splitExternalLinkSegments } from "./externalLinks";

describe("splitExternalLinkSegments", () => {
  it("returns the whole text as one literal segment when there is no link", () => {
    expect(splitExternalLinkSegments("plain text")).toEqual([
      { text: "plain text", link: false, key: 0 },
    ]);
  });

  it("splits a markdown link into a link segment", () => {
    expect(splitExternalLinkSegments("见 [手册](https://example.com/manual) 了解")).toEqual([
      { text: "见 ", link: false, key: 0 },
      { text: "手册", link: true, href: "https://example.com/manual", key: 2 },
      { text: " 了解", link: false, key: 2 + "[手册](https://example.com/manual)".length },
    ]);
  });

  it("handles a leading and a trailing link", () => {
    expect(splitExternalLinkSegments("[a](https://x.com/1) and [b](https://x.com/2)")).toEqual([
      { text: "a", link: true, href: "https://x.com/1", key: 0 },
      { text: " and ", link: false, key: "[a](https://x.com/1)".length },
      { text: "b", link: true, href: "https://x.com/2", key: "[a](https://x.com/1) and ".length },
    ]);
  });

  it("ignores non-http targets and bracket-less text", () => {
    expect(splitExternalLinkSegments("[x](./path) [y](ftp://x) [z](not-a-url)")).toEqual([
      { text: "[x](./path) [y](ftp://x) [z](not-a-url)", link: false, key: 0 },
    ]);
  });

  it("returns no segments for an empty string", () => {
    expect(splitExternalLinkSegments("")).toEqual([]);
  });
});
