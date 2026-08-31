import type { SessionEntry } from "@future-os/thread-projection";
import { entriesToMessages, entriesToTurns } from "@future-os/thread-projection";
import { describe, expect, it } from "vitest";

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

  it("projects a finalized assistant entry's canonical run id", () => {
    const messages = entriesToMessages([
      { id: "u1", role: "user", content: "hi", meta: { run_id: "run-1" } },
      { id: "a1", role: "assistant", content: "hello", meta: { run_id: "run-1" } },
    ]);

    // User metadata identifies the accepted prompt but must not suppress the
    // live assistant bubble. Only a finalized assistant entry carries the run id.
    expect(messages[0]?.runId).toBeUndefined();
    expect(messages[1]?.runId).toBe("run-1");
  });

  it("produces only the user message for an exchange with no assistant entry", () => {
    // A streaming or aborted exchange: the agent recorded the user prompt but
    // has not yet written (or will never write) an assistant reply.  An empty
    // "completed" bubble would steal the runId in applyRunMetadata and block
    // upsertStreamingPreview from attaching the live preview, so the exchange
    // produces only the user message; the streaming bubble (or aborted-run
    // recovery) fills in the assistant side at render time.
    const userTs = "2026-07-01T10:00:00+08:00";
    const entries: SessionEntry[] = [
      { id: "u1", role: "user", content: "写一首长诗", timestamp: userTs },
    ];

    const messages = entriesToMessages(entries);

    expect(messages).toHaveLength(1);
    expect(messages[0]?.role).toBe("user");
    expect(messages[0]?.content).toBe("写一首长诗");
    expect(messages[0]?.createdAt).toBe(userTs);
  });

  it("places a reply-less failure on its canonical user turn", () => {
    const messages = entriesToMessages([
      {
        id: "u1",
        role: "user",
        content: "first",
        meta: { run_id: "run-failed" },
        run_status: "failed",
        run_error: "authentication failed",
        run_duration_ms: 120,
      },
      {
        id: "u2",
        role: "user",
        content: "second",
        meta: { run_id: "run-ok" },
        run_status: "completed",
      },
      {
        id: "a2",
        role: "assistant",
        content: "done",
        meta: { run_id: "run-ok" },
        run_status: "completed",
      },
    ]);

    expect(messages.map(message => message.id)).toEqual([
      "m_u1",
      "failed_run-failed",
      "m_u2",
      "m_a2",
    ]);
    expect(messages[1]).toMatchObject({
      role: "assistant",
      runId: "run-failed",
      status: "failed",
      runError: "authentication failed",
      durationMs: 120,
    });
    expect(messages[3]?.runId).toBe("run-ok");
  });

  it("fails closed when user and assistant canonical run ids conflict", () => {
    const nodes = entriesToTurns([
      {
        id: "u1",
        role: "user",
        content: "hello",
        meta: { run_id: "run-user" },
        run_status: "failed",
        run_error: "must not attach",
      },
      {
        id: "a1",
        role: "assistant",
        content: "reply",
        meta: { run_id: "run-assistant" },
      },
    ]);

    expect(nodes).toHaveLength(1);
    const turn = nodes[0]?.kind === "turn" ? nodes[0].turn : undefined;
    expect(turn?.identitySource).toBe("conflict");
    expect(turn?.runId).toBeUndefined();
    expect(turn?.outcome).toBeUndefined();
    expect(turn?.assistant?.runId).toBeUndefined();
    expect(turn?.assistant?.status).toBe("complete");
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
          { id: "call_00_test", function: { name: "write", arguments: JSON.stringify({ path: "poem.md", content: "..." }) } },
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
        tool_calls: [{ id: "call_00_test", function: { name: "write", arguments: JSON.stringify({ path: "poem.md" }) } }],
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
        tool_calls: [{ id: "call_00_test", function: { name: "shell", arguments: JSON.stringify({ command: "futre --version" }) } }],
      },
      { id: "t1", role: "tool", name: "shell", content: "futre: command not found\n[exit: 127]", timestamp: "2026-07-01T10:00:04+08:00" },
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
        tool_calls: [{ id: "call_00_test", function: { name: "shell", arguments: JSON.stringify({ command: "grep foo file.txt" }) } }],
      },
      { id: "t1", role: "tool", name: "shell", content: "[exit: 1]", timestamp: "2026-07-01T10:00:04+08:00" },
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
        tool_calls: [{ id: "call_00_test", function: { name: "read", arguments: JSON.stringify({ path: "config.toml" }) } }],
      },
    ];

    const kinds = entriesToMessages(entries)[1]?.segments?.map(segment => segment.kind);
    expect(kinds).toEqual(["text", "activity"]);
  });

  it("collapses a burst of same-kind tools into one row with a count", () => {
    const editCall = (path: string) => ({ id: "call_00_test", function: { name: "edit", arguments: JSON.stringify({ path }) } });
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
    // A divider message (compaction segment) + the real user message + its reply.
    const divider = messages.find(message => message.segments?.some(s => s.kind === "compaction"));
    expect(divider).toBeDefined();
    expect(divider?.role).toBe("assistant");
    // The compaction text must not appear as a user bubble.
    expect(messages.some(message => message.role === "user" && message.content.startsWith("[Context compaction:"))).toBe(false);
  });

  it("renders a durable v2 checkpoint with its stable checkpoint id", () => {
    const messages = entriesToMessages([{
      id: "checkpoint-entry",
      entry_type: "compaction",
      role: "system",
      content: "",
      timestamp: "2026-07-01T10:00:00+08:00",
      checkpoint: {
        schema_version: 2,
        checkpoint_id: "cp-1",
        cutoff_entry_id: "a1",
        tokens_before: 190_000,
        tokens_after: 20_000,
        trigger: "manual",
      },
    }]);

    expect(messages).toEqual([expect.objectContaining({
      id: "m_cp-1",
      role: "assistant",
      segments: [{ id: "seg_cp-1_compaction", kind: "compaction", tokensBefore: 190_000, trigger: "manual" }],
    })]);
  });

  it("groups entries positionally — a user entry opens a new exchange", () => {
    const entries: SessionEntry[] = [
      { id: "u1", role: "user", content: "first question" },
      { id: "u2", role: "user", content: "follow-up" },
      { id: "a1", role: "assistant", content: "answer" },
    ];

    const messages = entriesToMessages(entries);

    // Positional: the assistant follows the most recent user message.
    expect(messages.map(m => [m.role, m.content])).toEqual([
      ["user", "first question"],
      ["user", "follow-up"],
      ["assistant", "answer"],
    ]);
  });

  it("derives stable ids from entry ids across re-projections", () => {
    // A run settle / thread switch re-projects the same JSONL; if message and
    // segment ids change between projections, React remounts the whole window.
    const entries: SessionEntry[] = [
      { id: "c1", role: "user", content: "[Context compaction: summarized]", timestamp: "2026-07-01T09:00:00+08:00" },
      { id: "u1", role: "user", content: "hi", timestamp: "2026-07-01T10:00:00+08:00" },
      {
        id: "a1",
        role: "assistant",
        content: "",
        thinking: "hm",
        timestamp: "2026-07-01T10:00:03+08:00",
        tool_calls: [
          { id: "call_00_read", function: { name: "read", arguments: JSON.stringify({ path: "a.ts" }) } },
        ],
      },
      { id: "t1", role: "tool", name: "read", content: "ok", timestamp: "2026-07-01T10:00:04+08:00" },
      { id: "a2", role: "assistant", content: "done", timestamp: "2026-07-01T10:00:05+08:00" },
    ];

    const first = entriesToMessages(entries);
    const second = entriesToMessages(entries);

    expect(second.map(m => m.id)).toEqual(first.map(m => m.id));
    expect(second.map(m => m.segments?.map(s => s.id))).toEqual(first.map(m => m.segments?.map(s => s.id)));
    // Ids are derived from the entry ids, not timestamps/sequences.
    expect(first.map(m => m.id)).toEqual(["m_c1", "m_u1", "m_a2"]);
  });
});

