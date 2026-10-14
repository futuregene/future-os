import type { MessageSegment } from "@future-os/thread-projection";
import type { NavRung, TranscriptMessage } from "./questionNav";
import { describe, expect, it } from "vitest";
import { buildNavRungs, nextRung, previousQuestion, sameRung } from "./questionNav";

function user(id: string): TranscriptMessage {
  return { id, role: "user", content: `prompt ${id}` };
}

function assistant(id: string, extra: Partial<TranscriptMessage> = {}): TranscriptMessage {
  return { id, role: "assistant", content: `answer ${id}`, ...extra };
}

const text = (id: string): MessageSegment => ({ id, kind: "text", text: "…" }) as MessageSegment;
const reasoning = (id: string): MessageSegment => ({ id, kind: "thinking", text: "…" }) as MessageSegment;
function call(id: string): MessageSegment {
  return ({ id, kind: "activity", item: { id, kind: "read", status: "completed" } }) as unknown as MessageSegment;
}
const divider = (id: string): MessageSegment => ({ id, kind: "compaction", status: "completed" }) as MessageSegment;

/**
 * The fixture transcript, in the order it is drawn (oldest first):
 *
 *   0 u1     the first prompt
 *   1 a1     …its reply: reasoning, a call, then prose
 *   2 u2     the second prompt
 *   3 a2     …a reply that folded entirely into thinking: no rung of its own
 *   4 u3     the third prompt
 *   5 a3     …its reply, compacted down to a divider: the rung stands on it
 *   6 u4     the fourth prompt, still unanswered
 */
const MESSAGES: TranscriptMessage[] = [
  user("u1"),
  assistant("a1", { segments: [reasoning("h1"), call("w1"), text("t1")] }),
  user("u2"),
  assistant("a2", { segments: [reasoning("h2")] }),
  user("u3"),
  assistant("a3", { segments: [divider("c1")] }),
  user("u4"),
];

describe("buildNavRungs", () => {
  it("is empty for an empty transcript", () => {
    expect(buildNavRungs([])).toEqual([]);
  });

  it("gives each turn its prompt then the end of its reply", () => {
    expect(buildNavRungs(MESSAGES)).toEqual([
      { kind: "question", index: 0, messageId: "u1" },
      { kind: "replyTail", index: 1, messageId: "a1" },
      { kind: "question", index: 2, messageId: "u2" },
      { kind: "question", index: 4, messageId: "u3" },
      { kind: "replyTail", index: 5, messageId: "a3" },
      { kind: "question", index: 6, messageId: "u4" },
    ]);
  });

  it("skips a reply that folded entirely into reasoning", () => {
    // A pause is not an answer: the press above it should reach the prompt that
    // started the turn, not stop on thinking.
    const rungs = buildNavRungs(MESSAGES);
    expect(rungs.filter(rung => rung.kind === "replyTail").map(rung => rung.messageId))
      .toEqual(["a1", "a3"]);
  });

  it("keeps a rung for a reply the compaction replaced", () => {
    // a3's prose is gone, but the divider marks where the answer was.
    expect(buildNavRungs(MESSAGES)).toContainEqual({ kind: "replyTail", index: 5, messageId: "a3" });
  });

  it("counts a legacy reply by its plain content", () => {
    const rungs = buildNavRungs([user("u1"), assistant("a1")]);
    expect(rungs).toEqual([
      { kind: "question", index: 0, messageId: "u1" },
      { kind: "replyTail", index: 1, messageId: "a1" },
    ]);
  });

  it("leaves an unanswered prompt with only its question", () => {
    expect(buildNavRungs([user("u1")])).toEqual([{ kind: "question", index: 0, messageId: "u1" }]);
  });

  it("takes the first reply of a turn that has output", () => {
    // The store can split a turn across assistant rows; the ladder stops on the
    // one the reader is heading for, which is the reply that answered.
    const rungs = buildNavRungs([
      user("u1"),
      assistant("a1", { segments: [reasoning("h1")] }),
      assistant("a2", { segments: [text("t1")] }),
      assistant("a3", { segments: [text("t2")] }),
    ]);
    expect(rungs).toEqual([
      { kind: "question", index: 0, messageId: "u1" },
      { kind: "replyTail", index: 2, messageId: "a2" },
    ]);
  });
});

const LADDER = buildNavRungs(MESSAGES);
const rungAt = (position: number): NavRung => LADDER[position]!;

describe("the ↑/↓ walk over rungs", () => {
  it("offers nothing without a reader position", () => {
    expect(previousQuestion(LADDER, null)).toBeNull();
    expect(nextRung(LADDER, null)).toBeNull();
  });

  it("↑ offers the reader's own prompts and never a reply", () => {
    // From inside the second turn's reply, the press reaches the prompt that
    // produced it — not the first prompt, and not the reply it is already in.
    expect(previousQuestion(LADDER, 3)).toEqual(rungAt(2));
    // From the second prompt itself, the first one.
    expect(previousQuestion(LADDER, 2)).toEqual(rungAt(0));
    // From inside the first turn's reply, its own prompt.
    expect(previousQuestion(LADDER, 1)).toEqual(rungAt(0));
    // …and from the third prompt, whose reply is a pause with no rung, the
    // second prompt — the answer in between is skipped either way.
    expect(previousQuestion(LADDER, 4)).toEqual(rungAt(2));
  });

  it("the row at the top edge is the reader's, not a destination", () => {
    // A prompt landed at the top edge (the press before this one put it there)
    // is not re-selected.
    expect(previousQuestion(LADDER, 0)).toBeNull();
    expect(previousQuestion(LADDER, 4)).toEqual(rungAt(2));
  });

  it("↑ runs out at the oldest row", () => {
    expect(previousQuestion(LADDER, 0)).toBeNull();
    expect(previousQuestion(LADDER, 1)).toEqual(rungAt(0));
  });

  it("↓ takes the very next stop below, of either kind", () => {
    // The transcript as written: prompt, reply, prompt, reply…
    expect(nextRung(LADDER, 0)).toEqual(rungAt(1)); // u1 → a1's end
    expect(nextRung(LADDER, 1)).toEqual(rungAt(2)); // a1 → u2
    expect(nextRung(LADDER, 2)).toEqual(rungAt(3)); // u2 → u3 (a2 is a pause: no rung)
    expect(nextRung(LADDER, 3)).toEqual(rungAt(3)); // inside a2 → u3
    expect(nextRung(LADDER, 4)).toEqual(rungAt(4)); // u3 → a3's end
    expect(nextRung(LADDER, 5)).toEqual(rungAt(5)); // a3 → u4
  });

  it("↓ never selects the row at the top edge", () => {
    // The last rung is the newest row: nothing below it.
    expect(nextRung(LADDER, 6)).toBeNull();
    expect(nextRung(LADDER, 7)).toBeNull();
  });

  it("↑ then ↓ returns to the reply the reader came from", () => {
    // From inside the third reply, ↑ reaches its prompt, and ↓ comes back to
    // that same reply — the pair is one ladder read both ways.
    const up = previousQuestion(LADDER, 5);
    expect(up).toEqual(rungAt(3));
    expect(nextRung(LADDER, up!.index)).toEqual(rungAt(4));
  });
});

describe("sameRung", () => {
  it("compares what a rung is, not where it sat", () => {
    expect(sameRung(rungAt(0), { kind: "question", index: 0, messageId: "u1" })).toBe(true);
    expect(sameRung(rungAt(1), { kind: "question", index: 1, messageId: "a1" })).toBe(false);
    expect(sameRung(null, null)).toBe(true);
    expect(sameRung(rungAt(0), null)).toBe(false);
  });
});
