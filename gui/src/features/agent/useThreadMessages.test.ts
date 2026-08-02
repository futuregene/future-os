import { describe, expect, it } from "vitest";
import { isCurrentAgentEventTarget } from "./useThreadMessages";

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
