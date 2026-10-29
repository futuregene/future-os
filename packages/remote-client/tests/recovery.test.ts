import { canRecoverMessage } from "../src/recovery";
import type { TimelineItem } from "../src/types";

function assistant(
  overrides: Partial<Extract<TimelineItem, { kind: "message" }>> = {},
): TimelineItem {
  return {
    id: "assistant-1",
    kind: "message",
    role: "assistant",
    text: "partial",
    runId: "run-1",
    failed: true,
    ...overrides,
  };
}

describe("recovery predicate", () => {
  test("allows only the latest reply", () => {
    expect(canRecoverMessage(assistant(), "assistant-1")).toBe(true);
    expect(canRecoverMessage(assistant(), "assistant-2")).toBe(false);
  });

  // The reason a reply stopped is not the reader's problem: every shape a run
  // can end in without finishing is resumable, including the one an app restart
  // leaves behind (a `cancelled` run the phone draws as stopped).
  test.each([
    ["a failed run", { failed: true }],
    ["a user stop", { failed: undefined, stopped: true }],
    ["a truncated stream", { failed: undefined, truncated: true }],
  ])("%s can be picked up from", (_label, overrides) => {
    expect(canRecoverMessage(assistant(overrides), "assistant-1")).toBe(true);
  });

  test("a reply that is still being written offers no recovery", () => {
    expect(canRecoverMessage(assistant({ streaming: true }), "assistant-1")).toBe(false);
  });

  test("suppresses an unidentified run", () => {
    expect(canRecoverMessage(assistant({ runId: undefined }), "assistant-1")).toBe(false);
  });

  test("does not recover a handled tool failure of a reply that finished", () => {
    expect(canRecoverMessage(assistant({ failed: undefined }), "assistant-1")).toBe(false);
  });
});
