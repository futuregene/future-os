import { isTransientQueueRefusal } from "../queueDelivery";

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
