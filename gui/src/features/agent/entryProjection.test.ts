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

  it("falls back to the user timestamp for a turn with no assistant entry", () => {
    // An aborted turn: the agent recorded the user prompt but no reply. The
    // assistant bubble must not be re-stamped "now" on every reload.
    const userTs = "2026-07-01T10:00:00+08:00";
    const entries: SessionEntry[] = [
      { id: "u1", role: "user", content: "写一首长诗", timestamp: userTs },
    ];

    const messages = entriesToMessages(entries);

    expect(messages).toHaveLength(2);
    expect(messages[1]?.role).toBe("assistant");
    expect(messages[1]?.createdAt).toBe(userTs);
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

  it("sets a write tool activity's target to the file path from its args", () => {
    const entries: SessionEntry[] = [
      { id: "u1", role: "user", content: "write a file", timestamp: "2026-07-01T10:00:00+08:00" },
      {
        id: "a1",
        role: "assistant",
        content: "done",
        timestamp: "2026-07-01T10:00:05+08:00",
        tool_calls: [
          { function: { name: "write", arguments: JSON.stringify({ path: "poem.md", content: "..." }) } },
        ],
      },
    ];

    const assistant = entriesToMessages(entries)[1];
    const activity = assistant?.segments?.find(segment => segment.kind === "activity");
    expect(activity?.kind === "activity" ? activity.item.target : undefined).toBe("poem.md");
  });

  it("does not duplicate a tool activity for the tool result entry", () => {
    const entries: SessionEntry[] = [
      { id: "u1", role: "user", content: "write a file", timestamp: "2026-07-01T10:00:00+08:00" },
      {
        id: "a1",
        role: "assistant",
        content: "",
        timestamp: "2026-07-01T10:00:03+08:00",
        tool_calls: [{ function: { name: "write", arguments: JSON.stringify({ path: "poem.md" }) } }],
      },
      // The agent's tool result entry for the same call — must not add a second row.
      { id: "t1", role: "tool", name: "write", content: "Written to poem.md", timestamp: "2026-07-01T10:00:04+08:00" },
      { id: "a2", role: "assistant", content: "done", timestamp: "2026-07-01T10:00:05+08:00" },
    ];

    const assistant = entriesToMessages(entries)[1];
    const activities = assistant?.segments?.filter(segment => segment.kind === "activity") ?? [];
    expect(activities).toHaveLength(1);
  });

  it("marks a tool activity failed when its result reports an error", () => {
    const entries: SessionEntry[] = [
      { id: "u1", role: "user", content: "run it", timestamp: "2026-07-01T10:00:00+08:00" },
      {
        id: "a1",
        role: "assistant",
        content: "",
        timestamp: "2026-07-01T10:00:03+08:00",
        tool_calls: [{ function: { name: "bash", arguments: JSON.stringify({ command: "futre --version" }) } }],
      },
      { id: "t1", role: "tool", name: "bash", content: "[exit code: 127]\nfutre: command not found", timestamp: "2026-07-01T10:00:04+08:00" },
      { id: "a2", role: "assistant", content: "that failed", timestamp: "2026-07-01T10:00:05+08:00" },
    ];

    const assistant = entriesToMessages(entries)[1];
    const activity = assistant?.segments?.find(segment => segment.kind === "activity");
    expect(activity?.kind === "activity" ? activity.item.status : undefined).toBe("failed");
  });

  it("keeps a bare grep exit-1 as completed (soft-fail exemption)", () => {
    const entries: SessionEntry[] = [
      { id: "u1", role: "user", content: "grep", timestamp: "2026-07-01T10:00:00+08:00" },
      {
        id: "a1",
        role: "assistant",
        content: "",
        timestamp: "2026-07-01T10:00:03+08:00",
        tool_calls: [{ function: { name: "bash", arguments: JSON.stringify({ command: "grep foo file.txt" }) } }],
      },
      { id: "t1", role: "tool", name: "bash", content: "[exit code: 1]\n", timestamp: "2026-07-01T10:00:04+08:00" },
    ];

    const assistant = entriesToMessages(entries)[1];
    const activity = assistant?.segments?.find(segment => segment.kind === "activity");
    expect(activity?.kind === "activity" ? activity.item.status : undefined).toBe("completed");
  });

  it("orders preamble text before the tool activity it introduces", () => {
    const entries: SessionEntry[] = [
      { id: "u1", role: "user", content: "check config", timestamp: "2026-07-01T10:00:00+08:00" },
      {
        id: "a1",
        role: "assistant",
        content: "Let me check the config.",
        timestamp: "2026-07-01T10:00:03+08:00",
        tool_calls: [{ function: { name: "read", arguments: JSON.stringify({ path: "config.toml" }) } }],
      },
    ];

    const kinds = entriesToMessages(entries)[1]?.segments?.map(segment => segment.kind);
    expect(kinds).toEqual(["text", "activity"]);
  });

  it("collapses a burst of same-kind tools into one row with a count", () => {
    const editCall = (path: string) => ({ function: { name: "edit", arguments: JSON.stringify({ path }) } });
    const entries: SessionEntry[] = [
      { id: "u1", role: "user", content: "edit files", timestamp: "2026-07-01T10:00:00+08:00" },
      {
        id: "a1",
        role: "assistant",
        content: "",
        timestamp: "2026-07-01T10:00:03+08:00",
        tool_calls: [editCall("a.ts"), editCall("b.ts"), editCall("c.ts")],
      },
    ];

    const activities = entriesToMessages(entries)[1]?.segments?.filter(s => s.kind === "activity") ?? [];
    expect(activities).toHaveLength(1);
    const item = activities[0]?.kind === "activity" ? activities[0].item : undefined;
    expect(item?.count).toBe(3);
    expect(item?.children).toHaveLength(3);
  });

  it("renders the compaction summary as a divider, not a user bubble", () => {
    const entries: SessionEntry[] = [
      { id: "c1", role: "user", content: "[Context compaction: Previous conversation summarized. Files read: a.ts. Modified: .]", timestamp: "2026-07-01T10:00:00+08:00" },
      { id: "u1", role: "user", content: "carry on", timestamp: "2026-07-01T10:01:00+08:00" },
      { id: "a1", role: "assistant", content: "ok", timestamp: "2026-07-01T10:01:02+08:00" },
    ];

    const messages = entriesToMessages(entries);
    // A divider message (compaction segment) + the real user turn + its reply.
    const divider = messages.find(message => message.segments?.some(s => s.kind === "compaction"));
    expect(divider).toBeDefined();
    expect(divider?.role).toBe("assistant");
    // The compaction text must not appear as a user bubble.
    expect(messages.some(message => message.role === "user" && message.content.startsWith("[Context compaction:"))).toBe(false);
  });
});
