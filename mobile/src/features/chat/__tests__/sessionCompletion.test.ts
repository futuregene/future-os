import {
  completeSessionReference,
  expandSessionReferences,
  pruneSessionReferences,
  sessionQuery,
  sessionReferenceToken,
} from "../sessionCompletion";

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

describe("sessionReferenceToken", () => {
  it("stands for a conversation as `#title`, the way the desktop pill reads", () => {
    expect(sessionReferenceToken({ sessionId: "s-1", title: "Fix the flaky test" }))
      .toBe("#Fix the flaky test");
  });

  it("keeps the token on one line and never leaves it bare", () => {
    // The token is what the user sees in a single-line-ish TextInput.
    expect(sessionReferenceToken({ sessionId: "s-1", title: "  two\n words  " }))
      .toBe("#two words");
    expect(sessionReferenceToken({ sessionId: "abcde12345", title: "   " }))
      .toBe("#12345");
  });

  it("distinguishes a title two conversations share", () => {
    const first = { sessionId: "sess-one", title: "未命名" };
    const second = { sessionId: "sess-two", title: "未命名" };
    const token = sessionReferenceToken(first, {});
    // Without this the second pick would expand to the first conversation.
    expect(sessionReferenceToken(second, { [token]: first })).toBe("#未命名·s-two");
    // The same conversation picked twice keeps the plain token.
    expect(sessionReferenceToken(first, { [token]: first })).toBe("#未命名");
  });
});

describe("completeSessionReference", () => {
  it("replaces the typed token with the compact form, not the link", () => {
    const query = at("ask #flaky")!;
    const next = completeSessionReference("ask #flaky", query, { sessionId: "s-1", title: "Fix the flaky test" });
    expect(next.text).toBe("ask #Fix the flaky test ");
    expect(next.token).toBe("#Fix the flaky test");
    expect(next.selection).toEqual({ start: 24, end: 24 });
  });

  it("keeps the text after the token and does not double the separator", () => {
    const query = at("ask #fix 后文", 8)!;
    const next = completeSessionReference("ask #fix 后文", query, { sessionId: "s-1", title: "Fix" });
    expect(next.text).toBe("ask #Fix 后文");
  });
});

describe("expandSessionReferences", () => {
  it("turns a token into the link the message must carry", () => {
    const refs = { "#Fix the flaky test": { sessionId: "s-1", title: "Fix the flaky test" } };
    expect(expandSessionReferences("ask #Fix the flaky test please", refs))
      .toBe("ask [Fix the flaky test](futureos://session/s-1) please");
  });

  it("expands every occurrence and leaves unrelated text alone", () => {
    const refs = { "#A": { sessionId: "s-a", title: "A" } };
    expect(expandSessionReferences("#A and #A", refs))
      .toBe("[A](futureos://session/s-a) and [A](futureos://session/s-a)");
    expect(expandSessionReferences("no tokens here", refs)).toBe("no tokens here");
  });

  it("resolves a token that is a prefix of another without corrupting it", () => {
    // `#未命名` is a prefix of `#未命名·-two`: shortest-first expansion would
    // rewrite the longer token's head and lose the second conversation.
    const refs = {
      "#未命名": { sessionId: "s-one", title: "未命名" },
      "#未命名·-two": { sessionId: "s-two", title: "未命名" },
    };
    expect(expandSessionReferences("#未命名·-two", refs))
      .toBe("[未命名](futureos://session/s-two)");
  });

  it("leaves a token whose conversation is no longer remembered as plain text", () => {
    expect(expandSessionReferences("ask #Gone", {})).toBe("ask #Gone");
  });
});

describe("pruneSessionReferences", () => {
  it("drops the references the draft no longer mentions", () => {
    const refs = {
      "#A": { sessionId: "s-a", title: "A" },
      "#B": { sessionId: "s-b", title: "B" },
    };
    expect(pruneSessionReferences(refs, "only #A here")).toEqual({
      "#A": { sessionId: "s-a", title: "A" },
    });
    // After a send the draft is empty, so nothing is kept.
    expect(pruneSessionReferences(refs, "")).toEqual({});
  });

  it("returns the same object when nothing changed, so a state update can skip", () => {
    const refs = { "#A": { sessionId: "s-a", title: "A" } };
    expect(pruneSessionReferences(refs, "#A")).toBe(refs);
  });
});
