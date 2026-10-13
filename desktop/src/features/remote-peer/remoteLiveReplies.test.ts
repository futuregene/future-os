import type { AgentMessage, MessageSegment } from "@future-os/thread-projection";
import { describe, expect, it } from "vitest";
import { foldLiveReply, stripRunCopies } from "./remoteLiveReplies";

/**
 * The reply a host is writing, drawn once.
 *
 * A conversation opened while its newest exchange is still running carries that
 * reply twice: the host's history page has the part the agent has already written
 * to its journal, and the events it is pushing carry the rest. This is the fold
 * that draws them as the one reply they are — what a reader on the other machine
 * sees instead of an answer beside itself.
 */

function message(overrides: Partial<AgentMessage> = {}): AgentMessage {
  return {
    id: "m_a1",
    role: "assistant",
    authorKey: "author.researchCopilot",
    content: "half of it",
    createdAt: new Date(1_700_000_000_000).toISOString(),
    ...overrides,
  };
}

function text(value: string, id = "seg_text"): MessageSegment {
  return { id, kind: "text", text: value };
}

function call(id: string): MessageSegment {
  return {
    id: `seg_${id}`,
    kind: "activity",
    item: { id, kind: "shell", status: "completed", toolCallId: id, runId: "run_1" },
  };
}

function compaction(checkpointId: string, id = "seg_ck"): MessageSegment {
  return { id, kind: "compaction", checkpointId, status: "completed" };
}

describe("remoteLiveReplies", () => {
  it("looks like the transcript when nothing is streaming", () => {
    const transcript = [message()];
    expect(foldLiveReply(transcript, null)).toBe(transcript);
  });

  /**
   * A reply the transcript has never seen is drawn after it, unchanged: this is
   * the prompt sent from this window, whose answer is not in any page read before
   * it.
   */
  it("appends a reply the transcript does not carry", () => {
    const transcript = [message({ id: "m_a1", runId: "run_0" })];
    const live = message({ id: "stream_run_1", runId: "run_1", content: "being written", status: "streaming" });
    expect(foldLiveReply(transcript, live)).toEqual([transcript[0], live]);
  });

  /** The reported shape: one row for the reply, not two. */
  it("folds the reply being written into the transcript's copy of it", () => {
    const transcript = [
      message({ id: "m_u1", role: "user", content: "keep going" }),
      message({
        id: "m_a1",
        runId: "run_1",
        sourceEntryId: "e_a1",
        segments: [text("half of it")],
      }),
    ];
    const live = message({
      id: "stream_run_1",
      runId: "run_1",
      content: "and the rest",
      status: "streaming",
      runStartedAt: 1_700_000_000_123,
      segments: [text("and the rest", "seg_live")],
    });

    const folded = foldLiveReply(transcript, live);

    expect(folded).toHaveLength(2);
    // The identity of the record: this is the row that was already there, and
    // the one a fork addresses.
    expect(folded[1]!.id).toBe("m_a1");
    expect(folded[1]!.sourceEntryId).toBe("e_a1");
    // The record's segments come first: it holds what the run wrote before this
    // window subscribed, and the bubble holds what it wrote after.
    expect(folded[1]!.segments?.map(segment => (segment.kind === "text" ? segment.text : "")))
      .toEqual(["half of it", "and the rest"]);
    expect(folded[1]!.content).toBe("half of it\n\nand the rest");
    // The row is the one still being written, so it carries the streaming mark
    // and the clock the reader watches.
    expect(folded[1]!.status).toBe("streaming");
    expect(folded[1]!.runStartedAt).toBe(1_700_000_000_123);
  });

  it("keeps a call the record already shows from being drawn twice", () => {
    const transcript = [message({ runId: "run_1", segments: [call("call_1"), text("done")] })];
    const live = message({
      id: "stream_run_1",
      runId: "run_1",
      segments: [call("call_1"), call("call_2"), text("more", "seg_live")],
    });

    const [folded] = foldLiveReply(transcript, live);

    const calls = (folded!.segments ?? []).filter(segment => segment.kind === "activity");
    expect(calls.map(segment => (segment.kind === "activity" ? segment.item.toolCallId : "")))
      .toEqual(["call_1", "call_2"]);
  });

  it("keeps a checkpoint the record already carries from being drawn twice", () => {
    const transcript = [
      message({ runId: "run_1", segments: [compaction("ck_1", "seg_head"), text("after the summary")] }),
    ];
    const live = message({
      id: "stream_run_1",
      runId: "run_1",
      segments: [compaction("ck_1", "seg_live_ck"), text("more", "seg_live")],
    });

    const [folded] = foldLiveReply(transcript, live);

    const checkpoints = (folded!.segments ?? []).filter(segment => segment.kind === "compaction");
    expect(checkpoints).toHaveLength(1);
  });

  /**
   * Only the run that is streaming is folded: an older reply keeps its place, and
   * its text is not merged into another run's row.
   */
  it("leaves an earlier run's reply alone", () => {
    const transcript = [
      message({ id: "m_a1", runId: "run_0", content: "answer one" }),
      message({ id: "m_u2", role: "user", content: "second" }),
    ];
    const live = message({ id: "stream_run_1", runId: "run_1", content: "answer two", status: "streaming" });

    const folded = foldLiveReply(transcript, live);

    expect(folded.map(item => item.content)).toEqual(["answer one", "second", "answer two"]);
  });

  /** A bubble with no run of its own is not evidence that it is this transcript's. */
  it("never folds a reply that names no run", () => {
    const transcript = [message({ id: "m_a1", segments: [text("half of it")] })];
    const live = message({ id: "stream_none", content: "being written", status: "streaming" });
    expect(foldLiveReply(transcript, live)).toEqual([transcript[0], live]);
  });
});

