import type { AgentMessage, MessageSegment } from "@future-os/thread-projection";

/**
 * The reply a host is writing, drawn once — folded into the transcript's copy of
 * it when the transcript already has one.
 *
 * A remote conversation is read from two lanes: the host's *record*, a history
 * page projected into `messages`, and its *stream*, the events it pushes, folded
 * into one reply bubble. They are two views of the same run, and on a run that
 * was already going they overlap: the agent persists each completed model call
 * mid-run, so opening a conversation whose newest exchange is still streaming
 * puts the same reply in both — the page's partial copy, and the bubble growing
 * under it. The reader saw the answer twice, the first one frozen and without the
 * streaming mark.
 *
 * Neither copy can simply be dropped. The page's copy is the reply's *beginning*:
 * the stream starts when this window subscribes, so everything the run wrote
 * before that is only in the record. The bubble is the reply's *end*: it is
 * written as the host writes it, which is why the reader opened the conversation.
 * Drawn as one message they are the reply — the record's segments, then the
 * stream's, in the order they were produced.
 *
 * The merged row keeps the record's identity (its entry id, its timestamp and the
 * user entry a fork branches at) and takes the stream's liveness, so the row the
 * reader is watching is the row that is still being written.
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
