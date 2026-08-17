import { canRecoverMessage } from "../recovery";
import type { TimelineItem } from "../types";

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

describe("desktop-parity recovery predicate", () => {
  test("allows only the latest failed run", () => {
    expect(canRecoverMessage(assistant(), "assistant-1")).toBe(true);
    expect(canRecoverMessage(assistant(), "assistant-2")).toBe(false);
  });

  test("suppresses user-stopped and unidentified runs", () => {
    expect(canRecoverMessage(assistant({ stopped: true }), "assistant-1")).toBe(false);
    expect(canRecoverMessage(assistant({ runId: undefined }), "assistant-1")).toBe(false);
  });

  test("does not recover a handled tool failure without a failed run", () => {
    expect(canRecoverMessage(assistant({ failed: undefined }), "assistant-1")).toBe(false);
  });
});
