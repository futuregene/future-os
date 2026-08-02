import type { AgentMessage } from "./agentThreadTypes";
import { describe, expect, it } from "vitest";
import { computePageStart } from "./useMessagePaging";

function user(id: string): AgentMessage {
  return { id, role: "user", authorKey: "author.you", content: id, status: "complete", createdAt: "2026-01-01T00:00:00Z" };
}

function assistant(id: string): AgentMessage {
  return { id, role: "assistant", authorKey: "author.researchCopilot", content: id, status: "complete", createdAt: "2026-01-01T00:00:00Z" };
}

describe("computePageStart", () => {
  it("returns 0 for an empty list", () => {
    expect(computePageStart([], 10)).toBe(0);
  });

  it("returns 0 for a non-positive page size", () => {
    expect(computePageStart([user("u1")], 0)).toBe(0);
    expect(computePageStart([user("u1")], -3)).toBe(0);
  });

  it("renders everything when the thread has fewer turns than a page", () => {
    const messages = [user("u1"), assistant("a1"), user("u2"), assistant("a2")];
    expect(computePageStart(messages, 10)).toBe(0);
  });

  it("starts at the Nth user message from the end", () => {
    // 3 turns: u1→a1, u2→a2, u3→a3. A 2-turn page starts at u2 (index 2).
    const messages = [
      user("u1"),
      assistant("a1"),
      user("u2"),
      assistant("a2"),
      user("u3"),
      assistant("a3"),
    ];
    expect(computePageStart(messages, 2)).toBe(2);
  });

  it("starts at a user boundary even when the tail has orphan messages", () => {
    // u1→a1, u2→a2, then a trailing assistant with no user (in-flight reply).
    const messages = [
      user("u1"),
      assistant("a1"),
      user("u2"),
      assistant("a2"),
      assistant("orphan"),
    ];
    // A 1-turn page must start at u2, keeping u2→a2→orphan together.
    expect(computePageStart(messages, 1)).toBe(2);
  });

  it("does not count compaction dividers as user turns", () => {
    const divider = assistant("divider");
    // u1→a1, u2→a2, [divider], u3→a3.
    const messages = [
      user("u1"),
      assistant("a1"),
      user("u2"),
      assistant("a2"),
      divider,
      user("u3"),
      assistant("a3"),
    ];
    // A 2-turn page still starts at u2 — the divider sits inside the window.
    expect(computePageStart(messages, 2)).toBe(2);
  });

  it("clamps to 0 when the requested page exceeds the whole thread", () => {
    const messages = [user("u1"), assistant("a1")];
    expect(computePageStart(messages, 5)).toBe(0);
  });
});
