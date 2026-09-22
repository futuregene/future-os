/**
 * Compaction-divider identity, shared by the durable projection and the live
 * run projector.
 *
 * One compaction reaches a client twice while its run is still live: as the
 * durable history row for the checkpoint entry, and as a slice of that run's
 * streamed reply. The two copies are rendered from unrelated id spaces — the
 * history row's message/segment ids derive from the checkpoint, the live
 * projector pins its own run-scoped slot — so the reconciliation each client
 * does on ids cannot fold them, and the same checkpoint is drawn twice: once as
 * a divider on its own, once inside the reply that contains it.
 *
 * `checkpointId` is the identity both copies carry. Clients use this to drop
 * the durable row the live render already supersedes.
 */

/** Anything that renders inline segments: the desktop `AgentMessage` and the
 * mobile `TimelineItem` are both structurally compatible. */
export interface CompactionDividerCarrier {
  segments?: readonly { kind: string; checkpointId?: string }[];
}

/**
 * The checkpoints these messages already render as compaction dividers.
 * Empty when none of them is a committed compaction marker.
 */
export function compactionCheckpoints(
  carriers: readonly CompactionDividerCarrier[],
): Set<string> {
  const checkpoints = new Set<string>();
  for (const carrier of carriers) {
    for (const segment of carrier.segments ?? []) {
      if (segment.kind === "compaction" && segment.checkpointId)
        checkpoints.add(segment.checkpointId);
    }
  }
  return checkpoints;
}
