import { collectQueuedAttachments, createSendQueueStore, joinQueuedContent } from "../../lib/sendQueue";

/** A store per test, so one case's queue never leaks into the next. */
function makeStore() {
  return createSendQueueStore<string>();
}

test("keeps messages in order, in their own conversation, with empty payloads out", () => {
  const store = makeStore();
  expect(store.enqueue("d1:s1", { content: "  " })).toBeNull();
  store.enqueue("d1:s1", { content: " first " });
  store.enqueue("d1:s1", { content: "second" });
  expect(store.snapshot("d1:s1").items.map(item => item.content)).toEqual(["first", "second"]);
  // The same session id on another desktop is another conversation.
  expect(store.snapshot("d2:s1").items).toEqual([]);
});

test("takes everything at once, and gives it back on failure as a failed queue", () => {
  const store = makeStore();
  store.enqueue("k", { content: "a" });
  store.enqueue("k", { content: "b" });
  const taken = store.take("k");
  expect(taken.map(item => item.content)).toEqual(["a", "b"]);
  expect(store.snapshot("k").items).toEqual([]);
  expect(store.take("k")).toEqual([]);

  store.restore("k", taken);
  expect(store.snapshot("k").items.map(item => item.content)).toEqual(["a", "b"]);
  // A failed delivery is a brake on automatic retries.
  expect(store.snapshot("k").failed).toBe(true);
  store.clearFailure("k");
  expect(store.snapshot("k").failed).toBe(false);
});

test("edits a queued message, and removes one whose last words are deleted", () => {
  const store = makeStore();
  const item = store.enqueue("k", { content: "typo", attachments: ["f.png"] })!;
  store.update("k", item.id, "fixed");
  expect(store.snapshot("k").items[0]!.content).toBe("fixed");
  // The file still travels, so emptying the words keeps the row.
  store.update("k", item.id, "  ");
  expect(store.snapshot("k").items).toHaveLength(1);

  const textOnly = store.enqueue("k", { content: "no files" })!;
  store.update("k", textOnly.id, " ");
  expect(store.snapshot("k").items.map(entry => entry.content)).toEqual([""]);
});

test("notifies only the conversation whose queue changed", () => {
  const store = makeStore();
  const item = store.enqueue("a", { content: "x" })!;
  store.enqueue("b", { content: "y" });
  const seen: string[] = [];
  const offA = store.subscribe("a", () => seen.push("a"));
  const offB = store.subscribe("b", () => seen.push("b"));
  store.remove("a", item.id);
  expect(seen).toEqual(["a"]);
  expect(store.snapshot("b").items).toHaveLength(1);
  offA();
  offB();
});

test("caps a conversation's queue instead of growing without bound", () => {
  const store = makeStore();
  for (let index = 0; index < 25; index += 1)
    store.enqueue("k", { content: `message ${index}` });
  expect(store.snapshot("k").items).toHaveLength(20);
  expect(store.enqueue("k", { content: "overflow" })).toBeNull();
});

test("sends the queue as one message, keeping attachment-only entries' files", () => {
  const store = makeStore();
  store.enqueue("k", { content: "", attachments: ["only.png"] });
  store.enqueue("k", { content: " words ", attachments: ["doc.pdf"] });
  const items = store.snapshot("k").items;
  expect(joinQueuedContent(items)).toBe("words");
  expect(collectQueuedAttachments(items)).toEqual(["only.png", "doc.pdf"]);
});
