import { describe, expect, it } from "vitest";
import { upsertUserMessage, userMessageFromEvent } from "./userMessage";
import type { AgentMessage } from "./model";

function user(id: string, runId?: string): AgentMessage {
  return { authorKey: "author.you", content: id, createdAt: "2026-01-01T00:00:00.000Z", id, role: "user", runId };
}

function assistant(id: string, runId?: string): AgentMessage {
  return { authorKey: "author.researchCopilot", content: id, createdAt: "2026-01-01T00:00:00.000Z", id, role: "assistant", runId };
}

describe("userMessageFromEvent", () => {
  it.each<[Record<string, unknown>]>([
    [{}],
    [{ entry_id: "e1" }],
    [{ run_id: "r1" }],
    [{ entry_id: "", run_id: "r1" }],
    [{ entry_id: "e1", run_id: "" }],
    [{ entry_id: 1, run_id: "r1" }],
  ])("rejects %j", (payload) => {
    expect(userMessageFromEvent(payload)).toBeNull();
  });

  it("builds a user message carrying the persisted identity", () => {
    const message = userMessageFromEvent({
      attachments: [{ kind: "image", name: "shot.png", path: "/tmp/shot.png", thumbnail: "/tmp/t.png" }],
      created_at_ms: 1_700_000_000_000,
      entry_id: "e1",
      run_id: "r1",
      text: "hello",
    });
    expect(message).toMatchObject({
      attachments: [{ kind: "image", name: "shot.png", path: "/tmp/shot.png", thumbnail: "/tmp/t.png" }],
      content: "hello",
      createdAt: new Date(1_700_000_000_000).toISOString(),
      id: "m_e1",
      role: "user",
      runId: "r1",
      sourceEntryId: "e1",
    });
  });

  it("falls back for a missing timestamp, non-string text and non-array attachments", () => {
    const message = userMessageFromEvent({
      attachments: "nope",
      created_at_ms: "yesterday",
      entry_id: "e2",
      run_id: "r2",
      text: 42,
    });
    expect(message).toMatchObject({ content: "", id: "m_e2", role: "user" });
    expect(message?.attachments).toBeUndefined();
    expect(Number.isNaN(Date.parse(message!.createdAt))).toBe(false);
  });
});

describe("upsertUserMessage", () => {
  it("replaces the message with the same id, keeping the original id", () => {
    const before = [user("m_1"), assistant("m_2")];
    const after = upsertUserMessage(before, { ...user("m_1"), content: "edited" });
    expect(after).toHaveLength(2);
    expect(after[0]).toMatchObject({ content: "edited", id: "m_1" });
    expect(after[1]).toBe(before[1]);
  });

  it("matches an existing user message by shared run id", () => {
    const before = [user("m_1", "r1")];
    const after = upsertUserMessage(before, user("m_optimistic", "r1"));
    expect(after).toHaveLength(1);
    expect(after[0]).toMatchObject({ id: "m_1" });
  });

  it("inserts a new user message before the matching assistant turn", () => {
    const before = [assistant("a1", "r1")];
    const after = upsertUserMessage(before, user("u1", "r1"));
    expect(after.map((message) => message.id)).toEqual(["u1", "a1"]);
  });

  it("appends a new user message when no turn matches", () => {
    const before = [assistant("a1", "r1")];
    expect(upsertUserMessage(before, user("u2", "r2")).map((message) => message.id)).toEqual(["a1", "u2"]);
    expect(upsertUserMessage(before, user("u3")).map((message) => message.id)).toEqual(["a1", "u3"]);
    expect(upsertUserMessage([], user("u1")).map((message) => message.id)).toEqual(["u1"]);
  });

  it("does not merge with a non-user message that shares the id", () => {
    const before = [assistant("same")];
    expect(upsertUserMessage(before, user("same")).map((message) => message.id)).toEqual(["same", "same"]);
  });
});
