import type { AgentMessage } from "./agentThreadTypes";
import { describe, expect, it } from "vitest";
import { isCurrentAgentEventTarget, liveBubblesToKeep } from "./useThreadMessages";

function bubble(id: string, runId: string): AgentMessage {
  return {
    id: `stream_${id}`,
    runId,
    role: "assistant",
    authorKey: "author.researchCopilot",
    content: "",
    status: "streaming",
    createdAt: new Date().toISOString(),
  };
}

describe("liveBubblesToKeep", () => {
  it("never carries a foreign thread's live bubble across a switch", () => {
    // The cross-talk bug: switching away from a streaming conversation left
    // its live bubble in `current`, and the merge grafted it onto the new
    // thread's history — the old run rendered inside the new conversation.
    const current = [bubble("run-a", "run-a")];
    const restored: AgentMessage[] = [
      {
        id: "u1",
        role: "user",
        authorKey: "author.you",
        content: "new question",
        createdAt: new Date().toISOString(),
      },
    ];
    expect(liveBubblesToKeep(current, restored, "run-b")).toEqual([]);
    expect(liveBubblesToKeep(current, restored, null)).toEqual([]);
  });

  it("keeps the loaded thread's own live bubble when the load didn't fold it", () => {
    const own = bubble("run-b", "run-b");
    expect(liveBubblesToKeep([own], [], "run-b")).toEqual([own]);
  });

  it("dedups a bubble the fresh projection already folded in", () => {
    const own = bubble("run-b", "run-b");
    expect(liveBubblesToKeep([own], [own], "run-b")).toEqual([]);
  });
});

describe("isCurrentAgentEventTarget", () => {
  it("rejects an old session event after AgentThread switches conversations", () => {
    // AgentThread stays mounted across a switch. The old listener can briefly
    // still be registered, but it must not write session A's user_message into
    // the messages state now rendering thread B.
    expect(isCurrentAgentEventTarget(
      "thread-b",
      "session-b",
      "thread-a",
      "session-a",
      "thread-a",
      "session-a",
    )).toBe(false);
  });

  it("accepts only the current thread and its one observer session", () => {
    expect(isCurrentAgentEventTarget(
      "thread-a",
      "session-a",
      "thread-a",
      "session-a",
      "thread-a",
      "session-a",
    )).toBe(true);
    expect(isCurrentAgentEventTarget(
      "thread-a",
      "session-a",
      "thread-a",
      "session-a",
      "thread-a",
      "session-b",
    )).toBe(false);
    expect(isCurrentAgentEventTarget(
      "thread-a",
      "session-a",
      "thread-a",
      "session-a",
      "thread-b",
      "session-a",
    )).toBe(false);
  });
});
