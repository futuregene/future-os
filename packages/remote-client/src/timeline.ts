// Generic client projection mapping over @future-os/thread-projection.
// Applications consume this data without importing their UI into the role.
export {
  appendUserMessage,
  applyStreamEvent,
  applyStreamEvents,
  createStreamEventBatch,
  applyReplayEvents,
  commitAcknowledgedUserMessage,
  dropSupersededCompactionDividers,
  foldLiveCompactionPlaceholdersIntoHistory,
  emptyTimeline,
  markApprovalDecision,
  mergeHistoryAttachments,
  normalizeReplayEvents,
  stripRunItems,
  timelineFromEntries,
  timelineFromHistory,
  timelineFromProjection,
  upsertTruncationNotice,
} from "./projection";
export type { ReplayEventWire } from "./projection";
export type { TimelineState } from "./projection";