/**
 * The other half of "one row per reply": the runs this client rebuilt from their
 * logs. Their copy in the record is hidden, and *only* theirs — a compaction
 * divider or a notice carries a run id too, and hiding those would drop content
 * the reply does not replace.
 */
describe("stripRunCopies", () => {
  it("returns the transcript untouched when nothing was rebuilt", () => {
    const transcript = [message({ id: "m_a1", runId: "run_1" })];
    expect(stripRunCopies(transcript, new Set())).toEqual(transcript);
  });

  it("hides a rebuilt run's reply and keeps the prompt it answers", () => {
    const transcript = [
      message({ id: "m_u1", role: "user", content: "keep going", runId: "run_1" }),
      message({ id: "m_a1", runId: "run_1" }),
      message({ id: "m_u2", role: "user", content: "and now", runId: "run_2" }),
      message({ id: "m_a2", runId: "run_2" }),
    ];

    const stripped = stripRunCopies(transcript, new Set(["run_1"]));

    expect(stripped.map(item => item.id)).toEqual(["m_u1", "m_u2", "m_a2"]);
  });

  /**
   * A compaction divider is not a reply, so it is not hidden.
   *
   * The projection draws a checkpoint as a row of its own with no run of its own
   * (`dividerMessage`), which is what keeps it out of this: the replayed reply
   * renders the same checkpoint from the run's journal, and hiding the record's
   * divider would leave the summary with nothing to sit under.
   */
  it("keeps rows that are not replies", () => {
    const transcript = [
      message({
        id: "m_ck",
        content: "",
        segments: [{ id: "seg_ck", kind: "compaction", checkpointId: "ck_1", status: "completed" }],
      }),
      message({ id: "m_a1", runId: "run_1" }),
      // A reply the record itself does not attribute to a run is not this
      // client's to hide either.
      message({ id: "m_a0", content: "unattributed" }),
    ];

    expect(stripRunCopies(transcript, new Set(["run_1"])).map(item => item.id)).toEqual(["m_ck", "m_a0"]);
  });
});
