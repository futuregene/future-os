import type { AgentMessage } from "@future-os/thread-projection";

/**
 * A compaction divider is projected as an assistant message but is not a real
 * exchange — it carries no content and a single `compaction` segment. It must not
 * consume a run slot when aligning runs to exchanges.
 */
export function isCompactionDivider(message: AgentMessage): boolean {
  return message.role === "assistant"
    && !message.content
    && message.segments?.length === 1
    && message.segments[0]?.kind === "compaction";
}

/**
 * A divider row the live bubble already renders. Until the exchange's first
 * reply entry is persisted, a checkpoint that opened the turn projects as a
 * message that IS the divider; the run's streamed reply carries the same
 * checkpoint, so both would draw it — the durable row is the copy to drop.
 * Returns false for every other divider (an earlier exchange's checkpoint, a
 * standalone manual one), which the bubble does not supersede.
 */
export function isSupersededCompactionDivider(
  message: AgentMessage,
  liveCheckpoints: ReadonlySet<string>,
): boolean {
  if (!isCompactionDivider(message))
    return false;
  return (message.segments ?? []).some(segment =>
    segment.kind === "compaction"
    && !!segment.checkpointId
    && liveCheckpoints.has(segment.checkpointId),
  );
}
