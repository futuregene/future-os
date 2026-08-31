import type { AgentMessage } from "@future-os/thread-projection";
import { beforeEach, describe, expect, it } from "vitest";
import {
  clearThreadMessageSnapshots,
  getThreadMessageSnapshot,
  setThreadMessageSnapshot,
} from "./threadMessageCache";

function messages(id: string): AgentMessage[] {
  return [{ id, role: "user", content: id }] as AgentMessage[];
}

beforeEach(() => {
  clearThreadMessageSnapshots();
});

describe("threadMessageCache", () => {
  it("keeps a conversation snapshot, including an empty conversation", () => {
    const snapshot = messages("m1");
    setThreadMessageSnapshot("thread-1", "session-1", snapshot);
    setThreadMessageSnapshot("thread-empty", null, []);

    expect(getThreadMessageSnapshot("thread-1", "session-1")).toBe(snapshot);
    expect(getThreadMessageSnapshot("thread-empty", null)).toEqual([]);
    expect(getThreadMessageSnapshot("missing", null)).toBeNull();
  });

  it("does not reuse a snapshot after the Agent session changes", () => {
    setThreadMessageSnapshot("thread-1", "session-old", messages("old"));

    expect(getThreadMessageSnapshot("thread-1", "session-new")).toBeNull();
  });

  it("bounds snapshots and refreshes recency on reads", () => {
    for (let index = 0; index < 12; index++)
      setThreadMessageSnapshot(`thread-${index}`, null, messages(`m${index}`));

    expect(getThreadMessageSnapshot("thread-0", null)?.[0]?.id).toBe("m0");
    setThreadMessageSnapshot("thread-12", null, messages("m12"));

    expect(getThreadMessageSnapshot("thread-0", null)?.[0]?.id).toBe("m0");
    expect(getThreadMessageSnapshot("thread-1", null)).toBeNull();
  });
});
