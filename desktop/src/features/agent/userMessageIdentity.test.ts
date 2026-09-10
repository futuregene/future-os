import { upsertUserMessage, userMessageFromEvent } from "@future-os/thread-projection";
import { expect, it } from "vitest";

it("places a late user event before its own assistant, not in another exchange", () => {
  const user = userMessageFromEvent({ entry_id: "u2", run_id: "r2", text: "same" })!;
  const old = { ...user, id: "m_u1", runId: "r1" };
  const reply = { ...user, id: "assistant2", role: "assistant" as const };
  expect(upsertUserMessage([old, reply], user).map(message => message.id)).toEqual(["m_u1", "m_u2", "assistant2"]);
});

it("matches an acknowledged optimistic message by run and preserves its UI id", () => {
  const user = userMessageFromEvent({ entry_id: "persisted", run_id: "r1", text: "same" })!;
  const optimistic = { ...user, id: "pending_user" };
  expect(upsertUserMessage([optimistic], user)).toEqual([optimistic]);
});

it("projects attachment-only events and rejects identity-less invalidations", () => {
  const user = userMessageFromEvent({ entry_id: "u1", run_id: "r1", text: "", attachments: [{ path: "/tmp/a", name: "a" }] })!;
  expect(user.attachments?.[0]?.path).toBe("/tmp/a");
  expect(userMessageFromEvent({ text: "same" })).toBeNull();
});
