import { describe, expect, it } from "vitest";
import { attentionOf, nextAttention } from "./attention";

describe("attentionOf", () => {
  it("puts an approval first, from the count or from the run's own status", () => {
    // The count is the rail's view; the run status is what the run reports. Either
    // one is enough: a blocking decision must not be missed because one of the two
    // sources is a moment behind.
    expect(attentionOf({ approvals: 1, runStatus: "running", unread: false })).toBe("approval");
    expect(attentionOf({ approvals: 0, runStatus: "waiting_approval", unread: false })).toBe("approval");
    // And it outranks a failure: a failed run with a pending approval cannot move
    // until the approval is answered.
    expect(attentionOf({ approvals: 2, runStatus: "failed", unread: true })).toBe("approval");
  });

  it("reports a failed run", () => {
    expect(attentionOf({ approvals: 0, runStatus: "failed", unread: false })).toBe("failed");
  });

  it("reports a run that finished while the reader was elsewhere", () => {
    expect(attentionOf({ approvals: 0, runStatus: "completed", unread: true })).toBe("finished");
    // A finished run the reader has seen is not attention: it is just a finished run.
    expect(attentionOf({ approvals: 0, runStatus: "completed", unread: false })).toBeNull();
  });

  it("leaves a busy or idle conversation alone", () => {
    // The point of the queue: "running" is not "needs you". A queue that included
    // it would be every cell of the panel.
    for (const runStatus of ["queued", "running", "cancelled", null, undefined])
      expect(attentionOf({ approvals: 0, runStatus, unread: false }), String(runStatus)).toBeNull();
    // And an unread mark on something that is not a finished run does not count.
    expect(attentionOf({ approvals: 0, runStatus: "running", unread: true })).toBeNull();
  });
});

describe("nextAttention", () => {
  const keys = ["local:a", "local:b", "local:c", "local:d"];
  const reasons = new Map([
    ["local:b", "failed" as const],
    ["local:d", "approval" as const],
  ]);

  it("finds the next one clockwise from where the reader is", () => {
    expect(nextAttention(keys, reasons, "local:a")).toBe("local:b");
    expect(nextAttention(keys, reasons, "local:b")).toBe("local:d");
  });

  it("wraps back to the first", () => {
    expect(nextAttention(keys, reasons, "local:d")).toBe("local:b");
    expect(nextAttention(keys, reasons, "local:c")).toBe("local:d");
  });

  it("starts at the first when nothing is selected", () => {
    expect(nextAttention(keys, reasons, null)).toBe("local:b");
  });

  it("returns null when nothing needs a human", () => {
    expect(nextAttention(keys, new Map(), "local:a")).toBeNull();
    expect(nextAttention([], new Map(), null)).toBeNull();
  });

  it("stays inside the panel's order when a key is unknown", () => {
    // A focused conversation unpinned a moment ago: start the walk from the top
    // rather than guessing a position.
    expect(nextAttention(keys, reasons, "local:gone")).toBe("local:b");
  });
});
