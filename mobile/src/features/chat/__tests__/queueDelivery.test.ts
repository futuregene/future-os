import { batchCommandId, isTransientQueueRefusal } from "../queueDelivery";

test("a run that has not settled yet is 'not yet', not a refusal", () => {
  expect(isTransientQueueRefusal(new Error("session_busy"))).toBe(true);
  expect(isTransientQueueRefusal(new Error("request failed: session_busy"))).toBe(true);
  expect(isTransientQueueRefusal(new Error("send_busy"))).toBe(true);
});

test("everything else is a real refusal, so the reader gets the retry", () => {
  expect(isTransientQueueRefusal(new Error("prompt_too_large"))).toBe(false);
  expect(isTransientQueueRefusal(new Error("not_connected"))).toBe(false);
  expect(isTransientQueueRefusal(new Error("superseded"))).toBe(false);
  expect(isTransientQueueRefusal("session_busy")).toBe(false);
  expect(isTransientQueueRefusal(undefined)).toBe(false);
});

test("a batch answers with the command id every row agrees on", () => {
  // One message, one id: the rows the outbox kept carry the id they were sent
  // under, and the next attempt has to reuse it or the desktop runs the words a
  // second time.
  expect(batchCommandId([
    { commandId: "prompt_a" },
    { commandId: "prompt_a" },
  ])).toBe("prompt_a");
});

test("a batch the reader changed has no command id to reuse", () => {
  // Rows added or edited since are a different message: sending them under the old
  // id would either be swallowed as a duplicate or ride on words it did not carry.
  expect(batchCommandId([{ commandId: "prompt_a" }, { commandId: "prompt_a" }])).toBe("prompt_a");
  expect(batchCommandId([{ commandId: "prompt_a" }, {}])).toBeNull();
  expect(batchCommandId([{ commandId: "prompt_a" }, { commandId: "prompt_b" }])).toBeNull();
  expect(batchCommandId([{}])).toBeNull();
  expect(batchCommandId([])).toBeNull();
});
