import { completeSessionReference, sessionQuery } from "../sessionCompletion";

const at = (text: string, cursor = text.length) => sessionQuery(text, { start: cursor, end: cursor });

describe("sessionQuery", () => {
  it("reads the query from a `#` token at the caret", () => {
    expect(at("#")).toEqual({ start: 0, end: 1, query: "" });
    expect(at("#fix")).toEqual({ start: 0, end: 4, query: "fix" });
    expect(at("look at #修复 这里", 11)).toEqual({ start: 8, end: 11, query: "修复" });
  });

  it("ignores a `#` that is not opening a standalone token", () => {
    // `##` is a markdown heading and `issue#12` is an id — neither is a trigger.
    expect(at("## heading")).toBeNull();
    expect(at("issue#12")).toBeNull();
    expect(at("#done ")).toBeNull();
    expect(at("a#", 2)).toBeNull();
  });

  it("ignores a selection, and extends over the rest of a token the caret sits inside", () => {
    expect(sessionQuery("#fix", { start: 1, end: 4 })).toBeNull();
    // The tail belongs to the same token, so completion replaces the whole token
    // rather than leaving a fragment behind (the same contract as `skillQuery`).
    expect(at("#fix", 3)).toEqual({ start: 0, end: 4, query: "fi" });
  });
});

describe("completeSessionReference", () => {
  it("replaces the typed token with a link carrying the session id", () => {
    const query = at("ask #flaky")!;
    const next = completeSessionReference("ask #flaky", query, { sessionId: "s-1", title: "Fix the flaky test" });
    expect(next.text).toBe("ask [Fix the flaky test](futureos://session/s-1) ");
    expect(next.selection).toEqual({ start: 49, end: 49 });
  });

  it("keeps the text after the token and does not double the separator", () => {
    const query = at("ask #fix 后文", 8)!;
    const next = completeSessionReference("ask #fix 后文", query, { sessionId: "s-1", title: "Fix" });
    expect(next.text).toBe("ask [Fix](futureos://session/s-1) 后文");
  });

  it("falls back to the id when the title is blank", () => {
    const query = at("#")!;
    const next = completeSessionReference("#", query, { sessionId: "s-9", title: "  " });
    expect(next.text).toBe("[s-9](futureos://session/s-9) ");
  });
});
