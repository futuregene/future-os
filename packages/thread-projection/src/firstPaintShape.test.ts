import { describe, expect, it } from "vitest";
import type { MessageBlock, SessionEntry } from "./events";
import { entriesToTurns } from "./projection";

/**
 * The two shapes a first page can arrive in, projected.
 *
 * The projection is deliberately strict about this (`turnFromAcc` returns null
 * without a question): a reply whose prompt is not on the page is not a turn, so
 * a page of them projects to nothing and the conversation draws blank. These are
 * the pages `packages/rpc/src/history.rs` served before 2026-11-17 and serves
 * now, in the shape the host sends them — 400 rows in the newest turn against a
 * 300-row first-paint bound, which is the reported blank conversation.
 */

const BASE = 1_700_000_000_000;

function text(text: string): MessageBlock {
  return { kind: "text", text };
}

function entry(
  id: string,
  role: SessionEntry["role"],
  blocks: MessageBlock[] = [],
): SessionEntry {
  return {
    blocks,
    createdAtMs: BASE,
    id,
    kind: role,
    role,
  };
}

/** A 300-row window of a 400-row turn: what the old row cut kept. */
function turnTail(rows: number): SessionEntry[] {
  return Array.from({ length: rows }, (_, index) =>
    entry(`e1-a${index}`, "assistant", [text(`step ${index}`)]));
}

/** The same turn as the fixed reader serves it: from its question. */
function wholeTurn(rows: number): SessionEntry[] {
  return [entry("q1", "user", [text("why is this page empty?")]), ...turnTail(rows)];
}

describe("a first page that opens mid-turn", () => {
  it("projects to nothing when its question is off the page", () => {
    // The defect, pinned: 300 assistant rows and no question is a page the
    // projection has no turn to build from.
    expect(entriesToTurns(turnTail(300))).toHaveLength(0);
  });

  it("projects to the turn when the page carries its question", () => {
    const nodes = entriesToTurns(wholeTurn(400));
    expect(nodes).toHaveLength(1);
    const node = nodes[0]!;
    if (node.kind !== "turn")
      throw new Error("expected a turn node");
    expect(node.turn.user.content).toBe("why is this page empty?");
  });
});