describe("entriesToMessages edge cases", () => {
  it("rebuilds attachment chips from user entry meta", () => {
    const messages = entriesToMessages([{
      id: "u1",
      role: "user",
      content: "see this",
      meta: { attachments: [{ path: "/a.png", name: "a.png", kind: "image", thumbnail: "/t.png" }, { path: "/b.pdf", name: "b.pdf" }] },
    }]);
    expect(messages[0]?.attachments).toEqual([
      { path: "/a.png", name: "a.png", kind: "image", thumbnail: "/t.png" },
      { path: "/b.pdf", name: "b.pdf", kind: "file", thumbnail: null },
    ]);
  });

  it("treats an empty tool result as completed and an Error: result as failed", () => {
    const entries: SessionEntry[] = [
      { id: "u1", role: "user", content: "run it" },
      {
        id: "a1",
        role: "assistant",
        content: "",
        tool_calls: [
          { id: "c1", function: { name: "shell", arguments: "{}" } },
          { id: "c2", function: { name: "shell", arguments: "{}" } },
        ],
      },
      { id: "t1", role: "tool", content: "" },
      { id: "t2", role: "tool", content: "Error: boom" },
    ];
    const messages = entriesToMessages(entries);
    const segments = messages[1]?.segments ?? [];
    const activities = segments.flatMap(s => (s.kind === "activity" ? [s.item] : []));
    expect(activities.map(a => a.status)).toEqual(["completed", "failed"]);
    // No text segments: content falls back to the (empty) joined text.
    expect(messages[1]?.content).toBe("");
  });

  it("synthesizes an id for tool calls without one", () => {
    const messages = entriesToMessages([
      { id: "u1", role: "user", content: "go" },
      {
        id: "a1",
        role: "assistant",
        content: "",
        tool_calls: [{ id: "", function: { name: "read", arguments: "{}" } }],
      },
    ]);
    const segments = messages[1]?.segments ?? [];
    const activity = segments.find(s => s.kind === "activity");
    expect(activity && "item" in activity && activity.item.id).toMatch(/^ep_/);
  });

  it("drops an assistant-only exchange (no user message) on flush", () => {
    const messages = entriesToMessages([
      { id: "a1", role: "assistant", content: "orphan reply" },
    ]);
    // The orphan exchange opens an accumulator but has no user message, so
    // nothing is emitted for it.
    expect(messages.some(m => m.content === "orphan reply")).toBe(false);
  });

  it("flushes an open exchange when a compaction divider arrives", () => {
    const messages = entriesToMessages([
      { id: "u1", role: "user", content: "before" },
      { id: "a1", role: "assistant", content: "reply" },
      { id: "c1", role: "user", content: "[Context compaction: summary]" },
      { id: "u2", role: "user", content: "after" },
    ]);
    expect(messages.map(m => m.content)).toContain("before");
    expect(messages.some(m => m.segments?.some(s => s.kind === "compaction"))).toBe(true);
    expect(messages.map(m => m.content)).toContain("after");
  });
});
