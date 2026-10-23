import type { AgentMessage } from "@future-os/thread-projection";
import { describe, expect, it } from "vitest";
import { reconcileThreadHistory } from "./reconcileThreadHistory";

function message(id: string, overrides: Partial<AgentMessage> = {}): AgentMessage {
  return { id, role: "assistant", authorKey: "author.researchCopilot", content: id, createdAt: "2026-01-01T00:00:00Z", ...overrides };
}

describe("history reconciliation", () => {
  it("matches optimistic and canonical exchanges by run while preserving DOM ids", () => {
    const user = message("pending_user", { role: "user", runId: "r1" });
    const assistant = message("local_assistant", { runId: "r1" });
    const current = [user, assistant];
    const result = reconcileThreadHistory(current, [
      { ...user, id: "stored_user" },
      { ...assistant, id: "stored_assistant", content: "persisted" },
    ], current, false, true);
    expect(result.messages.map(item => item.id)).toEqual(["pending_user", "local_assistant"]);
    expect(result.messages[1]?.content).toBe("persisted");
  });

  it("does not overwrite live writes even during a settling refresh", () => {
    const old = message("pending", { runId: "r1", status: "streaming" });
    const live = { ...old, content: "newer streamed text" };
    const next = message("pending_user_next", { role: "user", runId: "r2" });
    const result = reconcileThreadHistory([live, next], [{ ...old, id: "canonical", status: "complete" }], [old], false, true);
    expect(result.messages).toEqual([live, next]);
  });

  it("keeps pages prepended while a tail request was running", () => {
    const oldest = message("old");
    const tail = message("tail");
    const result = reconcileThreadHistory([oldest, tail], [{ ...tail, content: "fresh" }], [tail], true, false);
    expect(result.keptOlder).toBe(true);
    expect(result.messages.map(item => item.content)).toEqual(["old", "fresh"]);
  });

  it("keeps the array and row references for unchanged history", () => {
    const current = [message("same", { attachments: [] })];
    expect(reconcileThreadHistory(current, [{ ...current[0]! }], current, true, false).messages).toBe(current);
  });
});

it("keeps a local-only exchange that never reached the agent at its own time", () => {
  // A send whose prompt never reached the agent (network/IPC failure) is
  // persisted nowhere, so its optimistic row stays local-only for good. It is
  // NOT the newest write: appending it on every reload put an older exchange
  // below the newest reply, and the transcript read newest-first.
  const older = message("older", { createdAt: "2026-01-01T00:00:00Z" });
  const staleUser = message("pending_user_lost", {
    role: "user",
    runId: "run-lost",
    content: "继续上一个任务。",
    createdAt: "2026-01-01T00:05:00Z",
  });
  const staleFailure = message("pending_lost", {
    runId: "run-lost",
    status: "failed",
    createdAt: "2026-01-01T00:05:01Z",
  });
  const newerUser = message("pending_user_ok", { role: "user", runId: "run-ok", createdAt: "2026-01-01T00:10:00Z" });
  const newerReply = message("m_reply", { runId: "run-ok", createdAt: "2026-01-01T00:11:00Z" });
  const current = [older, staleUser, staleFailure, newerUser, newerReply];
  const result = reconcileThreadHistory(
    current,
    [older, newerUser, newerReply],
    current,
    false,
    true,
  );
  expect(result.messages.map(item => item.id)).toEqual([
    "older",
    "pending_user_lost",
    "pending_lost",
    "pending_user_ok",
    "m_reply",
  ]);
});

it("keeps a newer local-only row at the end", () => {
  const older = message("older", { createdAt: "2026-01-01T00:00:00Z" });
  const tail = message("tail", { createdAt: "2026-01-01T00:10:00Z" });
  const streaming = message("pending", {
    runId: "run-live",
    status: "streaming",
    createdAt: "2026-01-01T00:20:00Z",
  });
  const result = reconcileThreadHistory([older, tail, streaming], [older, tail], [older, tail], false, false);
  expect(result.messages.map(item => item.id)).toEqual(["older", "tail", "pending"]);
});

it("live mobile user arriving during history refresh keeps canonical position", () => {
  const previous = message("previous");
  const liveUser = message("user_123", { role: "user", runId: "remote-run", content: "from phone", status: "complete" });
  const storedUser = message("m_persisted", { role: "user", runId: "remote-run", content: "from phone", status: "complete" });
  const reply = message("m_reply", { runId: "remote-run", content: "reply", status: "complete" });
  const result = reconcileThreadHistory([previous, liveUser], [previous, storedUser, reply], [previous], false, true);
  expect(result.messages.map(m => m.content)).toEqual(["previous", "from phone", "reply"]);
});
