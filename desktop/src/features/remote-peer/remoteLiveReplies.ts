import type { AgentMessage, MessageSegment } from "@future-os/thread-projection";

/**
 * One reply, drawn once — the two rules a remote transcript needs, and when each
 * applies.
 *
 * A remote conversation is read from two lanes: the host's *record* (a page of
 * its journal, projected into `messages`) and its *stream* (the events it pushes,
 * folded into a reply bubble). They are two views of the same runs, and which one
 * is authoritative depends on whether this client also read the run's **log**
 * (`get_events_since`):
 *
 * - **Read the log** (`stripRunCopies`): the projection covers the run from its
 *   beginning, so the transcript's own partial copy of that run is redundant —
 *   and keeping it would draw the answer twice, because the page's copy and the
 *   pushed frames overlap for a run the reader joined mid-flight. The copy is
 *   hidden and the rebuilt reply is drawn in its place.
 * - **Only saw pushes** (`foldLiveReply`): the page holds the reply's *beginning*
 *   (the stream starts when this window subscribes) and the bubble holds its
 *   end*, so neither can be dropped: drawn as one message they are the reply.
 *
 * The second rule is the fallback, not the design: it holds only while a host
 * does not serve the read at all (an older paired desktop), or while the first
 * read is still in flight. Text has no identity across the two lanes, so the fold
 * relies on the boundary between them rather than on recognising the same words
 * twice — which is exactly why the log read, where it exists, is preferred.
 */

/**
 * Hide the transcript's copy of the runs this client rebuilt from their logs.
 *
 * Only a run's *reply* rows are hidden. The user's own prompt stays where it is:
 * it is the turn the reply answers, it is what a fork branches at, and the
 * rebuilt reply is drawn directly beneath it — the same place the record had it.
 * A compaction divider or a notice is not a reply either, and a run id can appear
 * on them.
 */
export function stripRunCopies(
  transcript: readonly AgentMessage[],
  runs: ReadonlySet<string>,
): AgentMessage[] {
  if (runs.size === 0)
    return [...transcript];
  return transcript.filter(message => !(message.role === "assistant" && message.runId && runs.has(message.runId)));
}

/**
 * The reply a host is writing, drawn once — folded into the transcript's copy of
 * it when the transcript already has one.
 *
 * Used for a run this client has *not* read the log of: what the pushes built is
 * the reply's tail, and the page's partial copy is its beginning. The merged row
 * keeps the record's identity (its entry id, its timestamp and the user entry a
 * fork branches at) and takes the stream's liveness, so the row on screen is the
 * one still being written.
 */
export function foldLiveReply(
  transcript: AgentMessage[],
  live: AgentMessage | null,
): AgentMessage[] {
  if (!live)
    return transcript;
  const index = live.runId
    ? transcript.findIndex(message => message.role === "assistant" && message.runId === live.runId)
    : -1;
  // A run the stream has covered in full — a prompt sent from this window, whose
  // reply is not in any page read before it — has no copy to fold into, and is
  // drawn after the transcript as before.
  if (index < 0)
    return [...transcript, live];
  return transcript.map((message, position) => (position === index ? foldReply(message, live) : message));
}

function foldReply(head: AgentMessage, live: AgentMessage): AgentMessage {
  const segments = [...(head.segments ?? []), ...unseenSegments(head, live)];
  return {
    ...head,
    content: [head.content, live.content].filter(text => text !== "").join("\n\n"),
    ...(segments.length > 0 ? { segments } : {}),
    // The row is being written: it is the one the view marks as in flight, and
    // the one that offers nothing to act on.
    status: "streaming",
    ...(live.thinkingActive ? { thinkingActive: true } : {}),
    ...(live.reconnecting ? { reconnecting: live.reconnecting } : {}),
    ...(live.runStartedAt != null ? { runStartedAt: live.runStartedAt } : {}),
    ...(live.outputTokens != null ? { outputTokens: live.outputTokens } : {}),
  };
}

/**
 * The stream's segments the record does not already carry.
 *
 * What the two lanes hold is disjoint by construction rather than by
 * coincidence: a tail read restarts the stream (`useRemoteTimeline`), so the
 * record ends where this window subscribed and the bubble begins there. Identity
 * keeps that a rule instead of a bet on two machines' clocks — a call the record
 * already shows is not drawn a second time, and neither is a compaction the
 * record already draws as a divider of its own.
 */
function unseenSegments(head: AgentMessage, live: AgentMessage): MessageSegment[] {
  const seen = new Set((head.segments ?? []).flatMap((segment) => {
    const key = segmentIdentity(segment);
    return key === null ? [] : [key];
  }));
  return (live.segments ?? []).filter((segment) => {
    const key = segmentIdentity(segment);
    return key === null || !seen.has(key);
  });
}

/**
 * A segment's identity across the two lanes, or null for one that has none.
 *
 * A tool call is identified by its call id (the record carries it in its
 * arguments, the stream on the call's start) and a compaction by its checkpoint.
 * Text and reasoning have no key the two lanes share — the record names them
 * after its entries and the stream after its events — so what keeps them from
 * repeating is the boundary between the lanes, not a key.
 */
function segmentIdentity(segment: MessageSegment): string | null {
  if (segment.kind === "activity")
    return `call:${segment.item.toolCallId ?? segment.item.id}`;
  if (segment.kind === "compaction" && segment.checkpointId)
    return `checkpoint:${segment.checkpointId}`;
  return null;
}
