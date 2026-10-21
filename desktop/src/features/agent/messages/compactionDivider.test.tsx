// @vitest-environment jsdom
// The compaction divider has two producers: the live stream (a run's
// `compaction_committed`, or the standalone `compaction_started/committed`
// events) and the durable history (a `compaction` entry's checkpoint). They
// must draw the *same* line with the same numbers — a user who sees
// "62,203 → 33,281" while the compaction happens cannot be told a different
// story after reopening the conversation.
//
// The indexed history used to break that: the display index stored the
// checkpoint as `""`, so the reloaded divider fell back to the number-less
// label. These tests pin the reading order (segments → label) from both
// producers, and pin that an old row without a checkpoint still draws a divider
// rather than nothing.

import type { MessageSegment, RunEvent, SessionEntry } from "@future-os/thread-projection";
import { buildAssistantRunProjection, entriesToMessages } from "@future-os/thread-projection";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { CompactionDivider } from "./MessageStatus";
import "../../../test/i18nTestSetup";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

type CompactionSegment = Extract<MessageSegment, { kind: "compaction" }>;

let counter = 0;

function asCompaction(
  segment: MessageSegment | undefined,
  message = "no compaction segment",
): CompactionSegment {
  if (segment?.kind !== "compaction") {
    throw new Error(message);
  }
  return segment;
}

function event(eventType: string, payload?: unknown): RunEvent {
  counter += 1;
  return {
    createdAt: counter,
    eventType,
    id: `e${counter}`,
    payload: payload === undefined ? null : JSON.stringify(payload),
    runId: "r1",
    sequence: counter,
  };
}

/** The live divider, from the run's own journal (`compaction_committed`). */
function liveDivider(payload: Record<string, unknown>): CompactionSegment {
  const projection = buildAssistantRunProjection([event("compaction_committed", payload)]);
  return asCompaction(projection.segments.find(item => item.kind === "compaction"));
}

/** The durable divider, from the history page the agent serves. */
function durableDivider(entry: SessionEntry): CompactionSegment {
  const [message] = entriesToMessages([entry]);
  return asCompaction(
    message?.segments?.find(item => item.kind === "compaction"),
    "the projection dropped the compaction marker",
  );
}

function label(segment: CompactionSegment): string {
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() => {
    root.render(
      createElement(CompactionDivider, {
        tokensBefore: segment.tokensBefore,
        tokensAfter: segment.tokensAfter,
        trigger: segment.trigger,
      }),
    );
  });
  const text = container.textContent ?? "";
  act(() => root.unmount());
  return text;
}

const checkpoint = {
  schemaVersion: 3,
  checkpointId: "cp_20261019-170529",
  tokensBefore: 62_203,
  tokensAfter: 33_281,
  trigger: "manual",
  phase: "standalone",
};

describe("the compaction divider reads the same live and after a reload", () => {
  it("shows the same counts, character for character", () => {
    const live = liveDivider({
      operation_id: "op_20261019-170529",
      checkpoint_id: checkpoint.checkpointId,
      tokens_before: checkpoint.tokensBefore,
      tokens_after: checkpoint.tokensAfter,
      trigger: checkpoint.trigger,
    });
    const durable = durableDivider({
      id: "cp-entry",
      kind: "compaction",
      role: "system",
      createdAtMs: Date.parse("2026-10-19T17:05:29+08:00"),
      blocks: [],
      checkpoint,
    });

    // The three fields the label is built from must agree before the text can.
    expect({
      tokensBefore: live.tokensBefore,
      tokensAfter: live.tokensAfter,
      trigger: live.trigger,
    }).toEqual({
      tokensBefore: durable.tokensBefore,
      tokensAfter: durable.tokensAfter,
      trigger: durable.trigger,
    });
    const text = label(live);
    expect(text).toBe(label(durable));
    expect(text).toContain("62,203");
    expect(text).toContain("33,281");
    expect(text).not.toBe("Context compacted");
  });

  it("keeps an automatic checkpoint's own wording on both sides", () => {
    const live = liveDivider({
      checkpoint_id: "cp_auto",
      tokens_before: 190_000,
      tokens_after: 20_000,
      trigger: "automatic",
    });
    const durable = durableDivider({
      id: "auto-entry",
      kind: "compaction",
      role: "system",
      createdAtMs: 0,
      blocks: [],
      checkpoint: {
        schemaVersion: 3,
        checkpointId: "cp_auto",
        tokensBefore: 190_000,
        tokensAfter: 20_000,
        trigger: "automatic",
        phase: "standalone",
      },
    });
    expect(label(durable)).toBe(label(live));
    expect(label(durable)).toBe("Context compacted · 190,000 → 20,000 tokens (estimated)");
  });

  it("still draws a divider for a row whose checkpoint was blanked", () => {
    // Exactly what the version-1 display index stored: `checkpoint: ""`. A
    // client must render the marker without counts — losing the marker would
    // claim the conversation continued unbroken.
    const durable = durableDivider({
      id: "legacy-entry",
      kind: "compaction",
      role: "system",
      createdAtMs: 0,
      blocks: [],
      checkpoint: "" as unknown as SessionEntry["checkpoint"],
    });
    expect(durable.tokensBefore).toBeUndefined();
    expect(durable.tokensAfter).toBeUndefined();
    expect(label(durable)).toBe("Context compacted");
  });
});
