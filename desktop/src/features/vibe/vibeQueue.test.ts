import type { MessageAttachment } from "@future-os/thread-projection";
import type { QueuedMap, QueuedPrompt } from "./vibeQueue";
import { describe, expect, it } from "vitest";
import { cancelPrompt, dueReturns, enqueuePrompt, isRunningStatus, takePrompt } from "./vibeQueue";

/**
 * The send queue's rules, as pure functions.
 *
 * Each case here is one of the constraints the queue was commissioned under: visible,
 * cancellable without losing the text, one per conversation, never sent by itself, and
 * kept across a failure. The "never sent by itself" half is asserted by absence — there
 * is no send function to call, and `dueReturns` produces *conversations to hand back*,
 * not messages to deliver.
 */
const prompt = (text: string, attachments: MessageAttachment[] = []): QueuedPrompt => ({ attachments, text });

function mapOf(entries: Array<[string, QueuedPrompt]>): QueuedMap {
  return new Map(entries);
}

describe("enqueuePrompt", () => {
  it("queues one prompt for a conversation", () => {
    const { queue, result } = enqueuePrompt(new Map(), "t1", prompt("first"));
    expect(result).toBe("queued");
    expect(queue.get("t1")?.text).toBe("first");
  });

  it("refuses a second one instead of replacing it", () => {
    // The requirement is explicit: replacing is how a sentence gets swallowed. The
    // caller shows "one is already queued" and leaves the new text in the box.
    const { queue, result } = enqueuePrompt(mapOf([["t1", prompt("first")]]), "t1", prompt("second"));
    expect(result).toBe("refused");
    expect(queue.get("t1")?.text).toBe("first");
  });

  it("keeps two conversations' queues apart", () => {
    const first = enqueuePrompt(new Map(), "t1", prompt("one")).queue;
    const second = enqueuePrompt(first, "t2", prompt("two")).queue;
    expect(second.get("t1")?.text).toBe("one");
    expect(second.get("t2")?.text).toBe("two");
  });

  it("never mutates the map it was given", () => {
    const original = new Map<string, QueuedPrompt>();
    enqueuePrompt(original, "t1", prompt("x"));
    expect(original.size).toBe(0);
  });
});

describe("takePrompt", () => {
  it("takes one out and leaves the rest", () => {
    const { prompt: taken, queue } = takePrompt(mapOf([["t1", prompt("one")], ["t2", prompt("two")]]), "t1");
    expect(taken?.text).toBe("one");
    expect([...queue.keys()]).toEqual(["t2"]);
  });

  it("returns null and an unchanged map for a conversation with nothing queued", () => {
    const { prompt: taken, queue } = takePrompt(mapOf([["t1", prompt("one")]]), "t2");
    expect(taken).toBeNull();
    expect([...queue.keys()]).toEqual(["t1"]);
  });
});

describe("cancelPrompt", () => {
  // Cancelling is "not now", not "never" — but it must not overwrite a message the reader
  // has started writing since. That is the whole reason this is a separate answer from a
  // boolean.
  it("hands the sentence back into an empty box", () => {
    const { prompt: taken, queue, result } = cancelPrompt(mapOf([["t1", prompt("one")]]), "t1", () => "");
    expect(result).toBe("cancelled");
    expect(taken?.text).toBe("one");
    expect(queue.has("t1")).toBe(false);
  });

  it("leaves the entry alone when the box already holds something", () => {
    const { queue, result } = cancelPrompt(mapOf([["t1", prompt("one")]]), "t1", () => "typing since");
    expect(result).toBe("busy");
    // Nothing is lost: the sentence stays queued, visible, and cancellable later.
    expect(queue.get("t1")?.text).toBe("one");
  });

  it("has nothing to cancel for a conversation with nothing held", () => {
    const { result } = cancelPrompt(new Map(), "t1", () => "");
    expect(result).toBe("none");
  });
});

describe("dueReturns", () => {
  const queue = mapOf([["t1", prompt("one")], ["t2", prompt("two")], ["t3", prompt("three")]]);

  it("hands back only what has stopped running", () => {
    // The middle of the three constraints: nothing is sent, a sentence that can now be
    // looked at goes back to its box.
    expect(dueReturns(queue, id => id === "t1", () => "")).toEqual(["t2", "t3"]);
  });

  it("keeps a sentence out of a box the reader has started typing in", () => {
    // Reading the two together must not fabricate a message: the box already holds
    // something, so the queued sentence waits rather than overwriting it.
    expect(dueReturns(queue, () => false, id => (id === "t2" ? "draft in progress" : "")))
      .toEqual(["t1", "t3"]);
  });

  it("hands nothing back while everything is busy", () => {
    expect(dueReturns(queue, () => true, () => "")).toEqual([]);
  });

  it("has nothing to do for an empty queue", () => {
    expect(dueReturns(new Map(), () => false, () => "")).toEqual([]);
  });
});

describe("isRunningStatus", () => {
  it("counts every in-flight state, and only those", () => {
    for (const status of ["queued", "running", "waiting_approval"])
      expect(isRunningStatus(status), status).toBe(true);
    // A failure is *not* running: that is what makes the held sentence come back after
    // one, rather than being re-sent as a second message.
    for (const status of ["completed", "failed", "cancelled", undefined])
      expect(isRunningStatus(status), String(status)).toBe(false);
  });
});
