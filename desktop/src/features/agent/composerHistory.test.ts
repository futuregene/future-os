import { beforeEach, describe, expect, it } from "vitest";
import { createComposerHistoryCursor, loadComposerHistory, pushComposerHistory } from "./composerHistory";

beforeEach(() => {
  sessionStorage.clear();
});

describe("composer history", () => {
  it("keeps what a conversation sent, newest last", () => {
    pushComposerHistory("t1", "first");
    pushComposerHistory("t1", "second");
    expect(loadComposerHistory("t1")).toEqual(["first", "second"]);
  });

  it("is per conversation", () => {
    pushComposerHistory("t1", "mine");
    expect(loadComposerHistory("t2")).toEqual([]);
  });

  it("collapses a re-sent message instead of storing it twice", () => {
    // A delivery that failed and was retried must not need two `↑` presses.
    pushComposerHistory("t1", "same");
    pushComposerHistory("t1", "same");
    expect(loadComposerHistory("t1")).toEqual(["same"]);
  });

  it("ignores an empty submission", () => {
    pushComposerHistory("t1", "   ");
    expect(loadComposerHistory("t1")).toEqual([]);
  });

  it("reads a store that is valid JSON but not a list as empty", () => {
    // A hand-edited or foreign write: an object here is not a history, and reading
    // entries off it would be a guess.
    sessionStorage.setItem("composer-history:t1", JSON.stringify({ entries: ["x"] }));
    expect(loadComposerHistory("t1")).toEqual([]);
    sessionStorage.setItem("composer-history:t1", '"one message"');
    expect(loadComposerHistory("t1")).toEqual([]);
  });

  it("survives a corrupt store", () => {
    sessionStorage.setItem("composer-history:t1", "{not json");
    expect(loadComposerHistory("t1")).toEqual([]);
  });

  it("walks back from the newest, then forward to an empty box", () => {
    pushComposerHistory("t1", "one");
    pushComposerHistory("t1", "two");
    const cursor = createComposerHistoryCursor("t1");

    expect(cursor.previous()).toBe("two");
    expect(cursor.previous()).toBe("one");
    // The oldest entry is the end of the history: pressing again holds there
    // rather than pretending there is more.
    expect(cursor.previous()).toBe("one");

    expect(cursor.next()).toBe("two");
    // Past the newest the box empties — not "the draft you had before ↑".
    expect(cursor.next()).toBe("");
    // …and the walk is over: the next `↓` is the editor's again.
    expect(cursor.next()).toBeNull();
  });

  it("says nothing when the conversation has not sent anything yet", () => {
    const cursor = createComposerHistoryCursor("empty");
    expect(cursor.previous()).toBeNull();
    expect(cursor.next()).toBeNull();
  });

  it("sees a message this composer sends while it is open", () => {
    // The cursor keeps its own copy: the walk must not re-read storage on every
    // key press, and must still include the message just sent.
    const cursor = createComposerHistoryCursor("t1");
    cursor.push("just sent");
    expect(cursor.previous()).toBe("just sent");
  });

  it("resets the walk when the reader types again", () => {
    pushComposerHistory("t1", "one");
    pushComposerHistory("t1", "two");
    const cursor = createComposerHistoryCursor("t1");
    expect(cursor.previous()).toBe("two");
    cursor.reset();
    expect(cursor.previous()).toBe("two");
  });
});
