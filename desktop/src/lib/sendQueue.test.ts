import { describe, expect, it } from "vitest";
import { collectQueuedAttachments, createSendQueueStore, joinQueuedContent } from "./sendQueue";

/** A store per test, so one case's queue never leaks into the next. */
function makeStore() {
  return createSendQueueStore<string>();
}

describe("send queue store", () => {
  it("keeps messages in order and empty payloads out", () => {
    const store = makeStore();
    expect(store.enqueue("t1", { content: "   " })).toBeNull();
    store.enqueue("t1", { content: " first " });
    store.enqueue("t1", { content: "second" });
    expect(store.snapshot("t1").items.map(item => item.content)).toEqual(["first", "second"]);
    // Another conversation has its own queue.
    expect(store.snapshot("t2").items).toEqual([]);
  });

  it("takes everything at once and gives it back on failure", () => {
    const store = makeStore();
    store.enqueue("t1", { content: "a" });
    store.enqueue("t1", { content: "b" });
    const taken = store.take("t1");
    expect(taken.map(item => item.content)).toEqual(["a", "b"]);
    expect(store.snapshot("t1").items).toEqual([]);
    expect(store.take("t1")).toEqual([]);

    store.restore("t1", taken);
    expect(store.snapshot("t1").items.map(item => item.content)).toEqual(["a", "b"]);
    // A failed delivery is a brake: the view must not retry per render.
    expect(store.snapshot("t1").failed).toBe(true);
    store.clearFailure("t1");
    expect(store.snapshot("t1").failed).toBe(false);
  });

  it("edits a queued message, and removes one whose last words are deleted", () => {
    const store = makeStore();
    const item = store.enqueue("t1", { content: "typo", attachments: ["f.png"] })!;
    store.update("t1", item.id, "fixed");
    expect(store.snapshot("t1").items[0]!.content).toBe("fixed");
    // The attachment still travels, so emptying the text keeps the row.
    store.update("t1", item.id, "   ");
    expect(store.snapshot("t1").items).toHaveLength(1);

    const textOnly = store.enqueue("t1", { content: "no files" })!;
    store.update("t1", textOnly.id, "  ");
    expect(store.snapshot("t1").items.map(entry => entry.content)).toEqual([""]);
  });

  it("removes an item and notifies only its own conversation", () => {
    const store = makeStore();
    const item = store.enqueue("t1", { content: "a" })!;
    store.enqueue("t2", { content: "b" });
    const seen: string[] = [];
    const unsubscribe = store.subscribe("t1", () => seen.push("t1"));
    const unsubscribeOther = store.subscribe("t2", () => seen.push("t2"));
    store.remove("t1", item.id);
    expect(seen).toEqual(["t1"]);
    expect(store.snapshot("t2").items).toHaveLength(1);
    unsubscribe();
    unsubscribeOther();
  });

  it("caps a conversation's queue instead of growing without bound", () => {
    const store = makeStore();
    for (let index = 0; index < 25; index += 1)
      store.enqueue("t1", { content: `message ${index}` });
    expect(store.snapshot("t1").items).toHaveLength(20);
    expect(store.enqueue("t1", { content: "overflow" })).toBeNull();
  });
});

describe("queue joining", () => {
  it("sends the queue as one message, in order, one paragraph each", () => {
    expect(joinQueuedContent([{ content: "first" }, { content: "  second  " }])).toBe("first\n\nsecond");
  });

  it("drops attachment-only entries from the text but keeps their files", () => {
    const store = makeStore();
    store.enqueue("t1", { content: "", attachments: ["only.png"] });
    store.enqueue("t1", { content: "words", attachments: ["doc.pdf"] });
    const items = store.snapshot("t1").items;
    expect(joinQueuedContent(items)).toBe("words");
    expect(collectQueuedAttachments(items)).toEqual(["only.png", "doc.pdf"]);
  });
});
