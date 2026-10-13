import type { AgentMessage } from "@future-os/thread-projection";
import { describe, expect, it } from "vitest";
import { buildQuestionTurns, findQuestionTurns, targetRowForQuestion } from "./questionNav";

function user(id: string): AgentMessage {
  return {
    id,
    role: "user",
    content: `prompt ${id}`,
    status: "complete",
    createdAt: "2026-01-01T00:00:00Z",
  } as AgentMessage;
}

function assistant(id: string): AgentMessage {
  return {
    id,
    role: "assistant",
    content: `answer ${id}`,
    status: "complete",
    createdAt: "2026-01-01T00:00:00Z",
  } as AgentMessage;
}

function divider(id: string): AgentMessage {
  return {
    id,
    role: "assistant",
    authorKey: "author.researchCopilot",
    content: "",
    status: "complete",
    isCompaction: true,
    createdAt: "2026-01-01T00:00:00Z",
  } as AgentMessage;
}

describe("buildQuestionTurns", () => {
  it("returns empty for an empty list", () => {
    expect(buildQuestionTurns([])).toEqual([]);
  });

  it("only counts user messages as questions", () => {
    const messages = [user("u1"), assistant("a1"), user("u2")];
    const turns = buildQuestionTurns(messages);
    expect(turns).toHaveLength(2);
    expect(turns[0]!.question).toBe(0);
    expect(turns[1]!.question).toBe(2);
  });

  it("ignores compaction dividers and tool rows", () => {
    const messages = [user("u1"), assistant("a1"), divider("d1"), user("u2"), assistant("a2")];
    const turns = buildQuestionTurns(messages);
    expect(turns.map(t => t.question)).toEqual([0, 3]);
  });
});

describe("findQuestionTurns", () => {
  const turns = [
    { question: 0, messageId: "u1" },
    { question: 2, messageId: "u2" },
    { question: 5, messageId: "u3" },
  ];

  it("returns nulls for empty turns", () => {
    expect(findQuestionTurns([], 0)).toEqual({
      current: null,
      previous: null,
      next: null,
    });
  });

  it("returns nulls for null top index", () => {
    expect(findQuestionTurns(turns, null)).toEqual({
      current: null,
      previous: null,
      next: null,
    });
  });

  it("identifies current, previous and next questions", () => {
    expect(findQuestionTurns(turns, 3)).toEqual({
      current: 1,
      previous: 0,
      next: 2,
    });
  });

  it("returns no previous at the first question", () => {
    expect(findQuestionTurns(turns, 1)).toEqual({
      current: 0,
      previous: null,
      next: 1,
    });
  });

  it("returns no next at the last question", () => {
    expect(findQuestionTurns(turns, 5)).toEqual({
      current: 2,
      previous: 1,
      next: null,
    });
  });

  it("treats a viewport above every question as having the first question below it", () => {
    expect(findQuestionTurns(turns, -1)).toEqual({
      current: null,
      previous: null,
      next: 0,
    });
  });
});

describe("targetRowForQuestion", () => {
  const turns = [
    { question: 0, messageId: "u1" },
    { question: 2, messageId: "u2" },
  ];

  it("returns the question index when it fits in the list", () => {
    expect(targetRowForQuestion(turns, 0, 4)).toBe(0);
    expect(targetRowForQuestion(turns, 1, 4)).toBe(2);
  });

  it("clamps to the last message index", () => {
    expect(targetRowForQuestion(turns, 1, 1)).toBe(1);
  });
});
