import type { SessionEntry } from "./entryProjection";
import { describe, expect, it } from "vitest";
import { entriesToMessages } from "./entryProjection";

describe("entriesToMessages", () => {
  it("carries per-entry timestamps onto user and assistant messages", () => {
    const userTs = "2026-07-01T10:00:00+08:00";
    const asstTs = "2026-07-01T10:00:07+08:00";
    const entries: SessionEntry[] = [
      { id: "u1", role: "user", content: "hi", timestamp: userTs },
      { id: "a1", role: "assistant", content: "hello", timestamp: asstTs },
    ];

    const messages = entriesToMessages(entries);

    expect(messages).toHaveLength(2);
    expect(messages[0]?.createdAt).toBe(userTs);
    expect(messages[1]?.createdAt).toBe(asstTs);
  });

  it("projects output tokens and duration from the final assistant entry", () => {
    const entries: SessionEntry[] = [
      { id: "u1", role: "user", content: "hi", timestamp: "2026-07-01T10:00:00+08:00" },
      {
        id: "a1",
        role: "assistant",
        content: "hello",
        timestamp: "2026-07-01T10:00:07+08:00",
        output_tokens: 42,
        duration_ms: 7000,
      },
    ];

    const messages = entriesToMessages(entries);

    expect(messages[1]?.outputTokens).toBe(42);
    expect(messages[1]?.durationMs).toBe(7000);
  });

  it("leaves usage undefined when the agent reported none (no footer shown)", () => {
    const entries: SessionEntry[] = [
      { id: "u1", role: "user", content: "hi", timestamp: "2026-07-01T10:00:00+08:00" },
      { id: "a1", role: "assistant", content: "hello", timestamp: "2026-07-01T10:00:01+08:00" },
    ];

    const messages = entriesToMessages(entries);

    expect(messages[1]?.outputTokens).toBeUndefined();
    expect(messages[1]?.durationMs).toBeUndefined();
  });
});
