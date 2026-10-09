import { describe, expect, it } from "vitest";
import {
  buildSessionReference,
  findSessionReferences,
  parseSessionReferenceHref,
} from "./sessionReference";

const ID = "20261007-005402-418721fcc201479e8551b2ccf54cda2f";

describe("buildSessionReference", () => {
  it("carries the session id in the link destination", () => {
    expect(buildSessionReference({ sessionId: ID, title: "Fix the flaky test" }))
      .toBe(`[Fix the flaky test](futureos://session/${ID})`);
  });

  it("strips brackets that would close the link early", () => {
    // A `]` in the title would end the label and leave the id outside the link.
    const text = buildSessionReference({ sessionId: ID, title: "fix [a] bug" });
    expect(text).toBe(`[fix a bug](futureos://session/${ID})`);
    expect(findSessionReferences(text)[0]?.sessionId).toBe(ID);
  });

  it("falls back to the id when the title has nothing left", () => {
    expect(buildSessionReference({ sessionId: ID, title: "   " }))
      .toBe(`[${ID}](futureos://session/${ID})`);
  });

  it("collapses whitespace so a multi-line title stays one link", () => {
    expect(buildSessionReference({ sessionId: ID, title: " a\n b " }))
      .toBe(`[a b](futureos://session/${ID})`);
  });
});

describe("parseSessionReferenceHref", () => {
  it("reads the id back", () => {
    expect(parseSessionReferenceHref(`futureos://session/${ID}`)).toBe(ID);
  });

  it("ignores other schemes, hosts and empty ids", () => {
    for (const href of [
      "https://example.com/session/x",
      `futureos://run/${ID}`,
      "futureos://session/",
      "futureos://session/has space",
      "./docs/a.md",
    ]) {
      expect(parseSessionReferenceHref(href)).toBeNull();
    }
  });
});

describe("findSessionReferences", () => {
  it("finds references among prose and reports their offsets", () => {
    const text = `look at [A](futureos://session/one) and [B B](futureos://session/two)`;
    const matches = findSessionReferences(text);
    expect(matches.map(match => match.sessionId)).toEqual(["one", "two"]);
    expect(matches.map(match => match.title)).toEqual(["A", "B B"]);
    expect(text.slice(matches[0]!.index, matches[0]!.index + matches[0]!.length))
      .toBe("[A](futureos://session/one)");
  });

  it("returns nothing for plain text and for other links", () => {
    expect(findSessionReferences("no references here")).toEqual([]);
    expect(findSessionReferences("[x](./a.md) [y](https://a.b)")).toEqual([]);
  });

  it("does not leak the previous call's lastIndex into the next call", () => {
    const text = "x [A](futureos://session/one)";
    expect(findSessionReferences(text)).toHaveLength(1);
    expect(findSessionReferences(text)).toHaveLength(1);
  });
});
