import type { AgentMessage } from "./model";
import { describe, expect, it } from "vitest";
import { collapseReplyDuplicates } from "./replyDuplicates";

function user(id: string, runId?: string): AgentMessage {
  return { authorKey: "author.you", content: id, createdAt: "2026-01-01T00:00:00.000Z", id, role: "user", runId };
}

function assistant(
  id: string,
  options: { runId?: string; inFlightRunId?: string; status?: "complete" | "streaming" } = {},
): AgentMessage {
  return {
    authorKey: "author.researchCopilot",
    content: id,
    createdAt: "2026-01-01T00:00:00.000Z",
    id,
    role: "assistant",
    ...options,
  };
}

const ids = (messages: AgentMessage[]) => messages.map(message => message.id);

describe("collapseReplyDuplicates", () => {
  it("leaves an ordinary conversation alone", () => {
    const messages = [user("u1", "r1"), assistant("a1", { runId: "r1" }), user("u2", "r2")];
    expect(ids(collapseReplyDuplicates(messages))).toEqual(["u1", "a1", "u2"]);
  });

  /**
   * The reported shape: one answer, drawn three times.
   *
   * A reload during the run projects the agent's partial snapshot as a
   * settled-looking reply carrying no run, and the live bubble for the same run
   * is a second copy; the next reload adds a third. The live bubble owns the
   * reply, so the snapshots go.
   */
  it("keeps one copy of a reply that was drawn three times", () => {
    const messages = [
      assistant("snapshot-1"),
      assistant("snapshot-2", { status: "complete" }),
      assistant("live", { runId: "r1" }),
    ];
    expect(ids(collapseReplyDuplicates(messages))).toEqual(["live"]);
  });

  /** Two bubbles naming the same run are the same reply, however they got there. */
  it("keeps the newest bubble of a run", () => {
    const messages = [assistant("first", { runId: "r1" }), assistant("second", { runId: "r1" })];
    expect(ids(collapseReplyDuplicates(messages))).toEqual(["second"]);
  });

  /**
   * A withheld stamp still names the run (`inFlightRunId`), so a reply drawn
   * once with a stamp and once without is still one reply.
   */
  it("treats a withheld stamp as the same run", () => {
    const messages = [
      assistant("snapshot"),
      assistant("reloaded", { inFlightRunId: "r1" }),
      assistant("live", { runId: "r1" }),
    ];
    // The newest bubble of the run owns the reply — the snapshot and the
    // reloaded copy both describe it.
    expect(ids(collapseReplyDuplicates(messages))).toEqual(["live"]);
  });

  /** A prompt between two replies is a new turn: nothing crosses it. */
  it("never collapses across a user message", () => {
    const messages = [
      assistant("a1", { runId: "r1" }),
      user("u2", "r2"),
      assistant("a2", { runId: "r2" }),
    ];
    expect(ids(collapseReplyDuplicates(messages))).toEqual(["a1", "u2", "a2"]);
  });

  /** An older exchange keeps its unbound reply when a run is bound further down. */
  it("keeps an older unbound reply that a prompt separates", () => {
    const messages = [user("u1"), assistant("legacy"), user("u2", "r2"), assistant("a2", { runId: "r2" })];
    expect(ids(collapseReplyDuplicates(messages))).toEqual(["u1", "legacy", "u2", "a2"]);
  });

  it("returns the same array when nothing moved", () => {
    const messages = [user("u1", "r1"), assistant("a1", { runId: "r1" })];
    expect(collapseReplyDuplicates(messages)).toBe(messages);
  });
});
